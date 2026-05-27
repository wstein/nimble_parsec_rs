<!-- SPDX-License-Identifier: Apache-2.0 -->

# RFC 0001 — Typed `Parser<Output>` surface

_Status: draft (2026-05-27). Owner: crate maintainers. Tracks the §2.2 "dynamic
`Value` enum" finding from the migration review and the 1.0 roadmap item._

## Summary

Replace the dynamic, runtime-tagged `Value` term list with combinators that are
**generic over their output type**, so `utf8_char` yields a `char`, `repeat(p)`
yields a `Vec<O>`, `a.then(b)` yields `(A, B)`, and the whole grammar composes
and type-checks at compile time with no runtime `match` on emitted values. This
is the crate's planned `1.0` and the single highest-leverage change for making
it idiomatic Rust.

## Motivation

Today every combinator emits `Vec<Value>` where `Value` is
`Int | Str | List | Tagged | KeyValue`. Consumers pattern-match and discard the
variants that don't apply (see `codepoints_to_string` in the lexer example). This
is faithful to NimbleParsec's untyped term list but un-idiomatic for Rust and
carries three costs:

1. **No compile-time output typing.** A grammar that emits the wrong shape fails
   at runtime, not at `cargo build`.
2. **Runtime tagging.** Each emitted token is a tagged union; `utf8_char` boxes a
   codepoint as `Value::Int`, decoded again by the consumer.
3. **Ecosystem fit.** nom (`IResult<I, O>`), winnow, and chumsky
   (`Parser<I, O, E>`) are all generic over the output precisely to avoid this.
   The `Value` enum is what a Rust reviewer judges the crate by.

The small-integer fast path and the shared accumulator have softened the
allocation cost, but the _typing_ cost is structural and only a generic surface
removes it.

## Goals / non-goals

**Goals.** Generic output type; compile-time composition; keep the combinator
**names** and semantics (this stays a NimbleParsec port); reuse the structured
[`ParseFailure`](../../src/lib.rs) (its `expected` set fits unchanged); keep
`&str` input and zero-copy slices where possible.

**Non-goals.** Generic _input_ (stay `&str`-only for now; bytes/streaming is a
later RFC). Preserving the `Value` API — per the repo's no-backward-compat
stance, `1.0` replaces it rather than layering over it.

## Proposed design

A trait-based core, mirroring winnow/chumsky, with combinators returning opaque
`impl Parser` types:

```rust
pub trait Parser<'i> {
    type Output;
    /// Parse from the front of `input`, advancing it past what was consumed.
    fn parse_next(&self, input: &mut &'i str) -> Result<Self::Output, ParseFailure<'i>>;
}

// Leaves
pub fn string(lit: &'static str) -> impl Parser<'_, Output = &'static str>;
pub fn utf8_char() -> impl for<'i> Parser<'i, Output = char>;

// Composition (provided methods on `Parser`)
fn map<U>(self, f: impl Fn(Self::Output) -> U) -> impl Parser<'i, Output = U>;
fn then<P>(self, next: P) -> impl Parser<'i, Output = (Self::Output, P::Output)>;
fn or(self, alt: impl Parser<'i, Output = Self::Output>) -> impl Parser<'i, Output = Self::Output>;
fn repeated(self) -> impl Parser<'i, Output = Vec<Self::Output>>;
fn optional(self) -> impl Parser<'i, Output = Option<Self::Output>>;
```

Worked example — today's codepoint round-trip disappears:

```rust
// before: Vec<Value> of Value::Int, reassembled by the consumer
// after:
let word = utf8_char().filter(|c| c.is_alphanumeric()).repeated();
let s: String = word.map(|cs| cs.into_iter().collect()).parse("abc")?; // "abc"
```

### Key decisions

- **Trait + `impl Parser` over boxed closures.** Zero-cost composition and good
  inference; only `recursive` needs a boxed indirection (a `BoxedParser<'i, O>`
  newtype). This matches winnow's ergonomics.
- **Reuse `ParseFailure`.** The structured `expected` set already landed and maps
  directly onto typed combinators (`choice` still unions). No new error type.
- **Recursion cap stays.** The thread-local depth budget is orthogonal to output
  typing and carries over unchanged.
- **Codegen.** The `compile_parser!` / `defparsec!` macros lower to the same typed
  combinators; the generated code becomes _simpler_ (no `Value` push/pop), and
  `__private` helpers are re-typed. This is the largest single migration cost.

### Alternatives considered

- **Concrete `Parser<O>` struct boxing a closure.** Simpler types, uniform
  storage, but a heap allocation + dynamic dispatch per combinator. Rejected on
  performance grounds (the whole point is to remove overhead).
- **Keep `Value`, add a typed _view_ layer.** A back-compat layer the repo
  explicitly forbids, and it would not remove the runtime tagging.

## Migration plan (phased, each phase independently landable)

1. **This RFC.** (done)
2. **Typed core (`typed` module).** ✅ Done. The `Parser<'i>` trait with
   `map`/`then`/`ignore_then`/`then_ignore`/`or`/`optional`/`repeated`/`labelled`,
   leaves (`literal`/`any`/`satisfy`/`take_while`/`digits`/`eof`), and a boxed
   `recursive` that reuses the structured `ParseFailure` and the recursion cap.
   Tested in [`rust/tests/typed.rs`](../../tests/typed.rs); lives beside the
   existing API.
3. **Combinator parity.** Port the remaining ~25 combinators to the typed surface,
   reaching feature parity with the `Value` API; port the differential fixtures.
4. **Codegen.** Re-target the proc-macro to emit typed combinators; re-type
   `__private`.
5. **Consumer migration.** Move Stem's `np_lexer` / `np_expr` to the typed API
   behind the existing differential gate (`compile_diff` / `verify` / `fuzz`).
6. **Remove `Value`-based API → `1.0`.** Delete the dynamic surface once no
   consumer depends on it. (Transitional coexistence during phases 2–5 is a
   migration period, not a shipped compatibility layer.)

## Open questions

- Should `then` flatten tuples (`(A, B, C)`) via a chaining trait, or nest
  (`((A, B), C)`) and lean on `map`? (Winnow flattens; chumsky nests.)
- `tag`/`unwrap_and_tag` shapes for typed output — likely subsumed by `map`/named
  structs, possibly dropped.
- Whether to expose `recursive` as `BoxedParser` or a declared-reference handle
  like today's `ParserRef`.

## Risks

Largest is the macro re-targeting (phase 4) and keeping the differential green
through the consumer migration (phase 5). Both are de-risked by the existing
gate; the typed core (phase 2) carries near-zero risk since it is additive.
