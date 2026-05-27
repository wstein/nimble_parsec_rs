# Changelog

All notable changes to the `nimble_parsec_rs` crate are documented here. This
crate follows [Semantic Versioning](https://semver.org/). While at `0.x`, the
public API (the ~31 combinators, the fluent `Parser` methods, and the `Value` /
`Cursor` / `ParseFailure` types) may change between minor versions; a typed
`Parser<T>` surface is the planned `1.0` (see the README roadmap).

## [Unreleased]

### Generic and binary input (feat/generic-binary) — BREAKING

The crate now supports `&[u8]` (binary) input alongside `&str` (text), with
`Partial<S>` for streaming (incomplete-input) parsing. All leaf parsers and the
`Stream` abstraction are part of the public API.

#### Added in feat/generic-binary

- **`Stream` trait** (`lib.rs`): describes a parser input sequence. Associated
  items: `Token` (element type — `char` for `&str`, `u8` for `&[u8]`), `Slice`
  (multi-token output type), and `PARTIAL: bool` (`false` for complete input,
  `true` for `Partial<S>`). Implemented for `&str`, `&[u8]`, and `Partial<S>`.
- **`Compare<Pat>` trait** (`lib.rs`): type-safe pattern matching used by
  `literal` and `byte`. Implemented for `&str`/`&str`, `&[u8]`/`&[u8]`, and
  `&[u8]`/`u8`.
- **`Partial<S>` wrapper type**: wraps `&str` or `&[u8]` to signal incomplete
  input. Parsers propagate `Incomplete` upwards instead of hard-failing when
  more data is needed.
- **Binary leaf parsers** (available when `S = &[u8]` or `Partial<&[u8]>`):
  `byte(b)`, `byte_range(lo, hi)`, `be_u8/16/32/64`, `le_u8/16/32/64`,
  `utf8_char()`, `take(n)`, `bytes(n)` (alias for `take(n)`), `rest()`.
- **`take(n)`** generalises the old text-only `bytes(n)`: yields `&[u8]` on
  byte-slice input and `&str` on text input. `bytes(n)` is retained as an alias.

#### Changed in feat/generic-binary (BREAKING)

- **All leaf parser constructors are now generic over `S: Stream`**: `any<S>()`,
  `satisfy<S, F>()`, `literal<S, Pat>()`, `take<S>()`, `bytes<S>()`, `eof<S>()`,
  `empty<S>()`, etc. Rust infers `S` from context in almost all cases; a type
  annotation is only required in generate-only or otherwise ambiguous contexts
  (e.g. `literal::<&str, _>("x")`).
- **`Recursive<'a, S, O>`**: the `recursive` combinator now carries an explicit
  `Stream` type parameter `S` and a lifetime `'a`. Existing `&str`-only uses
  require adding `&str` as the stream type; the lifetime is inferred.
- **Error messages updated for generic input**:
  - `"expected any character"` → `"expected any token"`
  - `"expected at least one matching character"` → `"expected at least one matching token"`
  - `"expected N base units"` → `"expected N bytes"`

---

### Typed parser redesign (RFC 0001) — BREAKING

A breaking redesign: the crate is now a **typed** parser-combinator library,
replacing the dynamic `Value`-based port.

#### Added in RFC-0001

- Typed `Parser<Output>` surface (`nimble_parsec_rs::typed`, re-exported at the
  crate root): generic over its output and composed at compile time with no
  runtime tagging. Leaves `literal`/`any`/`satisfy`/`one_of`/`none_of`/
  `take_while`/`take_while1`/`digits`/`eof`/`choice`/`lookahead`/`not`/
  `recursive`; methods `map`/`try_map`/`to`/`ignored`/`then`/`ignore_then`/
  `then_ignore`/`or`/`optional`/`repeated`/`repeated_at_least`/`repeated_in`/
  `labelled`; run via `parse`/`parse_partial` (and the `*_with_max_depth`
  variants).
- Structured parse errors: `ParseFailure { reason, expected, rest, cursor }` with
  `expecting`/`rejected` constructors; `expected` is unioned across `or`/`choice`
  and empty for non-expectation failures (negative assertions, `try_map`
  rejections, the recursion cap).
- Parity leaves/combinators carrying their NimbleParsec names: `integer()`
  (a digit run parsed to `i64`, overflow-safe), `eventually(p)` (skip input until
  `p` matches), `empty()` (always succeeds, consumes nothing), and `bytes(n)`
  (exactly `n` bytes as `&str`, requiring a UTF-8 boundary — arbitrary non-UTF-8
  bytes still await byte-slice input).
- A dedicated `nimble` module (`nimble_parsec_rs::nimble`) of NimbleParsec
  terminology as free functions, for readers porting from Elixir — `use
  nimble_parsec_rs::nimble::*` gives the vocabulary in one import. Renames
  (`string`/`eos`/`concat`/`replace`/`duplicate`) plus free-function forms of the
  method-combinators (`optional`/`repeat`/`times`/`ignore`/`map`/`label`/
  `lookahead_not`/`byte_offset`/`line`/`debug`/`post_traverse`/`pre_traverse`) and
  re-exports of the already-matching names. `tag` / `unwrap_and_tag` / `reduce` /
  `wrap` are intentionally **omitted** — in the typed API they are `.map` into a
  typed value (or `.fold`); aliasing them would re-import the untyped term-list
  model. The idiomatic core namespace is left uncluttered.
- `.flat_map(f)`: monadic bind — use a parser's output to choose the next parser,
  enabling context-sensitive grammars (dynamic length prefixes such as
  netstrings, layout-sensitive parsing). Not generatable (like `recursive`), so it
  has no `Generate` impl.
- `.fold(init, f)`: repeat a parser, folding outputs into an accumulator
  (NimbleParsec's `reduce`) without the intermediate `Vec` that
  `.repeated().map(…)` allocates. (`tag`/`wrap`/`unwrap_and_tag` are intentionally
  not resugared — in the typed API they are ordinary `.map` into a typed value.)
- Convenience combinators: `delimited(open, content, close)`,
  `separated_by` / `separated_by1` (separated lists, no trailing separator), and
  `repeated_until(p, end)` (repeat until a terminator, not consumed). `choice` now
  accepts a tuple `(a, b, …)` (arity ≤ 8) of differently-typed alternatives in
  addition to an array `[p; N]`, via the new `Alternatives` trait.
- `.post_traverse(f)` / `.pre_traverse(f)`: position-aware fallible transforms
  (the callback gets the end / start `Cursor` and may fail the parse with
  `Err(message)`). User context is threaded by capturing interior-mutable state
  (`Cell`/`RefCell`) in the callback, enabling context-dependent parsing.
- Position, debug, and generation combinators: `.with_byte_offset()` and
  `.with_line()` pair the output with the trailing position (NimbleParsec's
  `byte_offset` / `line`); `.debug(label)` traces a parser to stderr; and
  `generate(&parser, seed)` synthesizes a random accepted input for non-recursive
  grammars (via the `Generate` trait and a dependency-free PRNG).
- Recursion-depth cap (`DEFAULT_MAX_RECURSION_DEPTH`, default 256), overridable
  via `Parser::parse_with_max_depth`, returning a `ParseFailure` instead of
  overflowing the stack.
- Property-based tests (`proptest`), publication metadata, and a `1.70` MSRV.

#### Removed in RFC-0001

- **BREAKING:** the dynamic `Value`/`Ast` interpreter and its API
  (`Parser`/`ParserRef`/`choice`/`repeat`/`tag`/`reduce`/`post_traverse`/… over
  `Vec<Value>`), the `compile_parser!`/`defparsec!` codegen macros and the
  `parsec_macro` crate, the `generate` input synthesizer, and the Criterion
  benchmark. With them go the `num-bigint` and `rand` dependencies — the crate
  now has **zero runtime dependencies**.

## [0.1.0]

- Initial port: runtime combinator interpreter over a reified `Ast`, plus a
  proc-macro codegen path (`compile_parser!` / `defparsec!` family) specializing
  ~25 combinators. ~31 combinators, line/byte cursor tracking, `generate`-based
  input synthesis, and Criterion benchmarks.
