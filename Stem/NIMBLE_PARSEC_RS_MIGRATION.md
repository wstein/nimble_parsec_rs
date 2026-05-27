<!-- SPDX-License-Identifier: Apache-2.0 -->

# `nimble_parsec_rs` — Migration & Pre-Release Review

_Status: **verified against crate source 2026-05-27**. Audience: maintainers of `wstein/nimble_parsec_rs` before its first tagged release. Scope: the Rust port of Elixir's NimbleParsec, as consumed by Stem's native front end._

> **Revision note (2026-05-27).** An earlier draft of this document was _synthesized from inference_ — it explicitly warned that several findings were "inferences from the consuming code, not confirmed against the crate source" and "must be verified before being treated as fact." Those findings have now been checked directly against [`rust/src/lib.rs`](../rust/src/lib.rs), [`rust/parsec_macro/src/lib.rs`](../rust/parsec_macro/src/lib.rs), [`rust/tests/`](../rust/tests/), and [`rust/benches/parser_bench.rs`](../rust/benches/parser_bench.rs). **Most of the earlier "blockers" did not survive that check** and are corrected below. One — unbounded recursion — was confirmed, and has since been fixed. This revision supersedes the inferred version.

---

## 1. Was it a straightforward migration? No — a _model_ port, not a transliteration

The honest headline stands: **Elixir NimbleParsec and a Rust combinator library are different machines wearing the same name.**

- **Elixir NimbleParsec** is a _compile-time macro_ (`defparsec`/`defparsecp`) that generates specialized binary-pattern-matching functions.
- **What this crate provides** is _both_ a runtime combinator interpreter **and** a compile-time codegen path (see §2.1) — the latter narrowing the gap to NimbleParsec's compile-time premise far more than the earlier draft assumed.

### What was actually ported (scope)

Only the lexer and the expression tokenizer became combinators on the _consumer_ side:

- [`Stem/np_lexer.rs`](np_lexer.rs) — the `do_lex` combinator grammar plus a `tokenize` assembler.
- [`Stem/np_expr.rs`](np_expr.rs) — the top-level expression tokenizer.

The structural recursive-descent block parser stayed hand-written, deliberately, because validating `{{/kind}}` against `{{#kind}}` is context-sensitive and a context-free grammar buys no clarity there. Every migration phase was gated by the BEAM↔Rust differential (`mix stem.native.compile_diff` at 0 mismatches, plus the fuzz corpus) before the swap landed.

---

## 2. The findings — corrected against the crate source

At a glance (✅ = not a problem / already addressed; ⚠️ = real, now fixed; ◻️ = acknowledged roadmap debt):

| # | Earlier claim | Verified reality | Status |
| --- | --- | --- | --- |
| 2.1 | "Runtime interpreter only; discards NimbleParsec's perf premise; no benchmark" | Dual path: interpreter **+** proc-macro codegen; Criterion bench exists | ✅ disproven |
| 2.2 | "Dynamic `Value` enum is the biggest wart" | True it's dynamic, but cost is mitigated (small-int fast path + shared accumulator); typed redesign is a scheduled 1.0 goal | ◻️ roadmap |
| 2.3 | "Codepoint round-trips; add a `utf8_string`" | `utf8_string` already accumulates directly into one `Value::Str` | ✅ largely done |
| 2.4 | "`recursive` is unbounded → stack-overflow DoS" | **Confirmed.** Now fixed with a recursion-depth cap | ⚠️ fixed |
| 2.5 | "Byte-offset only; line/col is a parity gap" | `Cursor` carries `line`, `line_start_offset`, `byte_offset`; a `line` combinator exists | ✅ disproven |
| 2.6 | "~14 names; oversells coverage" | ~31 combinators incl. `reduce`/`map`/`tag`/`times`/`eventually`/`lookahead`/`line` | ✅ understated |

### 2.1 Execution model — interpreter **and** codegen (claim disproven)

