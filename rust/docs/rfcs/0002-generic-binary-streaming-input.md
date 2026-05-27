<!-- SPDX-License-Identifier: Apache-2.0 -->

# RFC 0002 — Generic, binary, and streaming input

_Status: draft (2026-05-27). Owner: crate maintainers. Builds on
[RFC 0001](0001-typed-parser.md) (the typed `Parser<Output>` surface) and tracks
the "binary / non-UTF-8 input is a future milestone" note in `typed.rs` and
`tests/binary_workarounds.rs`._

## Summary

Make the parser **generic over its input** so it natively handles, through a
single combinator set:

1. **UTF-8 text** (`&str`) — today's behavior, char-oriented;
2. **Arbitrary bytes** (`&[u8]`) — non-UTF-8 binary formats, byte-oriented;
3. **Token slices** (`&[T]`) — a lexer feeding a second parser stage (the
   abstraction should permit it; impl can land later);
4. **Streaming / partial input** — parse a buffer that may be a _prefix_ of the
   full input, signalling "need more bytes" rather than a hard failure, so large
   inputs can be processed incrementally.

The mechanism is a curated `Stream` trait (winnow-style: associated `Token` and
`Slice`, plus a `StreamIsPartial` const that lets one combinator body serve both
complete and streaming parsing) layered under the existing typed
`Parser<Output>` / named-zero-cost-combinator design. We keep the structured
`ParseFailure` errors and the always-on cursor; we add an `Incomplete(Needed)`
error path for streaming.

## Motivation

The Elixir original is **binary-first**: NimbleParsec's compiler emits
`<<byte, rest::binary>>` matching clauses over arbitrary Elixir binaries
(`lib/nimble_parsec/compiler.ex:502`, `:803`); `utf8_char`, `ascii_char`, and
`bytes` are layers on top of a raw byte stream. The Rust port inverted that — it
is **hardwired to `&str`** and can only express UTF-8 text grammars. `bytes(n)`
even rejects counts that don't land on a UTF-8 char boundary
(`rust/src/typed.rs:1349`). `tests/binary_workarounds.rs` documents the escape
hatches (transcode to UTF-8, carry binary as hex/base64 text, fixed-width
`bytes`) and exactly where they run out.

This blocks every non-text use case (image/network/file-format parsing,
length-prefixed records, lexer→parser pipelines) and diverges from the Elixir
semantics the crate aims for parity with.

Constraints (chosen for this RFC): the end state is a **single generic `Stream`
trait** (not a duplicated byte core); the crate is pre-1.0 so we are **free to
break** the current `&str` API; **streaming is in scope**, putting the Rust port
ahead of NimbleParsec (which is complete-input only).

## Current architecture & `&str` coupling points

The coupling is **localized**. Combinators that only _thread_ the input (`Map`,
`Then`, `Or`, `Repeated`, `Opt`, `Delimited`, `SeparatedBy`, `Choice`,
`Recursive`, …) never touch `&str` — they call `parse_next` and snapshot/restore
the input. The `&str` assumptions live in a small set of places:

| # | Location | Coupling |
|---|----------|----------|
| C1 | `Input<'i> { rest: &'i str, cursor }` (`typed.rs:34`) | `rest` is `&str`; `bump` slices by `consumed.len()` |
| C2 | `Cursor` + `advance` (`lib.rs:133`) | scans bytes for `\n`; works on bytes already, but is `&str`-typed |
| C3 | `ParseFailure<'a> { rest: &'a str, … }` (`lib.rs:54`) | error carries a `&str` remainder; `Display` prints "byte offset" |
| C4 | `Literal` (`typed.rs:788`) | `starts_with(&str)`, slices, yields `&'i str` |
| C5 | `AnyChar` / `Satisfy` (`typed.rs:814`, `:843`) | `rest.chars().next()`, `len_utf8`, yields `char` |
| C6 | `TakeWhile` (`typed.rs:868`) | `char_indices`, `Fn(char)->bool`, yields `&'i str` |
| C7 | `Bytes` (`typed.rs:1338`) | byte count **rejected** off char boundary (`is_char_boundary`) |
| C8 | `Integer` / `digits` (`typed.rs:1329`, `:907`) | ASCII-digit scan over chars |
| C9 | `Eventually` (`typed.rs:1389`) | skips one `char` at a time |
| C10 | `Eof` (`typed.rs:917`) | `is_empty` — already generic in spirit |
| C11 | `Generate` trait (`typed.rs:1565`) | round-trip generator emits `String`, not bytes |
| C12 | `Debug` preview (`typed.rs:725`) | `rest.chars().take(24)` for the trace line |

