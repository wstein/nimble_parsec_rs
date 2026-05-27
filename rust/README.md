# nimble_parsec_rs

A Rust port of [NimbleParsec](https://github.com/dashbitco/nimble_parsec) with an
idiomatic, **typed** combinator surface. Each combinator is generic over its
output, so grammars compose and type-check at compile time with no runtime
tagging — `literal` yields `&str`, `then` yields a tuple, `repeated` yields a
`Vec`, and `map` threads any output type through. Zero runtime dependencies.

```rust
use nimble_parsec_rs::{digits, literal, Parser};

// "(" digits ")" → the number, as a u32.
let number = literal("(")
    .ignore_then(digits())
    .then_ignore(literal(")"))
    .map(|ds: &str| ds.parse::<u32>().unwrap());

assert_eq!(number.parse("(42)").unwrap(), 42);
```

> **Scope.** A _subset_ of NimbleParsec's combinator surface, reimagined for
> Rust — the names and semantics mirror NimbleParsec, but the output is a real
> type rather than NimbleParsec's untyped term list. `&str` input only (for now).

## Combinators

Leaves (free functions):

- `literal(&str)` — match an exact string, yielding the slice
- `any()` — any one character
- `satisfy(label, pred)` — a character matching a predicate
- `one_of(set)` / `none_of(set)` — a character in / not in a set
- `take_while(pred)` / `take_while1(pred)` — a run of matching characters
- `digits()` — one or more ASCII digits
- `eof()` — end of input
- `choice(alts)` — the first matching alternative; `alts` is an array `[p; N]` (same type) or a tuple `(a, b, …)` up to arity 8 (different types, one `Output`)
- `lookahead(p)` / `not(p)` — zero-width positive / negative assertions
- `delimited(open, content, close)` — `content` between two delimiters
- `separated_by(item, sep)` / `separated_by1(item, sep)` — a separated list (no trailing separator)
- `repeated_until(p, end)` — repeat `p` until `end` would match (terminator not consumed)
- `recursive(|me| …)` — self-referential grammars (depth-bounded)

Composition (methods on [`Parser`]):

- `.map(f)` / `.try_map(f)` — transform the output (fallibly, for validation)
- `.to(value)` — replace the output with a constant
- `.ignored()` — discard the output
- `.then(p)` / `.ignore_then(p)` / `.then_ignore(p)` — sequence, keeping both / right / left
- `.or(p)` — ordered alternation
- `.optional()` — `Option` of the output
- `.repeated()` / `.repeated_at_least(min)` / `.repeated_in(min, max)` — `Vec` of outputs
- `.labelled(msg)` — override the failure message
- `.with_byte_offset()` / `.with_line()` — pair the output with the trailing byte offset / `(line, line-start offset)` (NimbleParsec's `byte_offset` / `line`)
- `.debug(label)` — trace the parser to stderr, passing the output through
- `.post_traverse(f)` / `.pre_traverse(f)` — fallible transform with the end / start `Cursor` in hand (NimbleParsec's `post_traverse` / `pre_traverse`); thread user _context_ by capturing `Cell`/`RefCell` state in `f` for context-dependent parsing

Run with `.parse(text)` (requires all input consumed), `.parse_partial(text)`
(returns the remainder), or the `*_with_max_depth` variants.

`generate(&parser, seed)` synthesizes a random input the parser accepts (seeded,
reproducible) — available for non-recursive grammars; see [`Generate`].

[`Generate`]: src/typed.rs

## Errors

A `ParseFailure` carries the human-readable `reason`, a structured
`expected: Vec<String>` (the descriptions the parser was looking for, unioned
across `choice`/`or` alternatives; empty for negative assertions, `try_map`
rejections, or the recursion cap), and a `cursor` with line and byte offset.

## Recursion safety

`recursive` grammars recurse on the native call stack. To keep deeply nested
untrusted input from overflowing the stack (an uncatchable abort), each parse is
bounded by [`DEFAULT_MAX_RECURSION_DEPTH`] (256); exceeding it returns a
`ParseFailure` rather than crashing. Tune per parse with
`Parser::parse_with_max_depth(text, max_depth)` — lower to harden against hostile
input, higher for legitimately deep grammars. The default suits release builds on
a 2 MiB stack; debug builds have larger frames, so lower the cap if you run
untrusted input through a debug build.

[`DEFAULT_MAX_RECURSION_DEPTH`]: src/lib.rs
[`Parser`]: src/typed.rs

## Design

The typed surface is the result of the redesign in
[`docs/rfcs/0001-typed-parser.md`](docs/rfcs/0001-typed-parser.md), which replaced
an earlier runtime-interpreted `Value`-based port (and its codegen macro) — the
generic combinators are monomorphized by the compiler, so no interpreter or
codegen layer is needed.

## Roadmap

- **Generic input.** Lift the `&str`-only restriction to bytes / custom streams.
- **Fuzz corpus.** A persisted `cargo-fuzz` target alongside the property tests.
- **Benchmarks.** A Criterion suite for the typed combinators.

## Run tests

```bash
cd rust
cargo test
```