The crate is **not** runtime-only. Alongside the AST interpreter ([`run_ast`](../rust/src/lib.rs)), it ships a proc-macro codegen path — `compile_parser!`, `defparsec!`, `defparsecp!`, `defcombinator!`, `defcombinatorp!` (defined in [`rust/parsec_macro/src/lib.rs`](../rust/parsec_macro/src/lib.rs), re-exported from `lib.rs`) — that specializes ~25 combinators into native closures. And the perf claim is **measurable**, not hand-waved: [`rust/benches/parser_bench.rs`](../rust/benches/parser_bench.rs) is a Criterion harness comparing a hand-written scanner, the interpreter, and the codegen path (per `PARITY_MATRIX.md`: interpreter ~0.25 µs, codegen ~0.15 µs, hand-written-same-tokens ~0.063 µs on the datetime grammar). The earlier "no benchmark / lost the premise" framing was wrong.

### 2.2 The dynamic `Value` enum (acknowledged roadmap debt)

`Value` ([`lib.rs`](../rust/src/lib.rs): `Int`/`Str`/`List`/`Tagged`/`KeyValue`) is faithful to NimbleParsec's untyped term list and remains un-idiomatic versus generic-output crates (nom/winnow/chumsky). But its cost is mitigated: `Integer` has a small-`i64` fast path (so codepoints don't heap-allocate a bigint), and the shared-accumulator refactor removed per-combinator `Vec` allocations (~3× interpreter speedup per the notes). A typed `Parser<T>` redesign is the **scheduled 1.0 goal** (see [`rust/README.md`](../rust/README.md) roadmap), not an accident.

### 2.3 Codepoint round-trips (largely done)

`utf8_char` emits one `Value::Int` per codepoint (the faithful primitive), but `utf8_string` already accumulates directly into a single `Value::Str`. The "concrete improvement" the earlier draft proposed is, in substance, already present.

### 2.4 Recursion — the one real finding, now fixed

**Confirmed:** the interpreter (`run_ast`) is plain recursive descent, and the `Ast::Reference` arm — how `recursive`/`ParserRef` resolve — recursed with no depth bound. Since [`np_expr.rs`](np_expr.rs) uses `recursive` for nested parens, input like `{{ ((((…)))) }}` could exhaust the native stack and **abort the process** (a Rust stack overflow is uncatchable). Stem is web-facing, so this was a genuine DoS.

**Fix (landed):** a per-parse recursion budget, spent only at `Ast::Reference` (the sole node where _input_ drives unbounded recursion — `repeat`/`eventually` are iterative, `choice`/`concat` recurse only to the static grammar depth). At zero it returns a `ParseFailure` ("maximum recursion depth exceeded") instead of overflowing. Default `DEFAULT_MAX_RECURSION_DEPTH = 256` (sized for release builds on a 2 MiB stack), overridable via `Parser::parse_with_max_depth` / `run_with_max_depth`. Regression test: [`rust/tests/recursion_depth.rs`](../rust/tests/recursion_depth.rs) — the class of input Stem's differential harness can never generate.

### 2.5 Position tracking (claim disproven)

`Cursor` ([`lib.rs`](../rust/src/lib.rs)) carries `line` (1-based), `line_start_offset`, and `byte_offset`; column is `byte_offset - line_start_offset`. A `line` combinator and a [`position.rs`](../rust/tests/position.rs) test exist. The consumer (Stem) _chooses_ to use only `byte_offset` — that is a consumer decision, not a crate gap.

### 2.6 Coverage (claim understated)

The public surface is ~31 combinators plus types and the 5 macros — including the very names the earlier draft listed as missing: `reduce`, `map`, `tag`, `unwrap_and_tag`, `times`, `eventually`, `lookahead`, `line`, `byte_offset`. It is still a _subset_ of NimbleParsec (honest to say so in the README), but a far larger one than "~14 names."

---

## 3. What was done well

