# NimbleParsec → Rust Combinator Parity Matrix

Combinator-by-combinator comparison of the Elixir public API in
[lib/nimble_parsec.ex](lib/nimble_parsec.ex) against the **typed** Rust port in
[rust/src/typed.rs](rust/src/typed.rs) (re-exported at the crate root). The port
is generic over each combinator's `Output`, so where NimbleParsec emits an
untyped term list, the Rust API yields a concrete type — the "Notes" column flags
where that changes the shape.

Where the idiomatic Rust name differs from NimbleParsec's, the
[`nimble_parsec_rs::nimble`](rust/src/typed.rs) module re-exposes it under the
NimbleParsec name (a free function) for porters — those are flagged "(alias …)".

Status legend:

- ✅ **Ported** — a direct equivalent exists (naming/shape differences noted).
- 🧩 **Composed** — no dedicated combinator, but the behavior is expressed by
  composing existing ones (the idiom is noted).
- ❌ **Not ported** — no equivalent yet (tracked in the README roadmap).
- ➖ **N/A** — obsolete under the typed design (the reason is noted).

## Primitives & character parsers

| Elixir | Rust (typed) | Status | Notes |
| --- | --- | --- | --- |
| `string` | `literal` (alias `string`) | ✅ | Yields the matched `&str`. |
| `utf8_char` | `any` / `satisfy` | ✅ | Yields a `char`. `satisfy(label, pred)` for a class. |
| `ascii_char` | `satisfy` / `one_of` / `none_of` | ✅ | Yields a `char`; predicates are plain closures, not a range list. |
| `utf8_string` / `ascii_string` | `take_while` / `take_while1` | ✅ | Yields a `&str`. `take_while1` enforces a minimum of 1. |
| `integer` | `integer()` | ✅ | A digit run parsed into `i64` (overflow → error). Use `digits().try_map(...)` for other widths or a sign. |
| `eos` | `eof` (alias `eos`) | ✅ | End-of-input assertion. |
| `empty` | `empty()` | ✅ | Always succeeds, consuming nothing; handy as a final `choice` branch. |
| `bytes` | `bytes(n)` | ✅ | Consumes exactly `n` bytes as `&str`; `n` must land on a UTF-8 character boundary. Arbitrary non-UTF-8 byte parsing still awaits byte-slice input. |

## Combination & control flow

| Elixir | Rust (typed) | Status | Notes |
| --- | --- | --- | --- |
| `concat` | `.then` (alias `concat`) | ✅ | Yields a tuple `(A, B)`. `.ignore_then` / `.then_ignore` keep one side. |
| `optional` | `.optional` | ✅ | Yields `Option<O>`. |
| `choice` | `choice(alts)` / `.or` | ✅ | `alts` is an array `[p; N]` (same type) or a tuple `(a, b, …)` ≤ arity 8 (different types, one `Output`); `.or` chains two. Failures union the `expected` set and join reasons with `" or "`. |
| `repeat` | `.repeated` | ✅ | Yields `Vec<O>`; a non-consuming match stops the loop. `.repeated_at_least(min)` for a floor. |
| `times` | `.repeated_in(min, max)` | ✅ | Exact `n` is `.repeated_in(n, n)`. |
| `lookahead` | `lookahead` | ✅ | Zero-width; yields the inner output without consuming. |
| `lookahead_not` | `not` | ✅ | Zero-width negative assertion, yields `()`. |
| `repeat_while` | `repeated_until(p, stop)` | ✅ | Repeats `p` until `stop` would match (the terminator is left unconsumed). For an arbitrary boolean predicate, compose with `not`/`lookahead`. |
| `eventually` | `eventually(p)` | ✅ | Skips input one character at a time until `p` matches, returning its output. |
| `duplicate` | `duplicate(p, n)` / `.repeated_in(n, n)` | ✅ | Exactly-`n` repetition; `duplicate` is the NimbleParsec-named alias. |

## Transformation & tagging