Everything else is reusable as-is. That is what makes the recommended design
(below) a refactor rather than a rewrite.

## Reference designs: nom vs winnow

**nom** (byte-first heritage; `&[u8]` canonical, `&str` supported):
- Generic input via a _galaxy of traits_ — historically `InputTake`,
  `InputIter`, `InputLength`, `Slice<Range>`, `Compare`, `FindSubstring`,
  `FindToken`, `Offset`, `AsBytes`, `AsChar`… (nom 8 consolidated these into one
  `Input` trait + an `OutputMode`). Bounds are verbose but very general.
- **Streaming is a separate module**: `nom::number::streaming` vs `::complete`,
  `nom::bytes::streaming` vs `::complete` — _every combinator is written twice_.
  Streaming variants return `Err::Incomplete(Needed)`. This duplication is nom's
  most-criticized design choice.
- Binary numbers: `be_u16`, `le_u32`, `be_f64`, `u32(Endianness)`, etc.

**winnow** (nom fork, ergonomics-first; the model to emulate):
- **One `Stream` trait** with associated `Token` (`u8` for bytes, `char` for str)
  and `Slice` types, plus `checkpoint()`/`reset()` for backtracking, `next_token`,
  `offset_for`/`offset_at`. Implemented for `&str`, `&[u8]`, `&Bytes`, `&[T]`.
- **Streaming is a wrapper, not a duplicate module**: `Partial<I>` wraps any
  stream; a `StreamIsPartial` associated const lets one combinator body serve both
  complete and partial parsing. At end-of-buffer, partial mode yields
  `ErrMode::Incomplete(Needed)`; complete mode yields a normal error. *This is the
  key idea we adopt — it avoids nom's double-write problem.*
- **Location is a wrapper**: `LocatingSlice<I>` adds offset tracking only when you
  ask for it; the base stream carries no cursor.
- Backtracking vs committed error via `ErrMode::{Backtrack, Cut, Incomplete}`.
- Binary combinators in `winnow::binary`: `be_u16`, `le_u32`, `length_take`,
  `length_and_then`, …

**Verdict.** Adopt winnow's _shape_ — single `Stream` trait + `Partial` wrapper +
`StreamIsPartial` — because it delivers generic input **and** streaming through
one combinator set. Reject nom's duplicated-module streaming. Keep the crate's
differentiators: **named zero-cost combinator types** (better error messages and
docs than closures), **structured `ParseFailure.expected`**, and the
**always-on cursor**.

## Recommended design: a curated `Stream` trait

### The no-GAT trick

The hard part of "generic input with borrowed output" is that `literal` must
yield `&'i str` / `&'i [u8]` — output borrowing the input. Naively this needs
GATs. winnow's escape: **implement `Stream` on the borrowed type itself**
(`&'i [u8]`), so the lifetime is carried by the `Self` type and `Slice` is a
_plain_ associated type. We do the same — no GATs, stable Rust.

