# NimbleParsec → Rust Combinator Parity Matrix

Combinator-by-combinator comparison of the Elixir public API in
[lib/nimble_parsec.ex](lib/nimble_parsec.ex) against the Rust port in
[rust/src/lib.rs](rust/src/lib.rs).

Status legend:

- ✅ **Ported** — behavior matches NimbleParsec (any naming/signature
  differences are noted).
- ⚠️ **Partial** — present but semantically divergent, or the data exists
  without a dedicated combinator.
- ❌ **Not ported** — no Rust equivalent yet.

## Primitives & character parsers

| Elixir | Rust | Status | Notes |
| --- | --- | --- | --- |
| `empty` | `empty` | ✅ | |
| `string` | `string` | ✅ | |
| `ascii_char` | `ascii_char` | ✅ | Emits the matched byte as an integer codepoint. |
| `utf8_char` | `utf8_char` | ✅ | Codepoint-range predicates via `Utf8Predicate`. |
| `utf8_string` | `utf8_string` | ✅ | Signature `(predicates, min, max)`; ranges honored. |
| `ascii_string` | `ascii_string` | ✅ | Signature `(predicates, min, max)`; emits a string. |
| `integer` | `integer_exact` / `integer_min` / `integer_range` | ✅ | Arbitrary precision; Elixir's overloaded arity is split into three. |
| `bytes` | `bytes` | ✅ | Consumes exactly N bytes; N must fall on a UTF-8 boundary. |
| `eos` | `eos` | ✅ | End-of-string assertion. |

## Combination & control flow

| Elixir | Rust | Status | Notes |
| --- | --- | --- | --- |
| `concat` | `concat` | ✅ | |
| `optional` | `optional` | ✅ | |
| `choice` | `choice` | ✅ | Aggregates branch failure messages (joined with " or "). |
| `repeat` | `repeat` | ✅ | Non-consuming match stops the loop, then `min` is enforced. |
| `repeat_while` | `repeat_while` | ✅ | Takes a native closure instead of an MFA. |
| `times` | `times` | ✅ | |
| `duplicate` | `duplicate` | ✅ | Parses the combinator N times in sequence. |
| `eventually` | `eventually` | ✅ | Skips input (per codepoint) until the inner combinator matches. |
| `lookahead` | `lookahead` | ✅ | |
| `lookahead_not` | `lookahead_not` | ✅ | |

## Transformation & tagging

| Elixir | Rust | Status | Notes |
| --- | --- | --- | --- |
| `tag` | `tag` | ✅ | Wraps results in a tagged list (`Value::Tagged`). |
| `unwrap_and_tag` | `unwrap_and_tag` | ✅ | Tags a single result (`Value::KeyValue`); errors if not exactly one. |
| `ignore` | `ignore` | ✅ | |
| `map` | `map` | ✅ | Per-element transform (`Value → Value`). |
| `reduce` | `reduce` | ✅ | Reduces all results into a single value (`Vec<Value> → Value`). |
| `wrap` | `wrap` | ✅ | Wraps results in a single list value (`Value::List`). |
| `replace` | `replace` | ✅ | Replaces results with a constant value. |
| `label` | `label` | ✅ | Overrides the failure message with `expected <label>`. |
| `debug` | `debug` | ✅ | Prints parser state to stderr; passes results through. |

## Position metadata

| Elixir | Rust | Status | Notes |
| --- | --- | --- | --- |
| `byte_offset` | `byte_offset` | ✅ | Wraps results with the trailing byte offset. |
| `line` | `line` | ✅ | Wraps results with the trailing `{line, line_offset}`. |

## Traversal, references, generators, codegen

| Elixir | Rust | Status | Notes |
| --- | --- | --- | --- |
| `post_traverse` / `pre_traverse` | `post_traverse` / `pre_traverse` | ✅ | Context is threaded through combinators; the callback receives results, context, and position. |
| `quoted_post_traverse` / `quoted_pre_traverse` | — | ❌ | Compile-time traversal variants. |
| `quoted_repeat_while` | — | ❌ | Compile-time `repeat_while` variant. |
| `parsec` | `ParserRef` / `recursive` | ✅ | Forward-declarable references for recursive grammars (runtime, not module-level names). |
| `generate` | `generate` | ✅ | Seeded random input synthesis by walking the AST; round-trips for non-recursive grammars. |
| `defparsec` / `defparsecp` / `defcombinator` / `defcombinatorp` | `compile_parser!` | ⚠️ | Codegen entry points; the Rust proc-macro is a passthrough scaffold with no specialization yet. |

## Summary

The full runtime combinator surface is ported: primitives, control flow,
transforms/tagging, error labeling, position metadata, recursion, and random
generation. `Parser` is now a reified `Ast` walked by an interpreter, so the
grammar is introspectable — which is what unblocked `generate`.

The one remaining ❌ of substance is **compile-time code generation** (the
`defparsec` family / specializing `compile_parser!`): emitting specialized Rust
for a grammar at compile time. The AST makes this tractable, but benchmarking
(see "Codegen verdict" below) shows it is not the highest-leverage next step.

The `quoted_*` traversal variants stay ❌ because they are compile-time forms of
the now-ported runtime `post_traverse`/`pre_traverse`. `parsec` is ported as a
runtime forward reference (`ParserRef`/`recursive`) rather than module-level
named parsers, which belong with codegen.

## Codegen verdict (measured)

`rust/benches/parser_bench.rs` benchmarks the datetime grammar three ways
(representative numbers on one machine — run `cargo bench` for your own):

| Variant | Time | Notes |
| --- | --- | --- |
| Interpreter (combinators) | ~1.36 µs | current `Parser::parse` |
| Hand-written, same tokens | ~0.42 µs | the *fair* codegen ceiling (still allocates the 6 `BigInt`s + `Vec`) |
| Hand-written, length only | ~0.0003 µs | absolute ceiling (allocates nothing) |

A `compile_parser!` specializer must emit **identical tokens**, so its ceiling
is the middle row, not the bottom. The ~0.9 µs gap between the interpreter and
that ceiling is dominated by **avoidable allocations** — `string("-")` under
`ignore` still allocates a `Value::Str` that is immediately discarded (7 such
throwaways here), plus intermediate per-combinator `Vec`s — not by AST match
dispatch. So the higher-leverage, lower-risk optimizations come first: teach
`ignore` to suppress inner-token allocation, reduce intermediate `Vec`s, and
consider a small-integer token representation. Codegen specialization is
deferred: it carries high proc-macro complexity and token-divergence risk for a
speedup an optimized interpreter would largely capture anyway.
