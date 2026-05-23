# NimbleParsec -> Rust Migration Review

## Executive Summary

The current repository is a complete Elixir implementation of NimbleParsec with extensive behavior coverage (160 tests passing). Migrating directly to a one-shot Rust rewrite is high risk because NimbleParsec's key value is compile-time specialization, not just parser combinator semantics.

A safer strategy is staged migration:

1. Build a runtime-semantics Rust core with parity tests against representative examples.
2. Port advanced combinators and context semantics.
3. Add Rust compile-time code generation (proc macros) to recover performance and ergonomics.
4. Add benchmark and fuzzing gates before declaring parity.

Phase 1 has been implemented in [rust/](rust/):

- Runtime parser core
- Basic combinators and token model
- Parity tests for selected integration scenarios

## Codebase Assessment (Elixir)

Strengths:

- Broad combinator surface and semantics
- Good integration tests and generator tests
- Tight compiler/recorder architecture for generated parser clauses

Migration challenges:

- Compile-time clause generation (`defparsec`) has no trivial Rust equivalent
- Error reporting and labels are nuanced and heavily tested
- Context propagation and traversal hooks rely on BEAM-style execution model
- Local/remote parsec references require robust module-level metadata handling

## Suggested Architecture for Full Rust Port

- `nimble_parsec_rs_core`:
  - Cursor, errors, parser trait/object, token representation
- `nimble_parsec_rs_combinators`:
  - Runtime combinators and semantic transforms
- `nimble_parsec_rs_codegen` (proc-macro crate):
  - Declarative macro API that emits specialized parser code
- `nimble_parsec_rs_compat`:
  - Optional compatibility layer for users migrating from Elixir concepts

## Security and Reliability Notes

- Prevent infinite loops in repetition combinators when no input is consumed
- Guard UTF-8 boundary handling and byte offsets carefully
- Add property-based tests/fuzzing for malformed UTF-8 and adversarial inputs
- Establish panic-free APIs for parsing failures (use typed errors)

## QA Plan

- Port tests in priority order:
  - Integration tests first
  - Core combinator tests (ascii/utf8/string/int)
  - Control-flow combinators (`choice`, `repeat`, `lookahead`)
  - Traversal/context combinators
  - Generator behavior
- Add differential testing against Elixir outputs for shared fixtures (the runner compares rest, byte offset, token count, and token values)
- Add criterion benchmarks to compare runtime combinators and generated variants

## Mid-Term Improvements

- Build a compatibility DSL that mirrors the NimbleParsec mental model
- Add serde-friendly token outputs
- Add no_std feasibility study for embedded parsing scenarios
- Add CI matrix for stable/beta/nightly to protect proc macro behavior

## Current Status

- Elixir baseline: `mix test` -> 160 tests passing
- Rust: runtime subset with thematic parity tests ([choice_and_repeat.rs](rust/tests/choice_and_repeat.rs), [control_flow_phase2.rs](rust/tests/control_flow_phase2.rs), [integration_examples.rs](rust/tests/integration_examples.rs)) and a value-level [differential_runner.rs](rust/tests/differential_runner.rs) against shared Elixir fixtures