```rust
/// A position-trackable input source. Implemented on borrowed slices, so the
/// input lifetime is carried by `Self` and `Slice`/`Token` stay plain assoc types.
pub trait Stream: Copy {
    type Token: Copy;          // u8 for bytes, char for str, T for &[T]
    type Slice;                // &'i [u8] / &'i str / &'i [T]

    /// First token and its width in *base units* (bytes for u8/char, 1 for T).
    fn first(&self) -> Option<(Self::Token, usize)>;
    /// Split off the first `n` base units: (consumed slice, remainder).
    fn split_at(&self, n: usize) -> (Self::Slice, Self);
    fn len(&self) -> usize;            // remaining base units
    fn is_empty(&self) -> bool;
    /// Raw bytes view, for line tracking and error previews (Token=char ⇒ utf8).
    fn raw(&self) -> &[u8];
    /// Cheap backtrack point (an offset). `reset` restores it.
    fn checkpoint(&self) -> usize;
}

/// Whether this stream may be only a prefix of the full input.
pub trait StreamIsPartial { const PARTIAL: bool; }
impl<'i> StreamIsPartial for &'i [u8] { const PARTIAL: bool = false; }
impl<'i> StreamIsPartial for &'i str { const PARTIAL: bool = false; }
impl<S> StreamIsPartial for Partial<S> { const PARTIAL: bool = true; }

/// Matching a fixed literal against the front of the stream (winnow's `Compare`).
pub trait Compare<T> { fn starts_with(&self, t: T) -> Option<usize>; } // → consumed len
```

`impl Stream for &str` has `Token = char`, `Slice = &str`, `first` via
`chars().next()` + `len_utf8`. `impl Stream for &[u8]` has `Token = u8`,
`Slice = &[u8]`, `first` returns `(b, 1)`. A future `impl Stream for &[T]` gives
token-slice parsing for a lexer→parser pipeline.

### Generalized `Parser` and `Input`

Keep the always-on cursor (it backs the structured errors and the
`line`/`byte_offset` combinators), but make it generic over the stream:

```rust
pub struct Input<S: Stream> { stream: S, cursor: Cursor }

pub trait Parser<S: Stream> {
    type Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Self::Output>;
    fn parse(&self, src: S) -> Result<Self::Output, ParseFailure<S>> where Self: Sized { … }
    // map/then/or/repeated/… unchanged in body — they only thread `Input<S>`.
}
```

The trait gains a type parameter `S` in place of the lifetime `'i`; the
combinator _methods_ (`map`, `then`, `or`, …) are unchanged because they never
name `&str`. Leaf parsers move from `impl Parser<'i> for X` to a **blanket impl
over `S`**:

```rust
// any(): yields the stream's token type — char for str, u8 for bytes.
impl<S: Stream> Parser<S> for AnyToken {
    type Output = S::Token;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Token> {
        match input.stream.first() {
            Some((tok, w)) => { input.bump(w); Ok(tok) }
            None => incomplete_or_err::<S>("expected any token", input),
        }
    }
}
```

`incomplete_or_err` is the single point that consults `S::PARTIAL`: at
end-of-buffer it returns `Incomplete(Needed)` for a partial stream and a hard
expectation error for a complete one. **This is how one combinator body serves
both complete and streaming parsing** — the nom double-write is avoided.

### Error model: add `Incomplete`

`ParseFailure` becomes generic over the slice and gains a partial-input variant:

```rust
pub enum ParseError<S: Stream> {
    Failure(ParseFailure<S>),       // today's reason/expected/rest/cursor
    Incomplete(Needed),             // streaming: more input needed
}
pub enum Needed { Unknown, Size(NonZeroUsize) }
```

`ParseFailure.rest` becomes `S::Slice` (a `&[u8]` for byte streams). `Display`
keeps "byte offset"; for `Token=char` it can still preview text, for `Token=u8`
it hex-previews via `raw()`. Complete-input parses can `?`-flatten `Incomplete`
into an "unexpected end of input" failure so the simple API stays simple.

### Streaming model (`Partial<S>`)

```rust
pub struct Partial<S: Stream>(pub S);   // "this buffer may be a prefix"
impl<S: Stream> Stream for Partial<S> { /* delegates, PARTIAL = true */ }
```