| Elixir | Rust (typed) | Status | Notes |
| --- | --- | --- | --- |
| `map` | `.map` | ✅ | Transforms the whole output (`O -> U`), not per-element. |
| `replace` | `.to(value)` | ✅ | Replaces the output with a constant. |
| `ignore` | `.ignored` | ✅ | Discards the output (yields `()`). |
| `label` | `.labelled` | ✅ | Overrides the failure message and the structured `expected`. |
| `post_traverse` / `pre_traverse` | `.post_traverse` / `.pre_traverse` | ✅ | Fallible transform with the end / start `Cursor`. Context is threaded by capturing `Cell`/`RefCell` state in the callback (idiomatic interior mutability; not auto-rolled-back on backtrack, as in winnow). `.try_map` is the position-free variant. |
| `reduce` | `.fold(init, f)` | ✅ | Folds a repetition into an accumulator without the intermediate `Vec`. |
| `tag` / `unwrap_and_tag` | `.map(\|o\| …)` | ➖ | Deliberately not resugared: tagging is `.map` into a typed value/enum variant — a stringly-typed `.tagged(name)` would re-import the dynamic-term model the typed surface removes. |
| `wrap` | `.map(\|o\| vec![o])` | ➖ | The output is already typed; wrap with `.map` if a `Vec` is wanted. |
| `debug` | `.debug(label)` | ✅ | Traces the parser to stderr (entry position + outcome), passing the output through. |

## Position metadata

| Elixir | Rust (typed) | Status | Notes |
| --- | --- | --- | --- |
| `byte_offset` | `.with_byte_offset()` | ✅ | Pairs the output with the trailing byte offset: `(O, usize)`. |
| `line` | `.with_line()` | ✅ | Pairs the output with `(1-based line, byte offset of the line start)`: `(O, (usize, usize))`. |

## Recursion, generation & definition macros

| Elixir | Rust (typed) | Status | Notes |
| --- | --- | --- | --- |
| `parsec` | `recursive` | ✅ | Forward-declared, self-referential parser (boxed); depth-bounded by the recursion cap. |
| `generate` | `generate(&p, seed)` | ✅ | Seeded input synthesis for **non-recursive** grammars (the `Generate` trait, enforced by trait bounds); dependency-free PRNG. Best-effort with negative assertions / restrictive predicates. |
| `defparsec` / `defparsecp` / `defcombinator` / `defcombinatorp` | plain `fn … -> impl Parser` | ➖ | The codegen macros existed to specialize the runtime interpreter; typed combinators are already monomorphized by the compiler, so a named parser is just a function. |
| `quoted_*` traversal variants | — | ➖ | Compile-time forms of `post_traverse`/`repeat_while`; no analogue in a non-macro design. |

## Rust additions (no direct NimbleParsec name)

| Rust (typed) | Purpose |
| --- | --- |
| `one_of(set)` / `none_of(set)` | A character in / not in a literal set. |
| `take_while` / `take_while1` / `digits` | Character-run leaves yielding `&str`. |
| `.ignore_then` / `.then_ignore` | Sequence keeping only the right / left side. |
| `.to(value)` | Constant replacement (NimbleParsec's `replace`, but typed). |
| `.try_map(f)` | Fallible/validating transform. |
| `.repeated_in(min, max)` / `.repeated_at_least(min)` | Bounded repetition. |
| `delimited(open, content, close)` | Content between two delimiters. |
| `separated_by` / `separated_by1` | A separated list (no trailing separator). |
| `repeated_until(p, end)` | Repeat until a terminator (not consumed). |
| `parse` / `parse_partial` (+ `*_with_max_depth`) | Run the parser, optionally returning the remainder or overriding the recursion cap. |

## Summary

The typed surface covers NimbleParsec's everyday grammar-building set —
primitives, sequencing, alternation, repetition, optionality, zero-width
assertions, transformation, error labelling, and recursion — with outputs that
are real Rust types rather than a dynamic term list. Several NimbleParsec
combinators (`tag`, `reduce`, `wrap`, `unwrap_and_tag`) become ordinary `.map`
calls because the output is already typed, and the `defparsec`/`defcombinator`
codegen family is obsolete because the compiler monomorphizes the combinators
directly.

Outstanding (tracked in [rust/README.md](rust/README.md)): generic / binary
(non-UTF-8) input — the surface is `&str`-only, so `bytes` requires UTF-8 boundaries
and there is no byte/bitstring parsing. This is the one remaining structural gap.
