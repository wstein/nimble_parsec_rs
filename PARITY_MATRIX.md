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
| `choice` | `choice` | ⚠️ | Returns the first branch's error; Elixir aggregates labels. |
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
| `post_traverse` / `pre_traverse` | — | ❌ | Require threading a parser context through every combinator. |
| `quoted_post_traverse` / `quoted_pre_traverse` | — | ❌ | Compile-time traversal variants. |
| `quoted_repeat_while` | — | ❌ | Compile-time `repeat_while` variant. |
| `parsec` | — | ❌ | Local/remote combinator references (named-parser registry, recursion). |
| `generate` | — | ❌ | Random input generation from a parser. |
| `defparsec` / `defparsecp` / `defcombinator` / `defcombinatorp` | `compile_parser!` | ⚠️ | Codegen entry points; the Rust proc-macro is a passthrough scaffold with no specialization yet. |

## Summary

The full runtime combinator surface is ported: primitives, control flow,
transforms/tagging, error labeling, and position metadata. The remaining ❌
rows are not drop-in combinators but four larger design efforts:

1. **Context-threaded traversal** (`post_traverse`/`pre_traverse` and their
   `quoted_*` variants) — needs a parser context carried through the chain.
2. **Combinator references** (`parsec`) — needs a named-parser registry to
   support recursion and modular grammars.
3. **Generators** (`generate`) — random input synthesis from a parser.
4. **Compile-time code generation** (`defparsec` family) — the proc-macro
   specialization that gives NimbleParsec its performance.

One ⚠️ remains within ported combinators: `choice` returns the first branch's
error rather than aggregating labels across branches.