Caller loop (winnow-style sliding window):
1. Parse `Partial(&buf)` from a saved `checkpoint`.
2. On `Ok` → consume, advance the window.
3. On `Incomplete(Needed)` → append more bytes to `buf`, retry from the checkpoint.
4. On `Failure` → real error.

**Honest limitation (documented):** `Partial<&[u8]>` still _borrows_ the whole
current buffer — it does not by itself free consumed bytes. True bounded-memory
streaming over an unbounded source requires the caller to maintain a sliding
window / ring buffer and drop already-consumed prefixes, or a future owned-input
adapter. winnow and nom have the exact same constraint; this is inherent to
zero-copy borrowed parsing, not a shortcoming we introduce. The design _enables_
incremental processing (you never need all bytes resident at once); it does not
make a borrowed slice forget its start. See risk R3.

### Cursor / location

- **A1 (recommended): keep cursor always-on inside `Input<S>`.** Minimal churn —
  `advance` already works on bytes; line tracking stays meaningful for text and is
  simply "0 newlines seen" for binary. `byte_offset`/`line` combinators keep
  working unchanged.
- **A2 (winnow-pure): location as a `LocatingSlice<S>` wrapper, base stream has no
  cursor.** More orthogonal, lets you opt out of line tracking on pure-binary hot
  paths, but churns every error site and the existing `line`/`byte_offset`
  combinators for a marginal win.

Recommend **A1**: the crate's identity includes always-structured errors with a
cursor; keep that guarantee and avoid a second axis of generics.

## Combinator inventory

### Modified (generalized over `S: Stream`)

| Combinator | Change |
|---|---|
| `any()` | yields `S::Token` (was always `char`) |
| `satisfy(pred)` / `one_of` / `none_of` | `pred: Fn(S::Token)->bool` |
| `take_while` / `take_while1` | `Fn(S::Token)->bool`, yields `S::Slice` |
| `literal(lit)` | via `Compare`; `&str` lit on str streams, `&[u8]` lit on byte streams; yields `S::Slice` |
| `bytes(n)` | retained as a thin alias of the new `take(n)` (below); on `&[u8]` it gains validity for any `n` — the char-boundary rejection at `typed.rs:1349` disappears |
| `eof()` | unchanged (already generic) |
| `eventually` | skips one _token_ at a time |
| every threading combinator | only the `'i`→`S` signature change; bodies unchanged |

### New (binary + streaming surface)

| Combinator | Purpose |
|---|---|
| `byte(b)` / `one_of_bytes` / `byte_range` | byte-level analogues of `satisfy`/`one_of` (parity with Elixir `ascii_char` ranges) |
| `utf8_char(ranges)` | decode one UTF-8 codepoint from a **byte** stream (Elixir parity) — the bridge that lets text grammars run on `&[u8]` |
| `be_u16/u32/u64`, `le_u16/u32/u64`, signed, `be_f32/f64`, `le_*` | fixed-width numeric parsers (nom/winnow `binary` parity) |
| `take(n)` | n base units — the general primitive that subsumes `bytes`, which stays as an alias (`pub fn bytes(n) -> Take { take(n) }`) |
| `length_take` / `length_value` | dynamic length prefix (today only expressible via `flat_map` — keep `flat_map`, add the idiomatic combinator) |
| `rest()` | yield all remaining input as `S::Slice` |
| `Partial<S>` + `Needed` | the streaming wrapper + error variant |

### `Generate` (round-trip)

Split the output: `generate_into(&mut Gen, &mut Vec<u8>)` becomes the primitive;
a `generate_str` convenience wraps it for `Token=char` parsers. Byte parsers emit
raw bytes; the existing `String`-based generators become thin adapters.

## Phasing

Each phase compiles and keeps tests green; later phases are additive once the
trait lands.