- **Differential-gated migration.** Each phase kept `compile_diff` / `verify` / `fuzz` green before landing — the correct way to swap a parser's execution model under a byte-parity contract.
- **Pragmatic scoping.** Not forcing the context-sensitive block parser into context-free combinators avoided a classic over-abstraction trap.
- **Independent test coverage.** 110+ crate-level tests across [`rust/tests/`](../rust/tests/) (≈28/30 combinators), contradicting the earlier "zero independent tests" claim. The differential harness is supplementary, not the sole net.
- **Measured performance.** A Criterion benchmark exists and is wired with real numbers.

---

## 4. Release readiness — what actually remained

The earlier draft's elaborate debate is dropped: it argued against a crate that, on inspection, mostly does not exist (no codegen, no tests, no benchmark, no line/col — all false). The real, verified picture is small:

- **One safety blocker — recursion depth.** ✅ Fixed (§2.4) with a regression test.
- **Rust CI.** ✅ Added a `cargo test` + `clippy -D warnings` + `cargo bench --no-run` job to [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) (it previously ran Elixir only, leaving the Rust tests unguarded).
- **Property tests.** ✅ Added a `proptest` suite ([`rust/tests/properties.rs`](../rust/tests/properties.rs)) covering totality/monotonicity, the `generate` round-trip, and recursion safety.
- **Structured errors.** ✅ `ParseFailure` now carries a structured `expected: Vec<String>` alongside `reason`, unioned across `choice` branches (the original review's "strong should").
- **Publication metadata.** ✅ Cargo.toml `repository`/`keywords`/`categories`/`readme` + a `1.70` MSRV; a `CHANGELOG.md` states the 0.x semver policy.
- **Truth-in-naming.** ✅ A README note: a runtime **+ codegen** port of a _subset_ of NimbleParsec, not the full compile-time `defparsec` macro.
- **Tag & repin.** ⏳ Cut `v0.1.0`; repin Stem's dependency off `branch = develop` onto the tag.

### Deferred to roadmap (acknowledged, scheduled — not accidents)

- Typed `Parser<T>` redesign (1.0 goal — erases the dynamic `Value` debt, §2.2). A design RFC is being drafted as the first phase.
- A persisted fuzz corpus (`cargo-fuzz`) to complement the property tests.
- Publishing `parsec_macro` as its own crate (metadata is now in place) — only when a second consumer beyond Stem is real. The near-term release is an **internal tag**.

---

## 5. Before release — concrete checklist

The one hard **safety** blocker is resolved; the rest are internal-tag readiness items.

0. **Recursion-depth bound — safety blocker.** ✅ Done — cap at `Ast::Reference`, default 256, overridable, returns a parse error; regression test added (§2.4).
1. **Rust CI.** ✅ Done — `cargo test` / `clippy -D warnings` / `cargo bench --no-run` in CI.
2. **Truth-in-naming README line.** Runtime + codegen, _subset_ of NimbleParsec, not the macro.
3. **Pin a tag** (`v0.1.0`) and repin Stem off `branch = develop`.
4. _(Roadmap, not gating an internal tag)_ typed `Parser<T>`, structured errors, property/fuzz tests, crates.io ceremony.

---

## References

- ADR-0008 — _Rust front end ported to nimble_parsec_rs_ (`docs/modules/architecture/pages/09-architecture-decisions.adoc`).
- Crate source: [`rust/src/lib.rs`](../rust/src/lib.rs), [`rust/parsec_macro/src/lib.rs`](../rust/parsec_macro/src/lib.rs), [`rust/tests/`](../rust/tests/), [`rust/benches/parser_bench.rs`](../rust/benches/parser_bench.rs), `PARITY_MATRIX.md`.
- Consuming code: [`Stem/np_lexer.rs`](np_lexer.rs), [`Stem/np_expr.rs`](np_expr.rs).
- Disclosure: `THIRD-PARTY-LICENSES.md` (Apache-2.0).
