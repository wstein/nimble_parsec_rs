# nimble_parsec_rs

This crate is an incremental Rust port of NimbleParsec. It implements the
runtime combinator surface with parity tests before attempting macro/codegen
parity. See [PARITY_MATRIX.md](../PARITY_MATRIX.md) for combinator-level status.

> **Scope & naming.** This is a runtime-interpreted **plus** codegen port of a
> _subset_ of NimbleParsec's combinator surface — not a drop-in for the full
> compile-time `defparsec` macro. The combinator names and semantics mirror
> NimbleParsec; the output is an untyped `Value` term list (a typed `Parser<T>`
> surface is the planned 1.0 — see [Roadmap](#roadmap)).

## Implemented

- Core parser runtime with line/byte cursor tracking
- Proc-macro scaffold (`compile_parser!`) for compile-time parser expressions
- Combinators:
  - `empty`
  - `concat`
  - `ignore`
  - `string`
  - `ascii_char` with positive and negative predicates (emits the matched byte as an integer codepoint, like NimbleParsec)
  - `utf8_char` and `utf8_string(predicates, min, max)` with codepoint-range predicates
  - `ascii_string(predicates, min, max)`
  - `bytes`, `eos`
  - `integer_exact`, `integer_min`, `integer_range` (arbitrary precision, matching NimbleParsec's unbounded BEAM integers)
  - `optional`
  - `choice`
  - `repeat(min, max)`, `repeat_while`, `times`, `duplicate`
  - `eventually`
  - `lookahead`, `lookahead_not`
  - `map` (per element), `reduce` (all results into one)
  - `post_traverse`, `pre_traverse` (low-level result/context transforms)
  - `tag`, `unwrap_and_tag`, `wrap`, `replace`
  - `label` (custom failure messages)
  - `line`, `byte_offset` (position metadata)
  - `debug` (prints parser state to stderr)
  - `ParserRef` / `recursive` (forward references for recursive grammars)
- `generate(&parser, seed)` produces a random accepted input (seeded; round-trips for non-recursive grammars); `generate_with(&parser, seed, GenerateConfig { .. })` tunes recursion depth and the repeat window

  ## Benchmark scaffold

  Criterion benchmarks are available in [benches/parser_bench.rs](benches/parser_bench.rs):

  - `runtime_builder_parse_datetime`
  - `proc_macro_builder_parse_datetime`

  Run:

  ```bash
  cargo bench
  ```

## Not Yet Ported

- Compile-time parser generation equivalent to `defparsec/defcombinator`

Error messages follow NimbleParsec's phrasing (e.g. `expected ASCII character in
the range "0" to "9"`), but exact `inspect` escaping of non-printable codepoints
and `integer`'s composite "followed by" message are not byte-identical.

## Why this split

NimbleParsec's main advantage is compile-time generation into highly optimized BEAM clauses.
A faithful Rust port likely needs procedural macros and specialized codegen. The runtime substrate here is now a reified `Ast` walked by an interpreter, which both de-risks API/semantics and is the structure codegen would lower. `compile_parser!` is still a validated passthrough; true specialization (emitting generated parsing code that produces identical tokens) remains future work.

## Recursion safety

`recursive` / `ParserRef` grammars recurse on the native call stack. To keep
deeply nested untrusted input from overflowing the stack (an uncatchable abort),
each parse is bounded by [`DEFAULT_MAX_RECURSION_DEPTH`] (256); exceeding it
returns a `ParseFailure` rather than crashing. Tune per parse with
`Parser::parse_with_max_depth(input, max_depth)` (or `run_with_max_depth`) — a
lower bound to harden against hostile input, a higher one for legitimately deep
grammars. The default suits release builds on a 2 MiB stack; debug builds have
larger frames, so lower the cap if you run untrusted input through a debug build.

[`DEFAULT_MAX_RECURSION_DEPTH`]: src/lib.rs

## Roadmap

Acknowledged, scheduled work — not accidents:

- **Typed `Parser<T>` surface (1.0).** Make combinators generic over their output
  type (à la nom/winnow/chumsky), retiring the dynamic `Value` enum.
- **Structured errors.** `ParseFailure::reason` is currently a freeform string;
  expose a typed "expected set" carrying position + alternatives.
- **Property tests + fuzz corpus.** Seed property tests from the existing
  `generate` facility, complementing the deterministic per-combinator tests.
- **crates.io publication.** Fill in Cargo.toml metadata
  (`repository`/`keywords`/`categories`/`readme`) and publish `parsec_macro`
  separately — deferred until there is a second consumer beyond Stem.

## Run tests

```bash
cd rust
cargo test
```