- **Phase 1 — Introduce `Stream` over `&str` only.** Define `Stream`, `Input<S>`,
  generalize `Parser` and all leaf parsers to the blanket impl, with `impl Stream
  for &str` as the sole impl. Behavior identical; this is the big mechanical
  refactor (C1–C12). Migrate tests/examples for the signature change.
- **Phase 2 — `impl Stream for &[u8]` + byte leaves.** `byte`, `take(n)`,
  `be_*`/`le_*`, `rest`. Add `take(n)` as the general primitive and keep
  `bytes(n)` as a thin alias of it; the char-boundary check is dropped on `&[u8]`.
- **Phase 3 — `utf8_char` + `ascii_char` ranges.** Text-on-bytes bridge; Elixir
  parity for `utf8_char`/`ascii_char`.
- **Phase 4 — Streaming.** `Partial<S>`, `StreamIsPartial`, `Needed`,
  `Incomplete`; route every token-consuming leaf through `incomplete_or_err`.
- **Phase 5 — `length_take`/`length_value`, `&[T]` token-slice impl, `Generate`
  bytes split, docs / parity matrix / CHANGELOG.**

## Migrating existing `&str` parsers

Although this is a breaking change, most text grammars need only mechanical
edits. The reason: `&str` implements `Stream` with `Token = char` and
`Slice = &str`, so every combinator keeps the _same output type_ it had before
(`any()` → `char`, `digits()` → `&str`, `literal("x")` → `&str`). The break is in
the _type signatures_ of parsers you name explicitly, not in their behavior.

### TL;DR by how you use the crate

- **You only compose provided combinators and call `.parse("…")`** → near-zero
  changes. The `&str` argument pins `S = &str`, inference fills the rest. Nothing
  to rename — `bytes(n)` keeps working as an alias of the new `take(n)`.
- **You wrote functions that _return_ parsers with a named lifetime** → swap the
  lifetime bound for a stream bound (or pin to `&str`). See §"Functions that
  return parsers".
- **You hand-implemented the `Parser` trait** → this is the real work; the impl
  header changes. See §"Custom `Parser` impls".
- **You implemented `Generate`** → signature moves from `String` to `Vec<u8>`.

### Mechanical renames / signature swaps

| Before (`&str`-only) | After (generic) | Note |
|---|---|---|
| `bytes(n)` | `bytes(n)` _or_ `take(n)` | No change required — `bytes` is now an alias of `take`. Same behavior on `&str` (counts bytes, still errors off a char boundary); `take` reads better and also fits `&[u8]`, where any `n` is valid. |
| `fn p<'i>() -> impl Parser<'i>` | `fn p<S: Stream>() -> impl Parser<S>` | Or pin: `fn p<'i>() -> impl Parser<&'i str>` to migrate incrementally. |
| `impl<'i> Parser<'i> for X` | `impl<S: Stream> Parser<S> for X` | Or pin: `impl<'i> Parser<&'i str> for X`. See below. |
| `ParseFailure<'a>` in your signatures | `ParseFailure<&'a str>` | The struct gained a stream parameter; for text it is `&str`, so fields (`reason`, `expected`, `rest`, `cursor`) are unchanged. |
| matching the parse `Result` | unchanged for `.parse` | The top-level `.parse(src)` flattens `Incomplete` into a normal `ParseFailure`, so complete-input error handling is untouched. `Incomplete` only surfaces under `Partial<S>`. |

Closures keep their annotations as-is: `.map(|ds: &str| …)` and
`.satisfy(|c: char| …)` still type-check on `&str` streams because the token and
slice types are unchanged.

### Functions that return parsers

The common pattern of factoring a sub-grammar into a function gains a stream
parameter:

```rust
// Before
fn quoted<'i>() -> impl Parser<'i, Output = &'i str> { … }

// After — generic (works on &str and &[u8])
fn quoted<S: Stream>() -> impl Parser<S, Output = S::Slice> { … }

// After — pinned to text, smallest diff if you don't need bytes
fn quoted<'i>() -> impl Parser<&'i str, Output = &'i str> { … }
```

