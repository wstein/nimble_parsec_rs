# nimble_parsec_rs

This crate is an incremental Rust port of NimbleParsec. It implements the
runtime combinator surface with parity tests before attempting macro/codegen
parity. See [PARITY_MATRIX.md](../PARITY_MATRIX.md) for combinator-level status.

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
- Generator support (`generate/1`) and metadata export

Error messages follow NimbleParsec's phrasing (e.g. `expected ASCII character in
the range "0" to "9"`), but exact `inspect` escaping of non-printable codepoints
and `integer`'s composite "followed by" message are not byte-identical.

## Why this split

NimbleParsec's main advantage is compile-time generation into highly optimized BEAM clauses.
A faithful Rust port likely needs procedural macros and specialized codegen. This phase gives a validated runtime substrate first, which de-risks API and semantics before code generation work.

## Run tests

```bash
cd rust
cargo test
```
