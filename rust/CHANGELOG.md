# Changelog

All notable changes to the `nimble_parsec_rs` crate are documented here. This
crate follows [Semantic Versioning](https://semver.org/). While at `0.x`, the
public API (the ~31 combinators, the fluent `Parser` methods, and the `Value` /
`Cursor` / `ParseFailure` types) may change between minor versions; a typed
`Parser<T>` surface is the planned `1.0` (see the README roadmap).

## [Unreleased]

### Added

- Recursion-depth cap on the interpreter: deeply nested input now returns a
  `ParseFailure` ("maximum recursion depth exceeded") instead of overflowing the
  native call stack. Configurable per parse via `Parser::parse_with_max_depth` /
  `Parser::run_with_max_depth`; the default is `DEFAULT_MAX_RECURSION_DEPTH`
  (256, sized for release builds on a 2 MiB stack).
- Property-based tests (`proptest`) covering totality/monotonicity, the
  `generate` round-trip contract, and recursion safety.
- crates.io publication metadata (`repository`, `keywords`, `categories`,
  `readme`, `rust-version`) and an explicit `1.70` MSRV.

### Changed

- README documents the crate's scope (a runtime + codegen port of a *subset* of
  NimbleParsec, not the full compile-time `defparsec` macro) and a roadmap.

## [0.1.0]

- Initial port: runtime combinator interpreter over a reified `Ast`, plus a
  proc-macro codegen path (`compile_parser!` / `defparsec!` family) specializing
  ~25 combinators. ~31 combinators, line/byte cursor tracking, `generate`-based
  input synthesis, and Criterion benchmarks.
