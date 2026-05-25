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
| `defparsec` / `defparsecp` / `defcombinator` / `defcombinatorp` | `defparsec!` / `defparsecp!` / `defcombinator!` / `defcombinatorp!` | ✅ | Named parse functions and combinator factories. `defparsec!`/`defparsecp!` generate specialized inline code for nearly the whole static combinator surface — primitives (`string`, `integer_exact`/`integer_min`/`integer_range`, `ascii_char`, `utf8_char`, `ascii_string`, `utf8_string`, `bytes`, `empty`, `eos`), `concat`/`ignore`/`choice`/`optional`/`repeat`/`duplicate`/`eventually`/`repeat_while`, the transform/tagging/position combinators, `post_traverse`/`pre_traverse`, and `label`/`lookahead`/`lookahead_not`/`debug`. Only `times`, recursive references (`parsec`/`ParserRef`), and calls whose arguments aren't literal (a `vec!`/predicate list or sub-parser supplied via a variable) fall back to a `OnceLock`-cached runtime `Parser`. `defcombinator!`/`defcombinatorp!` always cache via `OnceLock`. |
| `defparsec` (inline use) | `compile_parser!` | ✅ | Generates a specialized `Parser` backed by a native closure for fully-recognizable expressions; falls back to the unchanged runtime expression otherwise. |

## Summary

The full runtime combinator surface is ported: primitives, control flow,
transforms/tagging, error labeling, position metadata, recursion, and random
generation. `Parser` is now a reified `Ast` walked by an interpreter, so the
grammar is introspectable — which is what unblocked `generate`.

The `defparsec!`/`defparsecp!`/`defcombinator!`/`defcombinatorp!` macro family
is now implemented. `defparsec!`/`defparsecp!` emit specialized inline Rust (no
`Ast` interpreter) for nearly the whole static combinator surface — primitives,
`concat`/`ignore`, control flow (`choice`/`optional`/`repeat`/`duplicate`/
`eventually`/`repeat_while`), the transform/tagging/position combinators,
`post_traverse`/`pre_traverse`, and `label`/`lookahead`/`lookahead_not`/`debug`.
Only `times`, recursive references (`parsec`/`ParserRef`), and calls whose
arguments aren't literal (a `vec!`/predicate list or sub-parser supplied via a
variable) fall back to a `OnceLock`-cached runtime parser. `compile_parser!`
follows the same strategy, wrapping the generated code in an `Ast::Native`
closure. `defcombinator!`/`defcombinatorp!` always use `OnceLock`-cached runtime
parsers and return a clonable `Parser`.

The `quoted_*` traversal variants stay ❌ because they are compile-time forms of
the now-ported runtime `post_traverse`/`pre_traverse`. `parsec` is ported as a
runtime forward reference (`ParserRef`/`recursive`) rather than module-level
named parsers, which belong with codegen.

## Codegen verdict (measured)

`rust/benches/parser_bench.rs` benchmarks the datetime grammar four ways
(representative numbers on one machine — run `cargo bench` for your own):

| Variant | Time | Notes |
| --- | --- | --- |
| Interpreter (combinators) | ~0.25 µs | `Parser::parse` walking the `Ast` |
| **Specialized `compile_parser!`** | **~0.15 µs** | generated inline code (`Ast::Native`), emits identical tokens |
| Hand-written, same tokens | ~0.063 µs | the *fair* codegen ceiling (still allocates the result `Vec`) |
| Hand-written, length only | ~0.0003 µs | absolute ceiling (allocates nothing) |

Codegen specialization now covers nearly the whole static combinator surface
(see the macro row above); only `times`, recursive references, and calls with
non-literal arguments still fall back to the runtime parser. A specializer must
emit **identical tokens**, so its ceiling is the third row, not the bottom — and
the generated code lands at ~0.15 µs, about **1.7× faster than the interpreter**
and within ~2.5× of that fair ceiling.

Three optimizations are folded in. `ignore`d sub-combinators allocate no
throwaway tokens (the interpreter threads an `emit` flag so leaves under
`ignore` skip building tokens). `Value::Int` uses the small-integer [`Integer`]
representation, so in-range integers no longer heap-allocate a `BigInt` — this
alone took the interpreter from ~1.16 to ~0.81 µs and the specialized path from
~0.56 to ~0.14 µs. Finally, the interpreter now threads **one shared token
accumulator** (`&mut Vec<Value>`) through the whole parse instead of returning a
fresh `Vec` per combinator: leaves push into it and combinators that backtrack
(`choice`, `optional`, the repetitions, `eventually`, the lookaheads) truncate
it on a discarded attempt. That removed the per-leaf and per-`concat`
allocations and took the interpreter from ~0.77 to ~0.25 µs (**~3× faster**, far
more than the ~12% first estimated — the result `Vec`s dominated). The
specialized path is unchanged: its top-level `Native` parser still returns its
own `Vec`, which the interpreter now *adopts* directly when the accumulator is
empty (the whole-grammar case) rather than copying.

The remaining interpreter cost is dominated by `Ast` dispatch and recursion (an
`Arc`-walked tree), not allocation: context clones of an empty map are free, and
the shared accumulator means only the final result `Vec` is allocated. The
higher-leverage direction from here is broadening the codegen subset so more
grammars take the ~0.15 µs specialized path.
