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
| `defparsec` / `defparsecp` / `defcombinator` / `defcombinatorp` | `defparsec!` / `defparsecp!` / `defcombinator!` / `defcombinatorp!` | ✅ | Named parse functions and combinator factories. `defparsec!`/`defparsecp!` generate specialized inline code for the codegen-supported subset (`string`, `integer_exact`, `integer_min`, `ignore`, `concat`, `empty`, `eos`); otherwise cache the runtime `Parser` in a `OnceLock`. `defcombinator!`/`defcombinatorp!` always cache via `OnceLock`. |
| `defparsec` (inline use) | `compile_parser!` | ✅ | Generates a specialized `Parser` backed by a native closure for fully-recognizable expressions; falls back to the unchanged runtime expression otherwise. |

## Summary

The full runtime combinator surface is ported: primitives, control flow,
transforms/tagging, error labeling, position metadata, recursion, and random
generation. `Parser` is now a reified `Ast` walked by an interpreter, so the
grammar is introspectable — which is what unblocked `generate`.

The `defparsec!`/`defparsecp!`/`defcombinator!`/`defcombinatorp!` macro family
is now implemented. `defparsec!`/`defparsecp!` emit specialized inline Rust (no
`Ast` interpreter, no intermediate `Vec`s) for the codegen-supported combinator
subset (`string`, `integer_exact`, `integer_min`, `ignore`, `concat`, `empty`,
`eos`); grammars using combinators outside that subset fall back to a
`OnceLock`-cached runtime parser. `compile_parser!` follows the same strategy,
wrapping the generated code in an `Ast::Native` closure. `defcombinator!`/
`defcombinatorp!` always use `OnceLock`-cached runtime parsers and return a
clonable `Parser`.

The `quoted_*` traversal variants stay ❌ because they are compile-time forms of
the now-ported runtime `post_traverse`/`pre_traverse`. `parsec` is ported as a
runtime forward reference (`ParserRef`/`recursive`) rather than module-level
named parsers, which belong with codegen.

## Codegen verdict (measured)

`rust/benches/parser_bench.rs` benchmarks the datetime grammar four ways
(representative numbers on one machine — run `cargo bench` for your own):

| Variant | Time | Notes |
| --- | --- | --- |
| Interpreter (combinators) | ~1.16 µs | `Parser::parse` walking the `Ast` (after `ignore` token-suppression) |
| **Specialized `compile_parser!`** | **~0.56 µs** | generated inline code (`Ast::Native`), emits identical tokens |
| Hand-written, same tokens | ~0.43 µs | the *fair* codegen ceiling (still allocates the 6 `BigInt`s + `Vec`) |
| Hand-written, length only | ~0.0003 µs | absolute ceiling (allocates nothing) |

Codegen specialization is **implemented for the recognizable subset** (`string`,
`integer_exact`, `integer_min`, `ignore`, `concat`, `empty`, `eos`); broader
combinator coverage is future work. A specializer must emit **identical
tokens**, so its ceiling is the third row, not the bottom — and the generated
code already lands at ~0.56 µs, about **2.5× faster than the interpreter** and
within ~30% of that fair ceiling. The remaining gap is dominated by the
unavoidable token allocations (6 `BigInt`s + a `Vec`); `ignore`d
sub-combinators no longer allocate throwaway tokens in either the generated
path or the interpreter — the interpreter threads an `emit` flag so leaves
under `ignore` skip building tokens, worth ~16% on this grammar (1.38 → 1.16
µs). A further win would be a small-integer token representation to avoid the
`BigInt` heap allocations entirely.
