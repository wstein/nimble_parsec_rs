# nimble_parsec_rs (Phase 1)

This crate is an incremental Rust port of selected NimbleParsec behavior.
It intentionally starts with runtime combinators and parity tests before attempting macro/codegen parity.

## Implemented in Phase 1

- Core parser runtime with line/byte cursor tracking
- Proc-macro scaffold (`compile_parser!`) for compile-time parser expressions
- Combinators:
  - `empty`
  - `concat`
  - `ignore`
  - `string`
  - `ascii_char` with positive and negative predicates
  - `utf8_string(min, max)`
  - `integer_exact`, `integer_min`, `integer_range`
  - `optional`
  - `choice`
  - `repeat(min, max)`
  - `map`
  - `tag`

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
- `post_traverse`, `pre_traverse`
- `lookahead` and `lookahead_not`
- `repeat_while`, `times` options parity
- `parsec` (local/remote combinator references)
- Generator support (`generate/1`) and metadata export
- Error message parity with NimbleParsec's detailed labels

## Why this split

NimbleParsec's main advantage is compile-time generation into highly optimized BEAM clauses.
A faithful Rust port likely needs procedural macros and specialized codegen. This phase gives a validated runtime substrate first, which de-risks API and semantics before code generation work.

## Run tests

```bash
cd rust
cargo test
```
