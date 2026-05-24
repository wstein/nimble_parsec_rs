import NimbleParsec

defmodule DifferentialFixtures do
  import NimbleParsec

  date =
    integer(4)
    |> ignore(string("-"))
    |> integer(2)
    |> ignore(string("-"))
    |> integer(2)

  time =
    integer(2)
    |> ignore(string(":"))
    |> integer(2)
    |> ignore(string(":"))
    |> integer(2)

  defparsec :datetime, date |> ignore(string("T")) |> concat(time)

  defparsec :lookahead_digit,
            ascii_char([])
            |> lookahead(integer(min: 1))

  defparsec :repeat_while_digits,
            repeat_while(
              ascii_char([?0..?9]) |> ascii_char([?0..?9]),
              {:not_3, []}
            )

  defp not_3(<<?3, _::binary>>, context, _, _), do: {:halt, context}
  defp not_3(_, context, _, _), do: {:cont, context}

  defparsec :tagged_int, integer(min: 1) |> tag(:n)

  defparsec :wrapped_pair,
            wrap(integer(2) |> ignore(string(",")) |> integer(2))

  defparsec :ascii_lower, ascii_string([?a..?z], min: 1)

  defparsec :dup_ab, duplicate(string("ab"), 3)

  def run do
    cases = [
      {:datetime, "2010-04-17T14:12:34", &datetime/1},
      {:lookahead_digit, "a0", &lookahead_digit/1},
      {:repeat_while_digits, "12345", &repeat_while_digits/1},
      {:tagged_int, "42", &tagged_int/1},
      {:wrapped_pair, "12,34", &wrapped_pair/1},
      {:ascii_lower, "abc123", &ascii_lower/1},
      {:dup_ab, "ababab", &dup_ab/1}
    ]

    Enum.each(cases, fn {name, input, fun} ->
      case fun.(input) do
        {:ok, tokens, rest, _context, _line, offset} ->
          IO.puts("ok|#{name}|#{rest}|#{offset}|#{length(tokens)}|#{format_tokens(tokens)}")

        {:error, reason, rest, _context, _line, offset} ->
          IO.puts("err|#{name}|#{rest}|#{offset}|#{reason}")
      end
    end)
  end

  # Serializes a token list into a stable, language-neutral string so the Rust
  # differential runner can compare the parsed values, not just their count.
  defp format_tokens(tokens), do: Enum.map_join(tokens, ",", &format_token/1)
  defp format_token(int) when is_integer(int), do: Integer.to_string(int)
  defp format_token(bin) when is_binary(bin), do: "s:" <> bin
  defp format_token(list) when is_list(list), do: "l:(#{format_tokens(list)})"
  defp format_token({name, inner}) when is_list(inner), do: "t:#{name}:(#{format_tokens(inner)})"
  defp format_token({name, value}), do: "kv:#{name}:#{format_token(value)}"
end

DifferentialFixtures.run()
