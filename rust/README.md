# nimble_parsec_rs

A Rust port of [NimbleParsec](https://github.com/dashbitco/nimble_parsec) with an
idiomatic, **typed** combinator surface. Each combinator is generic over its
output, so grammars compose and type-check at compile time with no runtime
tagging — `literal` yields `&str`, `then` yields a tuple, `repeated` yields a
`Vec`, and `map` threads any output type through. Input is generic: `&str` for
text and `&[u8]` for binary data are both first-class; `Partial<S>` wraps either
for streaming (incomplete-input) parsing. Zero runtime dependencies.

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
> type rather than NimbleParsec's untyped term list. Input is generic: `&str`
> for text, `&[u8]` for binary data, `Partial<S>` for streaming.

## Combinators

Leaves (free functions):

- `literal(&str)` — match an exact string, yielding the slice
- `any()` — any one character
- `satisfy(label, pred)` — a character matching a predicate
- `one_of(set)` / `none_of(set)` — a character in / not in a set
- `take_while(pred)` / `take_while1(pred)` — a run of matching characters
- `digits()` — one or more ASCII digits (as `&str`)
- `integer()` — a run of digits parsed into an `i64`
- `take(n)` / `bytes(n)` — exactly `n` tokens (alias: `bytes` is `take` for `&[u8]`)
- `eof()` — end of input
- `empty()` — always succeeds, consuming nothing
- `eventually(p)` — skip input until `p` matches, then return its output
- `choice(alts)` — the first matching alternative; `alts` is an array `[p; N]` (same type) or a tuple `(a, b, …)` up to arity 8 (different types, one `Output`)
- `lookahead(p)` / `not(p)` — zero-width positive / negative assertions
- `delimited(open, content, close)` — `content` between two delimiters
- `separated_by(item, sep)` / `separated_by1(item, sep)` — a separated list (no trailing separator)
- `repeated_until(p, end)` — repeat `p` until `end` would match (terminator not consumed)
- `recursive(|me| …)` — self-referential grammars (depth-bounded)

Composition (methods on [`Parser`]):

- `.map(f)` / `.try_map(f)` — transform the output (fallibly, for validation)
- `.flat_map(f)` — use the output to choose the next parser (monadic bind), for context-sensitive grammars (length prefixes, layout)
- `.to(value)` — replace the output with a constant
- `.ignored()` — discard the output
- `.then(p)` / `.ignore_then(p)` / `.then_ignore(p)` — sequence, keeping both / right / left
- `.or(p)` — ordered alternation
- `.optional()` — `Option` of the output
- `.repeated()` / `.repeated_at_least(min)` / `.repeated_in(min, max)` — `Vec` of outputs
- `.fold(init, f)` — repeat, folding outputs into an accumulator (NimbleParsec's `reduce`, no intermediate `Vec`)
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

## Generic and binary input

All leaf parsers are generic over a `Stream` type parameter `S`:

```rust
// Text parsing — S = &str, Token = char
let p = literal::<&str, _>("GET ").ignore_then(take_while(|c: char| c != '\n'));

// Binary parsing — S = &[u8], Token = u8
use nimble_parsec_rs::{byte, be_u16, take};
let frame = byte(0x01_u8)
    .ignore_then(be_u16())
    .flat_map(|len| take(len as usize));
```

### `Stream` trait

The `Stream` trait (in `lib.rs`) describes an input sequence:

| Associated item | Meaning |
| --- | --- |
| `Token` | The element type — `char` for `&str`, `u8` for `&[u8]` |
| `Slice` | The type returned by multi-token parsers — `&str` or `&[u8]` |
| `PARTIAL: bool` | `false` for complete input; `true` for `Partial<S>` streaming |

Implementations ship for `&str`, `&[u8]`, and `Partial<S>` (wraps either for
streaming).

### `Compare<Pat>` trait

`Compare<Pat>` is how `literal` / `byte` perform type-safe pattern matching.
Implemented for:

- `&str` with a `&str` pattern
- `&[u8]` with a `&[u8]` pattern
- `&[u8]` with a single `u8`

### Binary leaf parsers

Available when `S = &[u8]` (or `Partial<&[u8]>`):

| Parser | Output | Description |
| --- | --- | --- |
| `byte(b)` | `u8` | Match one exact byte |
| `byte_range(lo, hi)` | `u8` | Match a byte in `lo..=hi` |
| `be_u8()` / `le_u8()` | `u8` | Single byte |
| `be_u16()` / `le_u16()` | `u16` | 2-byte big/little-endian |
| `be_u32()` / `le_u32()` | `u32` | 4-byte big/little-endian |
| `be_u64()` / `le_u64()` | `u64` | 8-byte big/little-endian |
| `utf8_char()` | `char` | Decode one UTF-8 scalar from `&[u8]` |
| `take(n)` | `&[u8]` | Exactly `n` bytes |
| `bytes(n)` | `&[u8]` | Alias for `take(n)` |
| `rest()` | `&[u8]` | All remaining bytes |

### Streaming with `Partial<S>`

Wrap a slice in `Partial` to signal that the input may be incomplete:

```rust
use nimble_parsec_rs::{Partial, be_u32};

let result = be_u32::<Partial<&[u8]>>().parse_partial(Partial(&[0x00, 0x00]));
// returns Err(Incomplete) — more bytes are needed
```

`Partial<S>` sets `S::PARTIAL = true`; parsers that see `Incomplete` propagate
it up rather than reporting a hard failure. Complete-input parsers (`&str`,
`&[u8]`) use `PARTIAL = false` and never produce `Incomplete`.

### `Recursive<'a, S, O>`

The `recursive` combinator now carries an explicit lifetime `'a` and stream
type `S`: `Recursive<'a, S, Output>`. Existing text-only uses just add `&str`
as the stream type annotation; the lifetime is almost always inferred.

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

## Porting from Elixir

The core uses idiomatic Rust names (`literal`, `.to`, `.repeated_in`, …). For a
closer mapping to NimbleParsec, `nimble_parsec_rs::nimble` re-exposes the surface
under NimbleParsec terminology as free functions:

```rust
use nimble_parsec_rs::nimble::*; // string, eos, concat, replace, duplicate, map, …

let pair = concat(string("("), concat(integer(), string(")")));
assert_eq!(pair.parse("(42)").unwrap(), ("(", (42, ")")));
```

`tag` / `unwrap_and_tag` / `reduce` / `wrap` are intentionally absent — in the
typed API those are `.map` into your own type (or `.fold`); see
[PARITY_MATRIX.md](../PARITY_MATRIX.md) for the full mapping.

## Design

The typed surface is the result of the redesign in
[`docs/rfcs/0001-typed-parser.md`](docs/rfcs/0001-typed-parser.md), which replaced
an earlier runtime-interpreted `Value`-based port (and its codegen macro) — the
generic combinators are monomorphized by the compiler, so no interpreter or
codegen layer is needed.

## Scope & limitations

This crate targets **UTF-8 text** (`&str`) and **binary data** (`&[u8]`). That
covers templates, config files, DSLs, source code, binary file formats, and wire
protocols. Combinator-level parity with NimbleParsec is complete. Remaining
structural gaps:

- **No bit-level parsing.** There is no `bits`/`bit_count` combinator. For
  bit-level fields, use [`nom`](https://crates.io/crates/nom) (which has a
  `bits` sub-module) or [`winnow`](https://crates.io/crates/winnow).
- **No custom stream types beyond `&str` / `&[u8]` / `Partial<S>`.** The
  `Stream` trait is public and can be implemented for custom slices, but there
  are no built-in adapters for `&[T]` where `T` is not `u8`.

## Roadmap

- **Fuzz corpus.** A persisted `cargo-fuzz` target alongside the property tests.
- **Benchmarks.** A Criterion suite for the typed combinators.
- **Bit-level parsing.** A `bits` sub-combinator for parsing individual bits or
  sub-byte fields from `&[u8]`.

## Run tests

```bash
cd rust
cargo test
```
