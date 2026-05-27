# SPDX-License-Identifier: Apache-2.0

defmodule Stem.Parser do
  @moduledoc false

  # Lexes and parses Stem source into a `Stem.AST`.
  #
  # This module combines lexing and structural parsing into a single stage.
  # NimbleParsec combinators recognise the `{{ }}` family of constructs and
  # attach line/column metadata via `post_traverse` hooks, eliminating manual
  # byte iteration for position tracking.  Elixir recursive-descent functions
  # then fold the flat token stream into the nested AST that `Stem.Compiler`
  # consumes.
  #
  # Partial expansion happens inline during the structural parse: each
  # `{{> name}}` token is replaced by recursively parsing the partial's source
  # with a recursion guard in the call stack.

  import NimbleParsec
  alias Stem.Expression

  # ---------------------------------------------------------------------------
  # Types (kept public so callers can reference them without Stem.Tokenizer)
  # ---------------------------------------------------------------------------

  @type meta :: %{
          required(:line) => pos_integer(),
          required(:column) => pos_integer(),
          optional(:end_line) => pos_integer(),
          optional(:end_column) => pos_integer()
        }
  @type kind :: :if | :unless | :each | :with | :region

  @type token ::
          {:text, binary(), meta()}
          | {:expr, binary(), atom(), meta()}
          | {:block_open, kind(), binary(), meta()}
          | {:block_else, meta()}
          | {:block_close, kind(), meta()}
          | {:yield, binary(), meta()}
          | {:partial, binary(), binary(), meta()}
          | {:eof, meta()}

  @block_kinds %{
    "if" => :if,
    "unless" => :unless,
    "each" => :each,
    "with" => :with,
    "region" => :region
  }

  @kind_tags %{if: "if", unless: "unless", each: "each", with: "with", region: "region"}

  # ---------------------------------------------------------------------------
  # NimbleParsec – position-capture callback
  # ---------------------------------------------------------------------------

  # Called by `post_traverse` hooks on each combinator.  Prepends
  # `{:end_pos, line, col}` to the accumulated args so that
  # `assemble_tokens/5` can read the end position of each raw token directly
  # from the tagged tuple without byte iteration.
  @doc false
  def inject_end_pos(rest, args, context, {line, line_offset}, byte_offset) do
    col = byte_offset - line_offset + 1
    # args is in reversed stack order; appending here means {:end_pos, ...}
    # appears FIRST after tag/1 reverses the accumulation.
    {rest, args ++ [{:end_pos, line, col}], context}
  end

  # Validates that the open and close names of a raw block match, then reduces
  # the content chars to a string. Returns the same shape as `inject_end_pos`
  # so `assemble_tokens` for `:raw_block` sees `[{:end_pos, …}, content]`.
  @doc false
  def validate_and_reduce_raw_block(rest, args, context, {line, line_offset}, byte_offset) do
    col = byte_offset - line_offset + 1
    # NimbleParsec prepends results (newest first), so args order is:
    # [close_name, charN, …, char1, open_name]
    [close_name | rest_args] = args
    open_name = List.last(rest_args)
    chars = Enum.drop(rest_args, -1)
    content = chars |> Enum.reverse() |> List.to_string()

    if open_name == close_name do
      {rest, [content, {:end_pos, line, col}], context}
    else
      {:error,
       "raw block '{{{{##{open_name}}}}}' closed by '{{{{/#{close_name}}}}}' — names must match",
       context}
    end
  end

  # ---------------------------------------------------------------------------
  # NimbleParsec combinators (compile-time)
  # ---------------------------------------------------------------------------

  # Matches any sequence of characters that does not start a `{{` tag.
  text_chunk =
    lookahead_not(string("{{"))
    |> utf8_char([])
    |> repeat(
      lookahead_not(string("{{"))
      |> utf8_char([])
    )
    |> reduce({List, :to_string, []})
    |> post_traverse({__MODULE__, :inject_end_pos, []})
    |> tag(:text_chunk)

  # Matches `{{!-- ... --}}` block comments.
  block_comment =
    ignore(string("{{!--"))
    |> repeat(
      lookahead_not(string("--}}"))
      |> utf8_char([])
    )
    |> ignore(string("--}}"))
    |> post_traverse({__MODULE__, :inject_end_pos, []})
    |> tag(:block_comment)

  # Matches `{{! ... }}` inline comments.
  inline_comment =
    ignore(string("{{!"))
    |> repeat(
      lookahead_not(string("}}"))
      |> utf8_char([])
    )
    |> ignore(string("}}"))
    |> post_traverse({__MODULE__, :inject_end_pos, []})
    |> tag(:inline_comment)

  # Matches `{{{{#name}}}}...{{{{/name}}}}` — 4-brace raw block; content emitted verbatim.
  # The post_traverse callback validates open/close names match and reduces content.
  raw_block =
    ignore(string("{{{{#"))
    |> ascii_string([?a..?z, ?A..?Z, ?0..?9, ?_, ?-], min: 1)
    |> ignore(string("}}}}"))
    |> repeat(
      lookahead_not(string("{{{{/"))
      |> utf8_char([])
    )
    |> ignore(string("{{{{/"))
    |> ascii_string([?a..?z, ?A..?Z, ?0..?9, ?_, ?-], min: 1)
    |> ignore(string("}}}}"))
    |> post_traverse({__MODULE__, :validate_and_reduce_raw_block, []})
    |> tag(:raw_block)

  # Matches `{{{ ... }}}` raw (no-escape) output.
  raw_tag =
    ignore(string("{{{"))
    |> repeat(
      lookahead_not(string("}}}"))
      |> utf8_char([])
    )
    |> ignore(string("}}}"))
    |> reduce({List, :to_string, []})
    |> post_traverse({__MODULE__, :inject_end_pos, []})
    |> tag(:raw_tag)

  # Matches `{{ ... }}` standard tags.
  standard_tag =
    ignore(string("{{"))
    |> repeat(
      lookahead_not(string("}}"))
      |> utf8_char([])
    )
    |> ignore(string("}}"))
    |> reduce({List, :to_string, []})
    |> post_traverse({__MODULE__, :inject_end_pos, []})
    |> tag(:standard_tag)

  defparsec(
    :do_lex,
    repeat(
      choice([
        block_comment,
        inline_comment,
        raw_block,
        raw_tag,
        standard_tag,
        text_chunk
      ])
    )
  )

  # ---------------------------------------------------------------------------
  # Public API
  # ---------------------------------------------------------------------------

  @spec parse(binary(), keyword()) :: {:ok, Stem.AST.t()} | {:error, binary(), meta()}
  def parse(source, opts \\ []) when is_binary(source) do
    case parse_with_spans(source, opts) do
      {:ok, ast} -> {:ok, strip_ast_spans(ast)}
      {:error, message, meta} -> {:error, message, strip_meta_spans(meta)}
    end
  end

  @doc false
  @spec parse_with_spans(binary(), keyword()) :: {:ok, Stem.AST.t()} | {:error, binary(), meta()}
  def parse_with_spans(source, opts \\ []) when is_binary(source) do
    partials = opts |> Keyword.get(:partials, %{}) |> normalize_partials()

    with {:ok, tokens} <- tokenize_with_spans(source, opts) do
      parse_stream(tokens, partials, [])
    end
  end

  @doc """
  Like `parse/2`, but accumulates **every** recoverable parse error instead of
  stopping at the first.

  Invalid expressions, reserved operators, unknown/recursive partials, and bad
  block or partial arguments are each recorded (substituting a placeholder node)
  so a single pass reports them all in source order; a structural error that
  desynchronises the token stream still ends collection and is reported last.
  Returns `{:ok, ast}` or `{:error, [%{file, message, meta}]}`, where `file` is
  `"main"` or the partial the error occurred in. Mirrors the native compiler's
  `compile_to_wire_all`.
  """
  @spec parse_all(binary(), keyword()) ::
          {:ok, Stem.AST.t()} | {:error, [%{file: binary(), message: binary(), meta: meta()}]}
  def parse_all(source, opts \\ []) when is_binary(source) do
    partials = opts |> Keyword.get(:partials, %{}) |> normalize_partials()

    case tokenize_with_spans(source, opts) do
      {:ok, tokens} ->
        case parse_stream_all(tokens, partials, []) do
          {:ok, nodes} -> {:ok, strip_ast_spans(nodes)}
          {:error, _errors} = error -> error
        end

      {:error, message, meta} ->
        {:error, [%{file: "main", message: message, meta: strip_meta_spans(meta)}]}
    end
  end

  @doc """
  Parses one template's source to its pre-expansion AST.

  Unlike `parse/2`, `{{> name}}` tags are kept as `:partial` nodes rather than
  inlined, so a file's own structure and its partial dependency edges stay
  visible. Takes no partials map: each file is parsed on its own. Serialize the
  result with `Stem.AST.to_wire/1` for the `stem-ast/v1` wire the playground
  consumes.
  """
  @spec parse_ast(binary(), keyword()) :: {:ok, Stem.AST.t()} | {:error, binary(), meta()}
  def parse_ast(source, opts \\ []) when is_binary(source) do
    case parse_ast_with_spans(source, opts) do
      {:ok, ast} -> {:ok, strip_ast_spans(ast)}
      {:error, message, meta} -> {:error, message, strip_meta_spans(meta)}
    end
  end

  @doc false
  @spec parse_ast_with_spans(binary(), keyword()) ::
          {:ok, Stem.AST.t()} | {:error, binary(), meta()}
  def parse_ast_with_spans(source, opts \\ []) when is_binary(source) do
    with {:ok, tokens} <- tokenize_with_spans(source, opts) do
      parse_stream(tokens, :no_expand, [])
    end
  end

  @doc """
  Parses a template plus its partials into a runtime-invocation *program*
  (ADR-0014).

  The main AST keeps `{:partial}` nodes (no inlining), and the returned
  `partials` registry maps every *transitively* referenced partial name to its
  parsed AST — each parsed once, so cycles are legal (the recursion guard is
  gone). An unknown partial is an error. Feed the result to
  `Stem.Bytecode.compile(nodes, partials: registry)`, which lowers each
  `{:partial}` to a runtime `invoke_partial` op against the registry.
  """
  @spec parse_program(binary(), map() | keyword()) ::
          {:ok, %{nodes: Stem.AST.t(), partials: %{binary() => Stem.AST.t()}}}
          | {:error, binary(), meta()}
  def parse_program(source, partials \\ %{}) when is_binary(source) do
    sources = normalize_partials(partials)

    with {:ok, main} <- parse_ast_with_spans(source),
         {:ok, registry} <- build_registry(referenced_partials(main), sources, %{}) do
      {:ok, %{nodes: main, partials: registry}}
    end
  end

  # Parse each transitively-referenced partial once (memoised by name in `acc`),
  # so a cycle terminates instead of looping. Unknown names are an error.
  defp build_registry([], _sources, acc), do: {:ok, acc}

  defp build_registry([{name, meta} | rest], sources, acc) do
    cond do
      Map.has_key?(acc, name) ->
        build_registry(rest, sources, acc)

      true ->
        case Map.fetch(sources, name) do
          {:ok, src} ->
            case parse_ast_with_spans(src) do
              {:ok, ast} ->
                build_registry(referenced_partials(ast) ++ rest, sources, Map.put(acc, name, ast))

              {:error, message, m} ->
                {:error, message, m}
            end

          :error ->
            {:error, "unknown partial '#{name}'", meta}
        end
    end
  end

  # The `{name, meta}` of every partial referenced directly in `nodes` (recursing
  # into block bodies). Used to walk the dependency graph for the registry.
  defp referenced_partials(nodes), do: Enum.flat_map(nodes, &node_partials/1)

  defp node_partials({:partial, name, _ctx, _hash, meta}), do: [{name, meta}]
  defp node_partials({:region, _name, body, _meta}), do: referenced_partials(body)
  defp node_partials({:if, _e, t, els, _m}), do: referenced_partials(t) ++ referenced_partials(els)
  defp node_partials({:unless, _e, t, els, _m}), do: referenced_partials(t) ++ referenced_partials(els)

  defp node_partials({:each, _e, _p, body, els, _m}),
    do: referenced_partials(body) ++ referenced_partials(els)

  defp node_partials({:with, _e, _p, body, els, _m}),
    do: referenced_partials(body) ++ referenced_partials(els)

  defp node_partials({:partial_scope, _c, _h, body, _m}), do: referenced_partials(body)
  defp node_partials(_other), do: []

  # Exposed for lexer-level tests (position tracking, trim markers, error
  # messages).  Not part of the stable public API.
  @doc false
  @spec tokenize(binary(), keyword()) :: {:ok, [token()]} | {:error, binary(), meta()}
  def tokenize(source, opts \\ []) when is_binary(source) do
    case tokenize_with_spans(source, opts) do
      {:ok, tokens} -> {:ok, Enum.map(tokens, &strip_token_spans/1)}
      {:error, message, meta} -> {:error, message, strip_meta_spans(meta)}
    end
  end

  defp tokenize_with_spans(source, opts) when is_binary(source) do
    line = Keyword.get(opts, :line, 1)
    column = Keyword.get(opts, :column, 1)

    case check_raw_blocks(source) do
      {:error, _, _} = err ->
        err

      :ok ->
        case do_lex(source) do
          {:ok, raw_tokens, "", _ctx, _pos, _offset} ->
            assemble_tokens(raw_tokens, line, column, [], false)

          {:ok, _raw, rest, _ctx, {err_line, err_line_offset}, err_byte} ->
            err_col = err_byte - err_line_offset + 1

            {:error, unterminated_error(rest),
             %{line: err_line, column: err_col, end_line: err_line, end_column: err_col}}
        end
    end
  end

  defp check_raw_blocks(source) do
    check_raw_block_open(source, 0)
  end

  defp check_raw_block_open(source, offset) do
    remaining = byte_size(source) - offset

    case :binary.match(source, "{{{{#", scope: {offset, remaining}) do
      :nomatch ->
        :ok

      {open_prefix_pos, open_prefix_len} ->
        name_start = open_prefix_pos + open_prefix_len

        case :binary.match(source, "}}}}", scope: {name_start, byte_size(source) - name_start}) do
          :nomatch ->
            before = binary_part(source, 0, open_prefix_pos)
            {err_line, err_col} = advance_through(before, 1, 1)
            meta = %{line: err_line, column: err_col, end_line: err_line, end_column: err_col}
            {:error, "unterminated raw block open tag", meta}

          {close_brace_pos, close_brace_len} ->
            name = binary_part(source, name_start, close_brace_pos - name_start)
            open_tag_end = close_brace_pos + close_brace_len
            close_tag = "{{{{/" <> name <> "}}}}"

            case :binary.match(source, close_tag,
                   scope: {open_tag_end, byte_size(source) - open_tag_end}
                 ) do
              :nomatch ->
                before = binary_part(source, 0, open_prefix_pos)
                {err_line, err_col} = advance_through(before, 1, 1)
                meta = %{line: err_line, column: err_col, end_line: err_line, end_column: err_col}
                {:error, "expected closing '{{{{/#{name}}}}}' for raw block", meta}

              {close_tag_pos, close_tag_len} ->
                check_raw_block_open(source, close_tag_pos + close_tag_len)
            end
        end
    end
  end

  # ---------------------------------------------------------------------------
  # Token assembly
  # ---------------------------------------------------------------------------

  # Folds the flat list emitted by `do_lex/1` into the `[token()]` list
  # consumed by the structural parser.  Each raw token carries
  # `{:end_pos, end_line, end_col}` as its first element (injected by
  # `inject_end_pos/5`).  `{line, col}` is the START position of the current
  # token (equal to the end position of the previous token).

  defp assemble_tokens([], line, col, acc, _trim_next) do
    {:ok, Enum.reverse([{:eof, span_meta(line, col, line, col)} | acc])}
  end

  defp assemble_tokens(
         [{:text_chunk, [{:end_pos, end_line, end_col}, text]} | rest],
         line,
         col,
         acc,
         trim_next
       ) do
    {text_line, text_col, trimmed} =
      if trim_next do
        remaining = String.replace(text, ~r/\A[\s]+/u, "")
        stripped = binary_part(text, 0, byte_size(text) - byte_size(remaining))
        {new_line, new_col} = advance_through(stripped, line, col)
        {new_line, new_col, remaining}
      else
        {line, col, text}
      end

    acc2 =
      case {trimmed, acc} do
        {"", _} ->
          acc

        {_, [{:text, prev_text, prev_meta} | rest_acc]} ->
          [{:text, prev_text <> trimmed, prev_meta} | rest_acc]

        {_, _} ->
          [{:text, trimmed, span_meta(text_line, text_col, end_line, end_col)} | acc]
      end

    assemble_tokens(rest, end_line, end_col, acc2, false)
  end

  defp assemble_tokens(
         [{:block_comment, [{:end_pos, end_line, end_col} | _chars]} | rest],
         _line,
         _col,
         acc,
         trim_next
       ) do
    assemble_tokens(rest, end_line, end_col, acc, trim_next)
  end

  defp assemble_tokens(
         [{:inline_comment, [{:end_pos, end_line, end_col} | _chars]} | rest],
         _line,
         _col,
         acc,
         trim_next
       ) do
    assemble_tokens(rest, end_line, end_col, acc, trim_next)
  end

  defp assemble_tokens(
         [{:raw_block, [{:end_pos, end_line, end_col}, content]} | rest],
         line,
         col,
         acc,
         _trim_next
       ) do
    acc2 =
      case {content, acc} do
        {"", _} ->
          acc

        {_, [{:text, prev_text, prev_meta} | rest_acc]} ->
          [{:text, prev_text <> content, prev_meta} | rest_acc]

        {_, _} ->
          [{:text, content, span_meta(line, col, end_line, end_col)} | acc]
      end

    assemble_tokens(rest, end_line, end_col, acc2, false)
  end

  defp assemble_tokens(
         [{:raw_tag, [{:end_pos, end_line, end_col}, inner]} | rest],
         line,
         col,
         acc,
         _trim_next
       ) do
    meta = span_meta(line, col, end_line, end_col)
    {inner2, trim_left, trim_right} = extract_trim_markers(inner)
    acc2 = maybe_trim_last_text(acc, trim_left)

    case classify_raw_expr(inner2, meta) do
      {:ok, token} -> assemble_tokens(rest, end_line, end_col, [token | acc2], trim_right)
      :skip -> assemble_tokens(rest, end_line, end_col, acc2, trim_right)
      {:error, message, emeta} -> {:error, message, emeta}
    end
  end

  defp assemble_tokens(
         [{:standard_tag, [{:end_pos, end_line, end_col}, inner]} | rest],
         line,
         col,
         acc,
         _trim_next
       ) do
    case escaped_mustache(acc, inner) do
      {:escaped, new_acc} ->
        assemble_tokens(rest, end_line, end_col, new_acc, false)

      {:normal, new_acc} ->
        meta = span_meta(line, col, end_line, end_col)
        {inner2, trim_left, trim_right} = extract_trim_markers(inner)
        acc2 = maybe_trim_last_text(new_acc, trim_left)

        case classify(inner2, meta) do
          {:ok, token} -> assemble_tokens(rest, end_line, end_col, [token | acc2], trim_right)
          :skip -> assemble_tokens(rest, end_line, end_col, acc2, trim_right)
          {:error, message, emeta} -> {:error, message, emeta}
        end
    end
  end

  defp maybe_trim_last_text(acc, false), do: acc

  defp maybe_trim_last_text([{:text, text, meta} | rest], true) do
    trimmed = String.replace(text, ~r/[\s]+\z/u, "")
    if trimmed == "", do: rest, else: [{:text, trimmed, meta} | rest]
  end

  defp maybe_trim_last_text(acc, true), do: acc

  # Counts trailing backslashes before `{{`. Consumes exactly one: N=1 escapes
  # the tag (emits literal `{{inner}}`); N≥2 emits N-1 backslashes and evaluates.
  defp escaped_mustache([{:text, text, meta} | rest_acc], inner) do
    n = count_trailing_backslashes(text, 0)

    if n == 0 do
      {:normal, [{:text, text, meta} | rest_acc]}
    else
      base = binary_part(text, 0, byte_size(text) - 1)

      if n == 1 do
        {:escaped, [{:text, base <> "{{" <> inner <> "}}", meta} | rest_acc]}
      else
        new_acc = if base == "", do: rest_acc, else: [{:text, base, meta} | rest_acc]
        {:normal, new_acc}
      end
    end
  end

  defp escaped_mustache(acc, _inner), do: {:normal, acc}

  defp count_trailing_backslashes(<<>>, n), do: n

  defp count_trailing_backslashes(text, n) do
    if :binary.last(text) == ?\\ do
      count_trailing_backslashes(binary_part(text, 0, byte_size(text) - 1), n + 1)
    else
      n
    end
  end

  # ---------------------------------------------------------------------------
  # Tag classification
  # ---------------------------------------------------------------------------

  defp classify(inner, meta) do
    tag = String.trim(inner)

    cond do
      String.contains?(tag, "{") or String.contains?(tag, "}") ->
        {:error, "nested braces are not supported in Stem expressions", meta}

      tag == "" ->
        :skip

      tag == "else" ->
        {:ok, {:block_else, meta}}

      first_word(tag) == "yield" ->
        classify_yield(tag, meta)

      String.starts_with?(tag, "#") ->
        classify_open(tag, meta)

      String.starts_with?(tag, "/") ->
        classify_close(tag, meta)

      String.starts_with?(tag, ">") ->
        {name, args} =
          tag
          |> binary_part(1, byte_size(tag) - 1)
          |> split_first_word()

        {:ok, {:partial, name, args, meta}}

      true ->
        {:ok, {:expr, tag, :default, meta}}
    end
  end

  defp classify_raw_expr(inner, meta) do
    tag = String.trim(inner)

    cond do
      String.contains?(tag, "{") or String.contains?(tag, "}") ->
        {:error, "nested braces are not supported in Stem expressions", meta}

      tag == "" ->
        :skip

      true ->
        {:ok, {:expr, tag, :none, meta}}
    end
  end

  defp classify_open(tag, meta) do
    {name, args} =
      tag
      |> binary_part(1, byte_size(tag) - 1)
      |> split_first_word()

    case Map.fetch(@block_kinds, name) do
      {:ok, kind} -> {:ok, {:block_open, kind, args, meta}}
      :error -> {:error, "unsupported Stem block helper '{{##{name}}}'", meta}
    end
  end

  defp classify_yield(tag, meta) do
    {_yield, args} = split_first_word(tag)
    {:ok, {:yield, args, meta}}
  end

  defp classify_close(tag, meta) do
    name = binary_part(tag, 1, byte_size(tag) - 1) |> String.trim()

    case Map.fetch(@block_kinds, name) do
      {:ok, kind} -> {:ok, {:block_close, kind, meta}}
      :error -> {:error, "unsupported Stem closing tag '{{#{tag}}}'", meta}
    end
  end

  defp split_first_word(string) do
    case String.split(String.trim_leading(string), ~r/\s+/, parts: 2) do
      [word] -> {word, ""}
      [word, rest] -> {word, String.trim(rest)}
    end
  end

  defp first_word(string) do
    string
    |> split_first_word()
    |> elem(0)
  end

  defp extract_trim_markers(inner) do
    trimmed = String.trim(inner)
    trim_left = String.starts_with?(trimmed, "~")
    trim_right = String.ends_with?(trimmed, "~")

    normalized =
      trimmed
      |> maybe_trim_leading_marker(trim_left)
      |> maybe_trim_trailing_marker(trim_right)
      |> String.trim()

    {normalized, trim_left, trim_right}
  end

  defp maybe_trim_leading_marker(inner, true), do: String.trim_leading(inner, "~")
  defp maybe_trim_leading_marker(inner, false), do: inner

  defp maybe_trim_trailing_marker(inner, true), do: String.trim_trailing(inner, "~")
  defp maybe_trim_trailing_marker(inner, false), do: inner

  defp span_meta(line, column, end_line, end_column) do
    %{line: line, column: column, end_line: end_line, end_column: end_column}
  end

  # ---------------------------------------------------------------------------
  # Error messages (lexer level)
  # ---------------------------------------------------------------------------

  defp unterminated_error(<<"{{!--", _::binary>>),
    do: "expected closing '--}}' for Stem comment"

  defp unterminated_error(<<"{{!", _::binary>>),
    do: "expected closing '}}' for Stem comment"

  defp unterminated_error(<<"{{{{", _::binary>>),
    do: "expected closing '{{{{/name}}}}' for raw block"

  defp unterminated_error(<<"{{{", _::binary>>),
    do: "expected closing '}}}' for raw Stem expression"

  defp unterminated_error(<<"{{", _::binary>>),
    do: "expected closing '}}' for Stem expression"

  # ---------------------------------------------------------------------------
  # Position tracking (trim-next prefix only)
  # ---------------------------------------------------------------------------

  # Advances {line, col} through `binary`, computing the position of the first
  # character that follows it.  Used only when a right-trim marker strips
  # leading whitespace from the next text chunk and the chunk's start position
  # must be updated accordingly.
  #
  # Unlike the former `advance_binary/3`, this function is non-recursive: it
  # splits on newline boundaries once and computes the result with arithmetic.
  defp advance_through("", line, col), do: {line, col}

  defp advance_through(binary, line, col) do
    case :binary.split(binary, ["\r\n", "\n"], [:global]) do
      [only] ->
        {line, col + String.length(only)}

      parts ->
        {line + length(parts) - 1, String.length(List.last(parts)) + 1}
    end
  end

  # ---------------------------------------------------------------------------
  # Structural parser helpers
  # ---------------------------------------------------------------------------

  # Tokenises and parses a nested source (used for partial expansion).
  defp strip_ast_spans(nodes), do: Enum.map(nodes, &strip_node_spans/1)

  defp strip_node_spans({:text, text}), do: {:text, text}
  defp strip_node_spans({:yield, name, meta}), do: {:yield, name, strip_meta_spans(meta)}

  defp strip_node_spans({:expr, expr, escape_mode, meta}) do
    {:expr, expr, escape_mode, strip_meta_spans(meta)}
  end

  defp strip_node_spans({:if, expr, body, else_body, meta}) do
    {:if, expr, strip_ast_spans(body), strip_ast_spans(else_body), strip_meta_spans(meta)}
  end

  defp strip_node_spans({:unless, expr, body, else_body, meta}) do
    {:unless, expr, strip_ast_spans(body), strip_ast_spans(else_body), strip_meta_spans(meta)}
  end

  defp strip_node_spans({:each, expr, params, body, else_body, meta}) do
    {:each, expr, params, strip_ast_spans(body), strip_ast_spans(else_body),
     strip_meta_spans(meta)}
  end

  defp strip_node_spans({:with, expr, params, body, else_body, meta}) do
    {:with, expr, params, strip_ast_spans(body), strip_ast_spans(else_body),
     strip_meta_spans(meta)}
  end

  defp strip_node_spans({:region, name, body, meta}) do
    {:region, name, strip_ast_spans(body), strip_meta_spans(meta)}
  end

  defp strip_node_spans({:partial_scope, context, hash, body, meta}) do
    {:partial_scope, context, hash, strip_ast_spans(body), strip_meta_spans(meta)}
  end

  defp strip_node_spans({:partial, name, context, hash, meta}) do
    {:partial, name, context, hash, strip_meta_spans(meta)}
  end

  defp strip_meta_spans(%{line: line, column: column}), do: %{line: line, column: column}

  defp strip_token_spans({:text, text, meta}), do: {:text, text, strip_meta_spans(meta)}

  defp strip_token_spans({:expr, raw, escape_mode, meta}),
    do: {:expr, raw, escape_mode, strip_meta_spans(meta)}

  defp strip_token_spans({:block_open, kind, args, meta}),
    do: {:block_open, kind, args, strip_meta_spans(meta)}

  defp strip_token_spans({:block_else, meta}), do: {:block_else, strip_meta_spans(meta)}

  defp strip_token_spans({:block_close, kind, meta}),
    do: {:block_close, kind, strip_meta_spans(meta)}

  defp strip_token_spans({:yield, name, meta}), do: {:yield, name, strip_meta_spans(meta)}

  defp strip_token_spans({:partial, name, args, meta}),
    do: {:partial, name, args, strip_meta_spans(meta)}

  defp strip_token_spans({:eof, meta}), do: {:eof, strip_meta_spans(meta)}

  # Fail-fast driver (parse/2, parse_ast/2): collection always accumulates, but
  # this surfaces only the first error in source order — identical to stopping
  # at it. Meta keeps its spans (parse_with_spans relies on them; parse/2 strips).
  defp parse_stream(tokens, partials, stack) do
    case collect(tokens, partials, stack, [], []) do
      {:cont, nodes, {:eof, _meta}, [], errors} ->
        case Enum.reverse(errors) do
          [] -> {:ok, nodes}
          [first | _] -> {:error, first.message, first.meta}
        end

      {:cont, _nodes, {:else, meta}, _rest, errors} ->
        first_error(errors, "unexpected '{{else}}' outside of a block", meta, stack)

      {:cont, _nodes, {:close, kind, meta}, _rest, errors} ->
        first_error(errors, "unexpected closing tag '{{/#{@kind_tags[kind]}}}'", meta, stack)

      {:halt, message, meta, errors} ->
        first_error(errors, message, meta, stack)
    end
  end

  # The earliest error in source order: a recoverable one if any preceded the
  # structural stop, else the structural error itself.
  defp first_error(errors, message, meta, stack) do
    [first | _] = Enum.reverse(add_error(errors, message, meta, stack))
    {:error, first.message, first.meta}
  end

  # Accumulating driver (parse_all/2): the same collection, but every recoverable
  # error is reported (a trailing structural error appended last), in source order.
  defp parse_stream_all(tokens, partials, stack) do
    case collect(tokens, partials, stack, [], []) do
      {:cont, nodes, {:eof, _meta}, [], errors} ->
        case all_errors(errors) do
          [] -> {:ok, nodes}
          list -> {:error, list}
        end

      {:cont, _nodes, {:else, meta}, _rest, errors} ->
        {:error, all_errors(add_error(errors, "unexpected '{{else}}' outside of a block", meta, stack))}

      {:cont, _nodes, {:close, kind, meta}, _rest, errors} ->
        {:error,
         all_errors(
           add_error(errors, "unexpected closing tag '{{/#{@kind_tags[kind]}}}'", meta, stack)
         )}

      {:halt, message, meta, errors} ->
        {:error, all_errors(add_error(errors, message, meta, stack))}
    end
  end

  # Errors accumulate newest-first; reverse to source order and strip meta spans.
  defp all_errors(errors) do
    errors |> Enum.reverse() |> Enum.map(&%{&1 | meta: strip_meta_spans(&1.meta)})
  end

  # Record a recoverable error, stamped with the file it occurred in (the partial
  # on top of the recursion stack, or "main" at the top level).
  defp add_error(errors, message, meta, stack) do
    [%{file: current_file(stack), message: message, meta: meta} | errors]
  end

  defp current_file([]), do: "main"
  defp current_file([name | _]), do: name

  # Collects nodes until a stop token (`{{else}}`, a block close, or eof),
  # threading the recoverable-error accumulator. Returns
  # `{:cont, nodes, stop, rest, errors}` for a normal stop, or
  # `{:halt, message, meta, errors}` when a structural error ends collection.
  # Recoverable errors are recorded into `errors` (a placeholder node keeps the
  # tree well-formed) and collection continues.
  defp collect([{:text, text, _meta} | rest], partials, stack, acc, errors) do
    collect(rest, partials, stack, [{:text, text} | acc], errors)
  end

  defp collect([{:expr, raw, escape_mode, meta} | rest], partials, stack, acc, errors) do
    case Expression.parse(raw) do
      {:ok, expr} ->
        collect(rest, partials, stack, [{:expr, expr, escape_mode, meta} | acc], errors)

      {:error, message} ->
        node = {:expr, {:literal, "null"}, escape_mode, meta}
        collect(rest, partials, stack, [node | acc], add_error(errors, message, meta, stack))
    end
  end

  defp collect([{:yield, raw_name, meta} | rest], partials, stack, acc, errors) do
    node = {:yield, raw_name, meta}

    case validate_region_name(raw_name) do
      :ok -> collect(rest, partials, stack, [node | acc], errors)
      {:error, message} -> collect(rest, partials, stack, [node | acc], add_error(errors, message, meta, stack))
    end
  end

  # In `:no_expand` mode (the `parse_ast/2` path) a partial stays an unexpanded
  # `:partial` node carrying its parsed context/hash, so a file's own structure
  # and its dependency edges remain visible. Otherwise it expands inline.
  defp collect([{:partial, name, args, meta} | rest], :no_expand, stack, acc, errors) do
    case Expression.parse_partial_args(args) do
      {:ok, context, hash} ->
        node = {:partial, String.trim(name), context, hash, meta}
        collect(rest, :no_expand, stack, [node | acc], errors)

      {:error, message} ->
        collect(rest, :no_expand, stack, acc, add_error(errors, message, meta, stack))
    end
  end

  defp collect([{:partial, name, args, meta} | rest], partials, stack, acc, errors) do
    # Unknown/recursive partials and faults inside an expanded partial are
    # recoverable for the caller: record them and keep assembling the rest.
    {nodes, errors} = expand_partial(name, args, meta, partials, stack, errors)
    collect(rest, partials, stack, Enum.reverse(nodes, acc), errors)
  end

  defp collect([{:block_open, kind, args, meta} | rest], partials, stack, acc, errors) do
    case parse_block(kind, args, meta, rest, partials, stack, errors) do
      {:ok, node, rest, errors} -> collect(rest, partials, stack, [node | acc], errors)
      {:halt, _message, _meta, _errors} = halted -> halted
    end
  end

  defp collect([{:block_else, meta} | rest], _partials, _stack, acc, errors) do
    {:cont, Enum.reverse(acc), {:else, meta}, rest, errors}
  end

  defp collect([{:block_close, kind, meta} | rest], _partials, _stack, acc, errors) do
    {:cont, Enum.reverse(acc), {:close, kind, meta}, rest, errors}
  end

  defp collect([{:eof, meta}], _partials, _stack, acc, errors) do
    {:cont, Enum.reverse(acc), {:eof, meta}, [], errors}
  end

  defp parse_block(kind, args, meta, tokens, partials, stack, errors) do
    # A bad block head (invalid subject or block params) is recoverable: record
    # it and keep collecting the body so its tags are still checked and the
    # matching `{{/..}}` close stays in sync.
    {expr, params, errors} =
      case parse_block_expression(kind, args) do
        {:ok, expr, params} -> {expr, params, errors}
        {:error, message} -> {placeholder_head(kind), [], add_error(errors, message, meta, stack)}
      end

    case collect(tokens, partials, stack, [], errors) do
      {:cont, _body, {:else, else_meta}, _rest, errors} when kind == :region ->
        {:halt, "unexpected '{{else}}' inside '{{#region}}'", else_meta, errors}

      {:cont, body, {:else, _else_meta}, rest, errors} ->
        parse_else(kind, expr, params, body, meta, rest, partials, stack, errors)

      {:cont, body, {:close, ^kind, _close_meta}, rest, errors} ->
        {:ok, block_node(kind, expr, params, body, [], meta), rest, errors}

      {:cont, _body, {:close, other, close_meta}, _rest, errors} ->
        {:halt, mismatched_close(kind, other), close_meta, errors}

      {:cont, _body, {:eof, _eof_meta}, _rest, errors} ->
        {:halt, unclosed_block(kind), meta, errors}

      {:halt, _message, _meta, _errors} = halted ->
        halted
    end
  end

  # Placeholder block head when the real one failed to parse (the block is
  # discarded along with the program once an error is recorded).
  defp placeholder_head(:region), do: ""
  defp placeholder_head(_kind), do: {:literal, "null"}

  defp parse_else(kind, expr, params, body, meta, tokens, partials, stack, errors) do
    case collect(tokens, partials, stack, [], errors) do
      {:cont, else_body, {:close, ^kind, _close_meta}, rest, errors} ->
        {:ok, block_node(kind, expr, params, body, else_body, meta), rest, errors}

      {:cont, _else_body, {:else, else_meta}, _rest, errors} ->
        {:halt, "unexpected second '{{else}}' inside '{{##{@kind_tags[kind]}}}'", else_meta, errors}

      {:cont, _else_body, {:close, other, close_meta}, _rest, errors} ->
        {:halt, mismatched_close(kind, other), close_meta, errors}

      {:cont, _else_body, {:eof, _eof_meta}, _rest, errors} ->
        {:halt, unclosed_block(kind), meta, errors}

      {:halt, _message, _meta, _errors} = halted ->
        halted
    end
  end

  defp parse_block_expression(kind, args) when kind in [:if, :unless] do
    case Expression.parse(args) do
      {:ok, expr} -> {:ok, expr, []}
      {:error, _message} = error -> error
    end
  end

  defp parse_block_expression(kind, args) when kind in [:each, :with] do
    case split_block_params(args) do
      {:ok, expr_source, params} ->
        with :ok <- validate_block_params(kind, params),
             {:ok, expr} <- Expression.parse(expr_source) do
          {:ok, expr, params}
        end
    end
  end

  defp parse_block_expression(:region, args) do
    name = String.trim(args)

    with :ok <- validate_region_name(name) do
      {:ok, name, []}
    end
  end

  defp split_block_params(args) do
    case Regex.run(~r/^(.*?)(?:\s+as\s+\|([^|]+)\|)?\s*$/s, args, capture: :all_but_first) do
      [expr_source] ->
        {:ok, String.trim(expr_source), []}

      [expr_source, params_source] ->
        params = params_source |> String.split(~r/\s+/, trim: true)
        {:ok, String.trim(expr_source), params}
    end
  end

  defp validate_block_params(:with, []), do: :ok
  defp validate_block_params(:with, [_param]), do: :ok

  defp validate_block_params(:with, _params),
    do: {:error, "{{#with}} accepts at most one block parameter"}

  defp validate_block_params(:each, params) when length(params) <= 3,
    do: validate_identifier_list(params)

  defp validate_block_params(:each, _params),
    do: {:error, "{{#each}} accepts at most three block parameters"}

  defp validate_identifier_list(params) do
    # `_` is the anonymous/wildcard param: it may appear repeatedly to skip a
    # positional slot, so it is exempt from the uniqueness check.
    named = Enum.reject(params, &(&1 == "_"))

    cond do
      Enum.any?(params, &(not String.match?(&1, ~r/^[A-Za-z_][A-Za-z0-9_]*$/))) ->
        {:error, "block parameters must be simple identifiers"}

      length(named) != length(Enum.uniq(named)) ->
        {:error, "block parameters must be unique"}

      true ->
        :ok
    end
  end

  defp block_node(:each, expr, params, body, else_body, meta),
    do: {:each, expr, params, body, else_body, meta}

  defp block_node(:with, expr, params, body, else_body, meta),
    do: {:with, expr, params, body, else_body, meta}

  defp block_node(:region, name, _params, body, _else_body, meta),
    do: {:region, name, body, meta}

  defp block_node(kind, expr, _params, body, else_body, meta),
    do: {kind, expr, body, else_body, meta}

  defp mismatched_close(open_kind, close_kind) do
    "unexpected closing tag '{{/#{@kind_tags[close_kind]}}}'; " <>
      "expected '{{/#{@kind_tags[open_kind]}}}'"
  end

  defp unclosed_block(kind) do
    "expected a closing '{{/#{@kind_tags[kind]}}}' for block expression in Stem"
  end

  # Expand `{{> name args}}` inline, threading the error accumulator. Every
  # failure mode — empty/unknown/recursive name, bad args, or a fault inside the
  # partial's body — is recoverable for the caller: it is recorded and no nodes
  # are spliced, so the surrounding collection continues. Returns
  # `{nodes, errors}` (nodes empty on failure).
  defp expand_partial("", _args, meta, _partials, stack, errors) do
    {[], add_error(errors, "partial name is required in '{{> ...}}'", meta, stack)}
  end

  defp expand_partial(name, args, meta, partials, stack, errors) do
    cond do
      name in stack ->
        {[], add_error(errors, "partial recursion detected for '#{name}'", meta, stack)}

      true ->
        case Map.fetch(partials, name) do
          {:ok, content} ->
            expand_partial_body(name, args, content, meta, partials, stack, errors)

          :error ->
            {[], add_error(errors, "unknown partial '#{name}'", meta, stack)}
        end
    end
  end

  defp expand_partial_body(name, args, content, meta, partials, stack, errors) do
    inner_stack = [name | stack]

    with {:ok, context, hash} <- Expression.parse_partial_args(args),
         {:ok, tokens} <- tokenize_with_spans(content, []) do
      case collect(tokens, partials, inner_stack, [], errors) do
        {:cont, nodes, {:eof, _meta}, [], errors} ->
          {wrap_partial_scope(context, hash, nodes, meta), errors}

        {:cont, _nodes, {:else, e_meta}, _rest, errors} ->
          {[], add_error(errors, "unexpected '{{else}}' outside of a block", e_meta, inner_stack)}

        {:cont, _nodes, {:close, kind, c_meta}, _rest, errors} ->
          {[],
           add_error(errors, "unexpected closing tag '{{/#{@kind_tags[kind]}}}'", c_meta, inner_stack)}

        {:halt, message, h_meta, errors} ->
          {[], add_error(errors, message, h_meta, inner_stack)}
      end
    else
      # Bad partial arguments (parsed against the caller's tag) or an
      # unterminated tag in the partial source (attributed to the partial).
      {:error, message} -> {[], add_error(errors, message, meta, stack)}
      {:error, message, t_meta} -> {[], add_error(errors, message, t_meta, inner_stack)}
    end
  end

  # Partials without arguments expand inline, inheriting the caller's scope.
  # Arguments establish a fresh scope, so the inlined nodes are wrapped in a
  # `:partial_scope` node the compiler lowers to a rebound `assigns` binding.
  defp wrap_partial_scope(nil, [], nodes, _meta), do: nodes

  defp wrap_partial_scope(context, hash, nodes, meta) do
    [{:partial_scope, context, hash, nodes, meta}]
  end

  defp normalize_partials(partials) when is_map(partials) do
    Map.new(partials, fn {key, value} -> {to_string(key), value} end)
  end

  defp normalize_partials(partials) when is_list(partials) do
    partials |> Enum.into(%{}) |> normalize_partials()
  end

  defp validate_region_name(name) do
    cond do
      name == "" ->
        {:error, "region name is required"}

      String.match?(name, ~r/^[a-z_][a-zA-Z0-9_]*$/) ->
        :ok

      true ->
        {:error, "region names must be simple identifiers"}
    end
  end
end
