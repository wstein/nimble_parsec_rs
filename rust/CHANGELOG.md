# Changelog

All notable changes to the `nimble_parsec_rs` crate are documented here. This
crate follows [Semantic Versioning](https://semver.org/). While at `0.x`, the
public API (the ~31 combinators, the fluent `Parser` methods, and the `Value` /
`Cursor` / `ParseFailure` types) may change between minor versions; a typed
`Parser<T>` surface is the planned `1.0` (see the README roadmap).

## [Unreleased]

A breaking redesign (RFC 0001): the crate is now a **typed** parser-combinator
library, replacing the dynamic `Value`-based port.

### Added

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
- Recursion-depth cap (`DEFAULT_MAX_RECURSION_DEPTH`, default 256), overridable
  via `Parser::parse_with_max_depth`, returning a `ParseFailure` instead of
  overflowing the stack.
- Property-based tests (`proptest`), publication metadata, and a `1.70` MSRV.

### Removed

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