Pinning to `&str` is the recommended incremental path: a parser written
`impl Parser<&'i str>` compiles unchanged in behavior and can be generalized later
without touching its call sites.

### Custom `Parser` impls

A hand-written leaf is where you do actual work, because it likely calls
`&str`-specific methods (`chars()`, `starts_with`, slicing). Two paths:

```rust
// Path A — pin to &str (no behavior change, smallest edit):
impl<'i> Parser<&'i str> for MyLeaf {
    type Output = &'i str;
    fn parse_next(&self, input: &mut Input<&'i str>) -> PResult<&'i str, Self::Output> { … }
}

// Path B — go generic: replace &str method calls with Stream methods
// (`first`, `split_at`, `raw`, `checkpoint`) and route end-of-input through
// `incomplete_or_err` so the leaf also works under `Partial<S>`.
impl<S: Stream> Parser<S> for MyLeaf {
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Self::Output> { … }
}
```

Most users have **no** custom leaves — they compose the built-ins — so this
section usually does not apply.

### `Generate` impls

`generate_into` takes `&mut Vec<u8>` instead of `&mut String`. Text generators
push UTF-8 bytes (`out.extend_from_slice(s.as_bytes())`); call the `generate_str`
convenience wrapper when you want a `String` back.

### Worked example

```rust
// ── Before (today's &str-only API) ──────────────────────────────────────
use nimble_parsec_rs::typed::{digits, literal, Parser};

fn paren_number<'i>() -> impl Parser<'i, Output = u32> {
    literal("(")
        .ignore_then(digits())
        .then_ignore(literal(")"))
        .map(|d: &str| d.parse().unwrap())
}
assert_eq!(paren_number().parse("(42)").unwrap(), 42);

// ── After (generic; this grammar is text-only, so pin to &str) ──────────
use nimble_parsec_rs::typed::{digits, literal, Parser};

fn paren_number<'i>() -> impl Parser<&'i str, Output = u32> {
    literal("(")                       // unchanged
        .ignore_then(digits())         // unchanged → &str
        .then_ignore(literal(")"))     // unchanged
        .map(|d: &str| d.parse().unwrap())
}
assert_eq!(paren_number().parse("(42)").unwrap(), 42);  // call site unchanged
```

The only edit is the return type's lifetime bound `Parser<'i, …>` →
`Parser<&'i str, …>`. The body and the call site are identical. Migration is
therefore a find-and-replace over parser-returning function signatures — no
combinator renames, since `bytes` stays as an alias of `take`; deeper changes are
needed only if you maintain custom `Parser`/`Generate` impls or want a grammar to
run on bytes.

## Alternatives considered, rated

**Input model.**

- **Option A — curated generic `Stream` trait (RECOMMENDED). ★★★★★**
  One combinator set covers str + bytes + token slices + streaming. Matches
  winnow's proven shape; no GATs; preserves the named-type / structured-error
  identity. Cost: a real refactor of C1–C12 and a one-time test/example migration.
  Chosen.
- **Option B — parallel `&[u8]` byte core alongside `&str`. ★★☆☆☆**
  Fastest to _start_, zero churn to the str API. But it bakes in nom's
  double-write tax forever (two combinator sets, two error types, drift), and
  streaming would have to be written a _third_ time. Rejected — contradicts the
  "single generic" goal.
- **Option C — binary-first core (`&[u8]` canonical, `&str` a layer). ★★★☆☆**
  Truest to Elixir and conceptually clean (text = `utf8_char` over bytes). But it
  makes the _common_ case (text grammars) pay a wrapping/decoding tax and lose
  direct `&str` outputs, and it's a bigger break than A for no extra capability
  once A already supports `&[u8]`. Rejected as the _primary_ model, but its
  `utf8_char`-over-bytes idea is adopted inside A.

**Streaming sub-design.**

- **S1 — winnow-style `Partial<S>` + `StreamIsPartial` const (RECOMMENDED). ★★★★★**
  One combinator body serves complete and partial via a compile-time const; the
  caller manages refill from a checkpoint. Idiomatic, no runtime cost in the
  complete case.
- **S2 — nom-style duplicated `complete`/`streaming` modules. ★★☆☆☆**
  Every leaf written twice. Rejected.
- **S3 — true resumable/suspending parser (coroutine state machine). ★★☆☆☆**
  Would let the parser fully release consumed input, but is a different machine
  entirely (no borrowed slices; heap-resident continuation), incompatible with the
  zero-cost named-combinator design. Rejected; revisit only if bounded-memory over
  truly unbounded streams becomes a hard requirement (see R3).

**Location sub-design:** A1 (always-on cursor in `Input<S>`) ★★★★☆ over A2
(`LocatingSlice` wrapper) ★★★☆☆.

## Risks & open questions

- **R1 — `literal` ergonomics across streams.** `literal("ab")` should work on
  `&str` and `literal(b"ab")` / `literal(&[0,1])` on `&[u8]`. Solved via
  `Compare<T>` with impls for `&str`-vs-str and `&[u8]`-vs-bytes; needs care so a
  `&str` literal on a byte stream is a _type error_, not a silent UTF-8
  reinterpretation.
- **R2 — `Token=char` vs `u8` changes output types.** `any()`/`satisfy` now yield
  `S::Token`, so grammars that used `char` must annotate or run on a `&str`
  stream. Acceptable under "free to break"; document in the migration note.
- **R3 — borrowed streaming doesn't free memory by itself.** Decide whether
  Phase 4 also ships a sliding-window helper / owned-buffer adapter, or documents
  the caller's responsibility (winnow's stance). Recommend: document now, helper
  later.
- **R4 — recursion cap & checkpoints** interact with `Partial` retries; ensure
  the `RECURSION_BUDGET` resets per attempt (it already restores per `parse*`
  call).
- **R5 — `Generate`/proptest** round-trip for byte parsers needs a `Vec<u8>`
  generator path; the differential Elixir/Rust fuzzing harness must feed bytes too.

## Verification

- **Unit/parity:** port NimbleParsec binary tests — `bytes`, `utf8_char`,
  `ascii_char` ranges — and confirm `take(n)` on `&[u8]` accepts counts that the
  old `bytes(n)` rejected mid-codepoint. Re-run the existing 100+ `&str` tests
  after migration; all must pass.
- **Binary formats:** add a real fixed-layout test (e.g. a PNG/IHDR-style header
  or a TLV record) using `be_u32`/`take`/`length_take` end-to-end.
- **Streaming:** feed a byte buffer one chunk at a time through `Partial`; assert
  `Incomplete(Needed)` at each short read and a correct final parse once complete —
  and that the result equals the one-shot parse of the full buffer.
- **Differential fuzz:** extend the Elixir↔Rust differential harness to byte
  inputs and `utf8_char` ranges.
- `cargo test`, `cargo clippy -- -D warnings`, `cargo llvm-cov` (hold ≥ the
  current ~86% line coverage), `cargo doc` (crate is `#![deny(missing_docs)]`).

## Affected files

- `rust/src/typed.rs` — `Input`, `Parser`, all leaf parsers, combinators,
  `Generate` (C1, C4–C12).
- `rust/src/lib.rs` — `Cursor`/`advance` (C2), `ParseFailure` / error model (C3),
  recursion cap.
- `rust/tests/*` — signature migration + new `binary_*` / `streaming_*` suites;
  `binary_workarounds.rs` shrinks as native support lands.
- `PARITY_MATRIX.md`, `README.md`, `CHANGELOG.md` — record the new `Stream` model
  and the `&str`-only → generic change.
- Reference: `lib/nimble_parsec.ex` (`bytes`/`ascii_char`/`utf8_char` semantics),
  `lib/nimble_parsec/compiler.ex` (binary-matching model).
