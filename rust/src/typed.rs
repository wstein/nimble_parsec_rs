//! Typed, generic parser combinators — RFC 0001 + RFC 0002.
//!
//! A [`Parser`] is generic over both its `Output` and its input **[`Stream`]**
//! (`&str`, `&[u8]`, `Partial<_>` for streaming, or a custom token sequence),
//! so grammars compose and type-check at compile time with no runtime `Value`
//! tagging.
//!
//! ## Text grammar (unchanged surface)
//!
//! ```
//! use nimble_parsec_rs::typed::{digits, literal, Parser};
//!
//! let inner = literal("(")
//!     .ignore_then(digits())
//!     .then_ignore(literal(")"))
//!     .map(|ds: &str| ds.parse::<u32>().unwrap());
//! assert_eq!(inner.parse("(42)").unwrap(), 42);
//! ```
//!
//! ## Binary grammar
//!
//! ```
//! use nimble_parsec_rs::typed::{be_u32, take, literal, Parser};
//!
//! // Parse a 4-byte big-endian magic number followed by a 2-byte payload.
//! let header = be_u32().then(take(2));
//! let (magic, payload) = header.parse(b"\xDE\xAD\xBE\xEF\x01\x02".as_ref()).unwrap();
//! assert_eq!(magic, 0xDEAD_BEEF);
//! assert_eq!(payload, &[0x01, 0x02]);
//! ```

use std::cell::{Cell, OnceCell};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::{Cursor, Needed, ParseError, ParseFailure, Stream, DEFAULT_MAX_RECURSION_DEPTH};

/// The result of a single parse step. `Ok(value)` on success; `Err` carries
/// either a [`ParseFailure`] (hard error) or [`crate::Needed`] (incomplete
/// input on a [`crate::Partial`] stream).
pub type PResult<S, O> = Result<O, ParseError<S>>;

// ── Input ─────────────────────────────────────────────────────────────────────

/// Parser input: the unconsumed stream and the current position. `Copy`
/// (because `S: Stream: Copy` and `Cursor: Copy`), so a combinator can
/// snapshot it before a fallible attempt and restore it on failure — the
/// zero-cost backtracking primitive used by alternation and repetition.
pub struct Input<S: Stream> {
    stream: S,
    cursor: Cursor,
}

// Manual Copy/Clone so we don't require S: Clone beyond what Stream provides.
impl<S: Stream> Copy for Input<S> {}
impl<S: Stream> Clone for Input<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Stream> core::fmt::Debug for Input<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Input")
            .field("preview", &self.stream.preview(32))
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl<S: Stream> Input<S> {
    /// Wraps `src`, positioned at its start.
    pub fn new(src: S) -> Self {
        Self {
            stream: src,
            cursor: Cursor::default(),
        }
    }

    /// The still-unconsumed stream.
    pub fn stream(&self) -> S {
        self.stream
    }

    /// The still-unconsumed stream (alias for [`stream`](Input::stream)).
    pub fn rest(&self) -> S {
        self.stream
    }

    /// The current position.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Advances `n` base units, scanning for newlines so line tracking stays
    /// accurate. Panics if `n > self.stream.len()`.
    fn bump(&mut self, n: usize) {
        self.cursor = self.stream.advance_cursor(self.cursor, n);
        let (_, rest) = self.stream.split_at(n);
        self.stream = rest;
    }
}

// ── incomplete_or_err ─────────────────────────────────────────────────────────

/// At end-of-buffer: returns `Incomplete` on partial streams, a hard
/// expectation failure on complete streams. This is the single point that lets
/// one combinator body serve both complete and streaming parsing (RFC 0002 §
/// "The no-GAT trick").
#[inline]
fn incomplete_or_err<S: Stream, O>(
    expectation: &str,
    rest: S::Slice,
    cursor: Cursor,
) -> PResult<S, O> {
    if S::PARTIAL {
        Err(ParseError::Incomplete(Needed::Unknown))
    } else {
        Err(ParseError::Failure(ParseFailure::expecting(
            expectation,
            rest,
            cursor,
        )))
    }
}

// ── Parser trait ──────────────────────────────────────────────────────────────

/// A parser that produces a typed `Output` from an input [`Stream`] `S`.
///
/// Implement [`parse_next`](Parser::parse_next); all composition methods are
/// provided. Leaf constructors ([`literal`], [`any`], [`satisfy`], …) and
/// combinators return named zero-cost structs so grammars are fully
/// monomorphized — no heap allocation, no dynamic dispatch in the hot path.
pub trait Parser<S: Stream> {
    /// The value this parser produces on success.
    type Output;

    /// Parses from the front of `input`, advancing it past what was consumed.
    /// On failure the input is left as the implementation chose; backtracking
    /// combinators snapshot and restore it themselves.
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Self::Output>;

    /// Runs the parser over the whole `src`, requiring all input to be
    /// consumed, with the default recursion cap.
    ///
    /// `Incomplete` (from a `Partial` stream) is converted to a
    /// [`ParseFailure`] so the simple error path is always available.
    fn parse(&self, src: S) -> Result<Self::Output, ParseFailure<S>>
    where
        Self: Sized,
    {
        self.parse_with_max_depth(src, DEFAULT_MAX_RECURSION_DEPTH)
    }

    /// Like [`parse`](Parser::parse), but caps recursion depth at `max_depth`.
    fn parse_with_max_depth(
        &self,
        src: S,
        max_depth: usize,
    ) -> Result<Self::Output, ParseFailure<S>>
    where
        Self: Sized,
    {
        match self.parse_partial_with_max_depth(src, max_depth) {
            Ok((output, remaining)) if remaining.is_empty() => Ok(output),
            Ok((_, remaining)) => {
                // Compute the cursor at the leftover position by fast-forwarding
                // through the consumed portion.
                let consumed = src.len() - remaining.len();
                let mut tmp = Input::new(src);
                tmp.bump(consumed);
                Err(ParseFailure::expecting(
                    "expected end of input",
                    remaining.as_slice(),
                    tmp.cursor,
                ))
            }
            Err(ParseError::Failure(f)) => Err(f),
            Err(ParseError::Incomplete(_)) => Err(ParseFailure::rejected(
                "incomplete input: more data is needed to complete this parse",
                src.as_slice(),
                Cursor::default(),
            )),
        }
    }

    /// Runs the parser, returning the output and the unconsumed remainder.
    /// Does **not** require all input to be consumed.
    fn parse_partial(&self, src: S) -> Result<(Self::Output, S), ParseError<S>>
    where
        Self: Sized,
    {
        self.parse_partial_with_max_depth(src, DEFAULT_MAX_RECURSION_DEPTH)
    }

    /// Like [`parse_partial`](Parser::parse_partial), but caps recursion
    /// depth at `max_depth`.
    fn parse_partial_with_max_depth(
        &self,
        src: S,
        max_depth: usize,
    ) -> Result<(Self::Output, S), ParseError<S>>
    where
        Self: Sized,
    {
        let prev = crate::RECURSION_BUDGET.with(|b| b.replace(max_depth));
        let mut input = Input::new(src);
        let result = self.parse_next(&mut input);
        crate::RECURSION_BUDGET.with(|b| b.set(prev));
        Ok((result?, input.stream))
    }

    /// Transforms the output with `f`.
    fn map<U, F>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> U,
    {
        Map { inner: self, f }
    }

    /// Discards the output (keeps only the fact that it matched).
    fn ignored(self) -> Ignored<Self>
    where
        Self: Sized,
    {
        Ignored { inner: self }
    }

    /// Sequences `self` then `next`, yielding both outputs as a tuple.
    fn then<P>(self, next: P) -> Then<Self, P>
    where
        Self: Sized,
        P: Parser<S>,
    {
        Then {
            first: self,
            second: next,
        }
    }

    /// Sequences `self` then `next`, keeping only `next`'s output.
    fn ignore_then<P>(self, next: P) -> IgnoreThen<Self, P>
    where
        Self: Sized,
        P: Parser<S>,
    {
        IgnoreThen {
            first: self,
            second: next,
        }
    }

    /// Sequences `self` then `next`, keeping only `self`'s output.
    fn then_ignore<P>(self, next: P) -> ThenIgnore<Self, P>
    where
        Self: Sized,
        P: Parser<S>,
    {
        ThenIgnore {
            first: self,
            second: next,
        }
    }

    /// Tries `self`; on failure (consuming nothing) tries `alt`. Both must
    /// produce the same output type.
    fn or<P>(self, alt: P) -> Or<Self, P>
    where
        Self: Sized,
        P: Parser<S, Output = Self::Output>,
    {
        Or { a: self, b: alt }
    }

    /// Makes `self` optional; yields `None` (consuming nothing) on failure.
    fn optional(self) -> Opt<Self>
    where
        Self: Sized,
    {
        Opt { inner: self }
    }

    /// Repeats `self` zero or more times, collecting the outputs.
    fn repeated(self) -> Repeated<Self>
    where
        Self: Sized,
    {
        Repeated {
            inner: self,
            min: 0,
            max: None,
        }
    }

    /// Repeats `self` at least `min` times.
    fn repeated_at_least(self, min: usize) -> Repeated<Self>
    where
        Self: Sized,
    {
        Repeated {
            inner: self,
            min,
            max: None,
        }
    }

    /// Repeats `self` between `min` and `max` times inclusive.
    fn repeated_in(self, min: usize, max: usize) -> Repeated<Self>
    where
        Self: Sized,
    {
        Repeated {
            inner: self,
            min,
            max: Some(max),
        }
    }

    /// Replaces the output with a clone of `value`.
    fn to<V: Clone>(self, value: V) -> To<Self, V>
    where
        Self: Sized,
    {
        To { inner: self, value }
    }

    /// Repeats `self` zero or more times, folding outputs into an accumulator
    /// seeded by `init` (NimbleParsec's `reduce`).
    fn fold<A, I, F>(self, init: I, f: F) -> Fold<Self, I, F>
    where
        Self: Sized,
        I: Fn() -> A,
        F: Fn(A, Self::Output) -> A,
    {
        Fold {
            inner: self,
            init,
            f,
        }
    }

    /// Transforms with a fallible `f`; returning `Err(message)` fails the parse.
    fn try_map<U, F>(self, f: F) -> TryMap<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> Result<U, String>,
    {
        TryMap { inner: self, f }
    }

    /// Monadic bind: uses this output to choose the next parser, enabling
    /// context-sensitive grammars (e.g. a length prefix). Not generatable.
    fn flat_map<U, F>(self, f: F) -> FlatMap<Self, F>
    where
        Self: Sized,
        U: Parser<S>,
        F: Fn(Self::Output) -> U,
    {
        FlatMap { inner: self, f }
    }

    /// Overrides the failure message and structured expectation with `label`.
    fn labelled(self, label: &'static str) -> Labelled<Self>
    where
        Self: Sized,
    {
        Labelled { inner: self, label }
    }

    /// Pairs the output with the byte offset reached after the match.
    fn with_byte_offset(self) -> WithByteOffset<Self>
    where
        Self: Sized,
    {
        WithByteOffset { inner: self }
    }

    /// Pairs the output with the position reached after the match
    /// `(1-based line, byte offset of the start of that line)`.
    fn with_line(self) -> WithLine<Self>
    where
        Self: Sized,
    {
        WithLine { inner: self }
    }

    /// Traces this parser to stderr (position on entry, outcome on exit),
    /// passing the output through unchanged.
    fn debug(self, label: &'static str) -> Debug<Self>
    where
        Self: Sized,
    {
        Debug { inner: self, label }
    }

    /// Transforms with `f(output, cursor_after)`, which may fail the parse.
    fn post_traverse<U, F>(self, f: F) -> PostTraverse<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output, Cursor) -> Result<U, String>,
    {
        PostTraverse { inner: self, f }
    }

    /// Like [`post_traverse`](Parser::post_traverse), but receives the cursor
    /// **before** the match.
    fn pre_traverse<U, F>(self, f: F) -> PreTraverse<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output, Cursor) -> Result<U, String>,
    {
        PreTraverse { inner: self, f }
    }
}

// ── Threading combinators ─────────────────────────────────────────────────────

/// [`Parser::map`].
pub struct Map<P, F> {
    inner: P,
    f: F,
}

impl<S: Stream, P, F, U> Parser<S> for Map<P, F>
where
    P: Parser<S>,
    F: Fn(P::Output) -> U,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, U> {
        let out = self.inner.parse_next(input)?;
        Ok((self.f)(out))
    }
}

/// [`Parser::ignored`].
pub struct Ignored<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Ignored<P> {
    type Output = ();
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, ()> {
        self.inner.parse_next(input)?;
        Ok(())
    }
}

/// [`Parser::then`].
pub struct Then<A, B> {
    first: A,
    second: B,
}

impl<S: Stream, A: Parser<S>, B: Parser<S>> Parser<S> for Then<A, B> {
    type Output = (A::Output, B::Output);
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, (A::Output, B::Output)> {
        let a = self.first.parse_next(input)?;
        let b = self.second.parse_next(input)?;
        Ok((a, b))
    }
}

/// [`Parser::ignore_then`].
pub struct IgnoreThen<A, B> {
    first: A,
    second: B,
}

impl<S: Stream, A: Parser<S>, B: Parser<S>> Parser<S> for IgnoreThen<A, B> {
    type Output = B::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, B::Output> {
        self.first.parse_next(input)?;
        self.second.parse_next(input)
    }
}

/// [`Parser::then_ignore`].
pub struct ThenIgnore<A, B> {
    first: A,
    second: B,
}

impl<S: Stream, A: Parser<S>, B: Parser<S>> Parser<S> for ThenIgnore<A, B> {
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, A::Output> {
        let a = self.first.parse_next(input)?;
        self.second.parse_next(input)?;
        Ok(a)
    }
}

/// [`Parser::or`].
pub struct Or<A, B> {
    a: A,
    b: B,
}

impl<S: Stream, A, B> Parser<S> for Or<A, B>
where
    A: Parser<S>,
    B: Parser<S, Output = A::Output>,
{
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, A::Output> {
        let start = *input;
        match self.a.parse_next(input) {
            Ok(out) => Ok(out),
            Err(ParseError::Incomplete(n)) => Err(ParseError::Incomplete(n)),
            Err(ParseError::Failure(err_a)) => {
                *input = start;
                match self.b.parse_next(input) {
                    Ok(out) => Ok(out),
                    Err(ParseError::Incomplete(n)) => Err(ParseError::Incomplete(n)),
                    Err(ParseError::Failure(err_b)) => {
                        *input = start;
                        let mut expected = err_a.expected;
                        expected.extend(err_b.expected);
                        Err(ParseError::Failure(ParseFailure {
                            reason: format!("{} or {}", err_a.reason, err_b.reason),
                            expected,
                            rest: start.stream.as_slice(),
                            cursor: start.cursor,
                        }))
                    }
                }
            }
        }
    }
}

/// [`Parser::optional`].
pub struct Opt<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Opt<P> {
    type Output = Option<P::Output>;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Option<P::Output>> {
        let start = *input;
        match self.inner.parse_next(input) {
            Ok(out) => Ok(Some(out)),
            // Propagate incomplete: on a partial stream we can't know if the
            // inner parser would have matched given more data.
            Err(e @ ParseError::Incomplete(_)) => Err(e),
            Err(ParseError::Failure(_)) => {
                *input = start;
                Ok(None)
            }
        }
    }
}

/// [`Parser::repeated`] / [`Parser::repeated_at_least`] / [`Parser::repeated_in`].
pub struct Repeated<P> {
    inner: P,
    min: usize,
    max: Option<usize>,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Repeated<P> {
    type Output = Vec<P::Output>;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Vec<P::Output>> {
        let mut out = Vec::new();
        loop {
            if self.max.is_some_and(|max| out.len() >= max) {
                break;
            }
            let start = *input;
            match self.inner.parse_next(input) {
                Ok(item) => {
                    if input.stream.len() == start.stream.len() {
                        // Non-advancing match: drop and stop to avoid infinite loop.
                        *input = start;
                        break;
                    }
                    out.push(item);
                }
                Err(ParseError::Incomplete(n)) => {
                    *input = start;
                    if out.len() < self.min || S::PARTIAL {
                        return Err(ParseError::Incomplete(n));
                    }
                    break;
                }
                Err(ParseError::Failure(err)) => {
                    *input = start;
                    if out.len() < self.min {
                        return Err(ParseError::Failure(err));
                    }
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// [`Parser::to`].
pub struct To<P, V> {
    inner: P,
    value: V,
}

impl<S: Stream, P: Parser<S>, V: Clone> Parser<S> for To<P, V> {
    type Output = V;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, V> {
        self.inner.parse_next(input)?;
        Ok(self.value.clone())
    }
}

/// [`Parser::fold`].
pub struct Fold<P, I, F> {
    inner: P,
    init: I,
    f: F,
}

impl<S: Stream, P, I, F, A> Parser<S> for Fold<P, I, F>
where
    P: Parser<S>,
    I: Fn() -> A,
    F: Fn(A, P::Output) -> A,
{
    type Output = A;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, A> {
        let mut acc = (self.init)();
        loop {
            let checkpoint = *input;
            match self.inner.parse_next(input) {
                Ok(item) => {
                    if input.stream.len() == checkpoint.stream.len() {
                        *input = checkpoint;
                        break;
                    }
                    acc = (self.f)(acc, item);
                }
                Err(ParseError::Incomplete(n)) => {
                    *input = checkpoint;
                    if S::PARTIAL {
                        return Err(ParseError::Incomplete(n));
                    }
                    break;
                }
                Err(ParseError::Failure(_)) => {
                    *input = checkpoint;
                    break;
                }
            }
        }
        Ok(acc)
    }
}

/// [`Parser::try_map`].
pub struct TryMap<P, F> {
    inner: P,
    f: F,
}

impl<S: Stream, P, F, U> Parser<S> for TryMap<P, F>
where
    P: Parser<S>,
    F: Fn(P::Output) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, U> {
        let out = self.inner.parse_next(input)?;
        (self.f)(out).map_err(|message| {
            ParseError::Failure(ParseFailure::rejected(
                message,
                input.stream.as_slice(),
                input.cursor,
            ))
        })
    }
}

/// [`Parser::flat_map`]. No [`Generate`] impl: the next parser depends on a
/// runtime-parsed value, so a `flat_map` grammar cannot be sampled.
pub struct FlatMap<P, F> {
    inner: P,
    f: F,
}

impl<S: Stream, P, F, U> Parser<S> for FlatMap<P, F>
where
    P: Parser<S>,
    F: Fn(P::Output) -> U,
    U: Parser<S>,
{
    type Output = U::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, U::Output> {
        let first = self.inner.parse_next(input)?;
        let next = (self.f)(first);
        next.parse_next(input)
    }
}

/// [`lookahead`].
pub struct Lookahead<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Lookahead<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        let start = *input;
        let out = self.inner.parse_next(input);
        *input = start;
        out
    }
}

/// Succeeds with `parser`'s output **without consuming input** (positive
/// lookahead), or fails if `parser` fails.
pub fn lookahead<P>(parser: P) -> Lookahead<P> {
    Lookahead { inner: parser }
}

/// [`not`].
pub struct Not<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Not<P> {
    type Output = ();
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, ()> {
        let start = *input;
        let matched = self.inner.parse_next(input).is_ok();
        *input = start;
        if matched {
            Err(ParseError::Failure(ParseFailure::rejected(
                "did not expect the lookahead parser to match",
                start.stream.as_slice(),
                start.cursor,
            )))
        } else {
            Ok(())
        }
    }
}

/// Succeeds (consuming nothing) only if `parser` fails — negative lookahead.
pub fn not<P>(parser: P) -> Not<P> {
    Not { inner: parser }
}

/// [`Parser::labelled`].
pub struct Labelled<P> {
    inner: P,
    label: &'static str,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Labelled<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        self.inner.parse_next(input).map_err(|e| match e {
            ParseError::Failure(err) => {
                ParseError::Failure(ParseFailure::expecting(self.label, err.rest, err.cursor))
            }
            ParseError::Incomplete(n) => ParseError::Incomplete(n),
        })
    }
}

/// [`Parser::with_byte_offset`].
pub struct WithByteOffset<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for WithByteOffset<P> {
    type Output = (P::Output, usize);
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, (P::Output, usize)> {
        let out = self.inner.parse_next(input)?;
        Ok((out, input.cursor.byte_offset))
    }
}

/// [`Parser::with_line`].
pub struct WithLine<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for WithLine<P> {
    type Output = (P::Output, (usize, usize));
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, (P::Output, (usize, usize))> {
        let out = self.inner.parse_next(input)?;
        Ok((out, (input.cursor.line, input.cursor.line_start_offset)))
    }
}

/// [`Parser::debug`].
pub struct Debug<P> {
    inner: P,
    label: &'static str,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Debug<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        let before = input.cursor;
        let preview = input.stream.preview(24);
        eprintln!(
            "[nimble_parsec_rs] {}: enter at line {}, byte {} — rest {:?}",
            self.label, before.line, before.byte_offset, preview
        );
        let result = self.inner.parse_next(input);
        match &result {
            Ok(_) => eprintln!(
                "[nimble_parsec_rs] {}: ok, now at byte {}",
                self.label, input.cursor.byte_offset
            ),
            Err(ParseError::Failure(err)) => {
                eprintln!("[nimble_parsec_rs] {}: failed — {}", self.label, err.reason)
            }
            Err(ParseError::Incomplete(_)) => {
                eprintln!("[nimble_parsec_rs] {}: incomplete", self.label)
            }
        }
        result
    }
}

/// [`Parser::post_traverse`].
pub struct PostTraverse<P, F> {
    inner: P,
    f: F,
}

impl<S: Stream, P, F, U> Parser<S> for PostTraverse<P, F>
where
    P: Parser<S>,
    F: Fn(P::Output, Cursor) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, U> {
        let out = self.inner.parse_next(input)?;
        (self.f)(out, input.cursor).map_err(|message| {
            ParseError::Failure(ParseFailure::rejected(
                message,
                input.stream.as_slice(),
                input.cursor,
            ))
        })
    }
}

/// [`Parser::pre_traverse`].
pub struct PreTraverse<P, F> {
    inner: P,
    f: F,
}

impl<S: Stream, P, F, U> Parser<S> for PreTraverse<P, F>
where
    P: Parser<S>,
    F: Fn(P::Output, Cursor) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, U> {
        let before = input.cursor;
        let out = self.inner.parse_next(input)?;
        (self.f)(out, before).map_err(|message| {
            ParseError::Failure(ParseFailure::rejected(
                message,
                input.stream.as_slice(),
                input.cursor,
            ))
        })
    }
}

// ── Leaf parsers — generic ────────────────────────────────────────────────────

/// [`literal`].
pub struct Literal<S, Pat> {
    lit: Pat,
    _phantom: PhantomData<fn(S)>,
}

impl<S, Pat> Parser<S> for Literal<S, Pat>
where
    S: Stream + crate::Compare<Pat>,
    Pat: Copy + core::fmt::Debug,
{
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Slice> {
        match input.stream.starts_with_pat(self.lit) {
            Some(n) => {
                let (consumed, _) = input.stream.split_at(n);
                input.bump(n);
                Ok(consumed)
            }
            None => Err(ParseError::Failure(ParseFailure::expecting(
                format!("expected {:?}", self.lit),
                input.stream.as_slice(),
                input.cursor,
            ))),
        }
    }
}

/// Matches the exact pattern `lit` at the front of the stream, yielding the
/// consumed slice. `lit` may be `&str` (on text streams) or `&[u8]` (on byte
/// streams); a `&str` pattern on a `&[u8]` stream is a **compile-time error**,
/// preventing silent misinterpretation.
pub fn literal<S, Pat: Copy + core::fmt::Debug>(lit: Pat) -> Literal<S, Pat> {
    Literal {
        lit,
        _phantom: PhantomData,
    }
}

/// [`any`] — yields `S::Token`: `char` for text streams, `u8` for byte streams.
pub struct AnyToken<S: Stream> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream> Parser<S> for AnyToken<S> {
    type Output = S::Token;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Token> {
        match input.stream.first() {
            Some((tok, w)) => {
                input.bump(w);
                Ok(tok)
            }
            None => incomplete_or_err("expected any token", input.stream.as_slice(), input.cursor),
        }
    }
}

/// Matches any single token (`char` for text, `u8` for bytes).
pub fn any<S: Stream>() -> AnyToken<S> {
    AnyToken {
        _phantom: PhantomData,
    }
}

/// [`satisfy`] — generic over `S::Token`.
pub struct Satisfy<S: Stream, F> {
    pred: F,
    label: &'static str,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream, F: Fn(S::Token) -> bool> Parser<S> for Satisfy<S, F> {
    type Output = S::Token;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Token> {
        match input.stream.first() {
            Some((tok, w)) if (self.pred)(tok) => {
                input.bump(w);
                Ok(tok)
            }
            Some(_) => Err(ParseError::Failure(ParseFailure::expecting(
                self.label,
                input.stream.as_slice(),
                input.cursor,
            ))),
            None => incomplete_or_err(self.label, input.stream.as_slice(), input.cursor),
        }
    }
}

/// Matches a single token satisfying `pred`; `label` describes it on failure.
/// Works for both text streams (`pred: Fn(char) -> bool`) and byte streams
/// (`pred: Fn(u8) -> bool`).
pub fn satisfy<S: Stream, F: Fn(S::Token) -> bool>(label: &'static str, pred: F) -> Satisfy<S, F> {
    Satisfy {
        pred,
        label,
        _phantom: PhantomData,
    }
}

/// [`take_while`] / [`take_while1`] — generic over `S::Token`.
pub struct TakeWhile<S: Stream, F> {
    pred: F,
    min: usize,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream, F: Fn(S::Token) -> bool> Parser<S> for TakeWhile<S, F> {
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Slice> {
        let mut end = 0usize;
        let mut count = 0usize;
        let mut scan = input.stream;
        let mut exhausted = false;
        loop {
            match scan.first() {
                Some((tok, w)) if (self.pred)(tok) => {
                    end += w;
                    count += 1;
                    let (_, rest) = scan.split_at(w);
                    scan = rest;
                }
                Some(_) => break,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        if count < self.min {
            if exhausted {
                return incomplete_or_err(
                    "expected at least one matching token",
                    input.stream.as_slice(),
                    input.cursor,
                );
            }
            return Err(ParseError::Failure(ParseFailure::expecting(
                "expected at least one matching token",
                input.stream.as_slice(),
                input.cursor,
            )));
        }
        let (consumed, _) = input.stream.split_at(end);
        input.bump(end);
        Ok(consumed)
    }
}

/// Consumes the maximal run of tokens satisfying `pred` (possibly empty).
pub fn take_while<S: Stream, F: Fn(S::Token) -> bool>(pred: F) -> TakeWhile<S, F> {
    TakeWhile {
        pred,
        min: 0,
        _phantom: PhantomData,
    }
}

/// Like [`take_while`], but requires at least one token.
pub fn take_while1<S: Stream, F: Fn(S::Token) -> bool>(pred: F) -> TakeWhile<S, F> {
    TakeWhile {
        pred,
        min: 1,
        _phantom: PhantomData,
    }
}

/// [`eof`].
pub struct Eof<S: Stream> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream> Parser<S> for Eof<S> {
    type Output = ();
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, ()> {
        if input.stream.is_empty() {
            Ok(())
        } else {
            Err(ParseError::Failure(ParseFailure::expecting(
                "expected end of input",
                input.stream.as_slice(),
                input.cursor,
            )))
        }
    }
}

/// Matches only at the end of input (or end of the current buffer for
/// `Partial` streams).
pub fn eof<S: Stream>() -> Eof<S> {
    Eof {
        _phantom: PhantomData,
    }
}

// ── Leaf parsers — text-specific ─────────────────────────────────────────────

/// Matches a single character contained in `set`.
pub fn one_of<S: Stream<Token = char>>(set: &'static str) -> Satisfy<S, impl Fn(char) -> bool> {
    satisfy("one of an expected set", move |c: char| set.contains(c))
}

/// Matches a single character **not** contained in `set`.
pub fn none_of<S: Stream<Token = char>>(set: &'static str) -> Satisfy<S, impl Fn(char) -> bool> {
    satisfy("a character outside an excluded set", move |c: char| {
        !set.contains(c)
    })
}

/// A run of one or more ASCII digits, yielding the consumed slice.
/// Works on any stream whose `Token = char` (i.e. `&str` and `Partial<&str>`).
pub fn digits<S: Stream<Token = char>>() -> TakeWhile<S, fn(char) -> bool> {
    TakeWhile {
        pred: |c: char| c.is_ascii_digit(),
        min: 1,
        _phantom: PhantomData,
    }
}

/// [`integer`].
pub struct Integer<S: Stream> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = char>> Parser<S> for Integer<S>
where
    S::Slice: AsRef<str>,
{
    type Output = i64;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, i64> {
        let mut end = 0usize;
        let mut count = 0usize;
        let mut scan = input.stream;
        let mut exhausted = false;
        loop {
            match scan.first() {
                Some((c, w)) if c.is_ascii_digit() => {
                    end += w;
                    count += 1;
                    let (_, rest) = scan.split_at(w);
                    scan = rest;
                }
                Some(_) => break,
                None => {
                    exhausted = true;
                    break;
                }
            }
        }
        if count == 0 {
            if exhausted {
                return incomplete_or_err(
                    "expected an integer",
                    input.stream.as_slice(),
                    input.cursor,
                );
            }
            return Err(ParseError::Failure(ParseFailure::expecting(
                "expected an integer",
                input.stream.as_slice(),
                input.cursor,
            )));
        }
        let (digit_slice, _) = input.stream.split_at(end);
        match digit_slice.as_ref().parse::<i64>() {
            Ok(value) => {
                input.bump(end);
                Ok(value)
            }
            Err(_) => Err(ParseError::Failure(ParseFailure::rejected(
                "integer out of range",
                input.stream.as_slice(),
                input.cursor,
            ))),
        }
    }
}

/// Parses a run of one or more ASCII digits into an `i64` (NimbleParsec's
/// `integer`), failing if the value overflows `i64`. For other widths or a
/// sign, compose `digits().try_map(...)`.
pub fn integer<S: Stream<Token = char>>() -> Integer<S>
where
    S::Slice: AsRef<str>,
{
    Integer {
        _phantom: PhantomData,
    }
}

// ── Leaf parsers — generic take / rest ───────────────────────────────────────

/// [`take`] / [`bytes`].
pub struct Take<S: Stream> {
    count: usize,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream> Parser<S> for Take<S> {
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Slice> {
        if input.stream.len() < self.count {
            return incomplete_or_err(
                &format!("expected {} bytes", self.count),
                input.stream.as_slice(),
                input.cursor,
            );
        }
        if !input.stream.is_valid_split(self.count) {
            return Err(ParseError::Failure(ParseFailure::rejected(
                format!(
                    "{} bytes does not land on a valid boundary \
                     (e.g. a UTF-8 codepoint boundary for text streams)",
                    self.count
                ),
                input.stream.as_slice(),
                input.cursor,
            )));
        }
        let (consumed, _) = input.stream.split_at(self.count);
        input.bump(self.count);
        Ok(consumed)
    }
}

/// Consumes exactly `count` base units, yielding them as `S::Slice`.
///
/// On `&str` streams, `count` must land on a UTF-8 codepoint boundary (same
/// constraint as before, now surfaced as a [`ParseFailure`] rather than a panic).
/// On `&[u8]` streams, any `count` is valid.
///
/// NimbleParsec's `bytes(n)` is an alias for this combinator.
pub fn take<S: Stream>(count: usize) -> Take<S> {
    Take {
        count,
        _phantom: PhantomData,
    }
}

/// Alias for [`take`] — NimbleParsec's `bytes(n)`.
///
/// On `&str` streams behaves identically to the old `bytes(n)` (counts UTF-8
/// bytes, fails off a char boundary). On `&[u8]` streams any count is valid.
pub fn bytes<S: Stream>(count: usize) -> Take<S> {
    Take {
        count,
        _phantom: PhantomData,
    }
}

/// [`rest`].
pub struct Rest<S: Stream> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream> Parser<S> for Rest<S> {
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Slice> {
        let n = input.stream.len();
        let consumed = input.stream.as_slice();
        input.bump(n);
        Ok(consumed)
    }
}

/// Consumes all remaining input, yielding it as `S::Slice`.
pub fn rest<S: Stream>() -> Rest<S> {
    Rest {
        _phantom: PhantomData,
    }
}

// ── Leaf parsers — byte-stream-specific (Phase 2) ─────────────────────────────

/// [`byte`].
pub struct Byte<S: Stream<Token = u8>> {
    expected: u8,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = u8>> Parser<S> for Byte<S> {
    type Output = u8;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, u8> {
        match input.stream.first() {
            Some((b, 1)) if b == self.expected => {
                input.bump(1);
                Ok(b)
            }
            Some((b, _)) => Err(ParseError::Failure(ParseFailure::expecting(
                format!("expected byte 0x{:02x}, found 0x{:02x}", self.expected, b),
                input.stream.as_slice(),
                input.cursor,
            ))),
            None => incomplete_or_err(
                &format!("expected byte 0x{:02x}", self.expected),
                input.stream.as_slice(),
                input.cursor,
            ),
        }
    }
}

/// Matches a single specific byte value (NimbleParsec's single-byte
/// `ascii_char` range, generalised). Only available on byte streams
/// (`Token = u8`).
pub fn byte<S: Stream<Token = u8>>(b: u8) -> Byte<S> {
    Byte {
        expected: b,
        _phantom: PhantomData,
    }
}

/// [`byte_range`].
pub struct ByteRange<S: Stream<Token = u8>> {
    lo: u8,
    hi: u8,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = u8>> Parser<S> for ByteRange<S> {
    type Output = u8;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, u8> {
        match input.stream.first() {
            Some((b, _)) if b >= self.lo && b <= self.hi => {
                input.bump(1);
                Ok(b)
            }
            Some(_) => Err(ParseError::Failure(ParseFailure::expecting(
                format!("expected byte in 0x{:02x}..=0x{:02x}", self.lo, self.hi),
                input.stream.as_slice(),
                input.cursor,
            ))),
            None => incomplete_or_err(
                &format!("expected byte in 0x{:02x}..=0x{:02x}", self.lo, self.hi),
                input.stream.as_slice(),
                input.cursor,
            ),
        }
    }
}

/// Matches a single byte in the inclusive range `[lo, hi]` (Elixir
/// `ascii_char` range parity). Only available on byte streams.
pub fn byte_range<S: Stream<Token = u8>>(lo: u8, hi: u8) -> ByteRange<S> {
    ByteRange {
        lo,
        hi,
        _phantom: PhantomData,
    }
}

// ── Binary numeric parsers (Phase 2) ─────────────────────────────────────────

macro_rules! impl_binary_num {
    ($Struct:ident, $fn_name:ident, $out:ty, $n_bytes:expr, $from_bytes:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $Struct<S: Stream<Token = u8>> {
            _phantom: PhantomData<fn(S)>,
        }

        impl<S: Stream<Token = u8>> Parser<S> for $Struct<S>
        where
            S::Slice: AsRef<[u8]>,
        {
            type Output = $out;
            fn parse_next(&self, input: &mut Input<S>) -> PResult<S, $out> {
                const N: usize = $n_bytes;
                if input.stream.len() < N {
                    return incomplete_or_err(
                        concat!(
                            "expected ",
                            stringify!($n_bytes),
                            " bytes for ",
                            stringify!($out)
                        ),
                        input.stream.as_slice(),
                        input.cursor,
                    );
                }
                let (slice, _) = input.stream.split_at(N);
                let src = slice.as_ref();
                let mut arr = [0u8; N];
                arr.copy_from_slice(&src[..N]);
                input.bump(N);
                Ok(<$out>::$from_bytes(arr))
            }
        }

        #[doc = $doc]
        pub fn $fn_name<S: Stream<Token = u8>>() -> $Struct<S>
        where
            S::Slice: AsRef<[u8]>,
        {
            $Struct {
                _phantom: PhantomData,
            }
        }
    };
}

impl_binary_num!(BeU16, be_u16, u16, 2, from_be_bytes, "Big-endian `u16`.");
impl_binary_num!(BeU32, be_u32, u32, 4, from_be_bytes, "Big-endian `u32`.");
impl_binary_num!(BeU64, be_u64, u64, 8, from_be_bytes, "Big-endian `u64`.");
impl_binary_num!(LeU16, le_u16, u16, 2, from_le_bytes, "Little-endian `u16`.");
impl_binary_num!(LeU32, le_u32, u32, 4, from_le_bytes, "Little-endian `u32`.");
impl_binary_num!(LeU64, le_u64, u64, 8, from_le_bytes, "Little-endian `u64`.");
impl_binary_num!(BeI16, be_i16, i16, 2, from_be_bytes, "Big-endian `i16`.");
impl_binary_num!(BeI32, be_i32, i32, 4, from_be_bytes, "Big-endian `i32`.");
impl_binary_num!(BeI64, be_i64, i64, 8, from_be_bytes, "Big-endian `i64`.");
impl_binary_num!(LeI16, le_i16, i16, 2, from_le_bytes, "Little-endian `i16`.");
impl_binary_num!(LeI32, le_i32, i32, 4, from_le_bytes, "Little-endian `i32`.");
impl_binary_num!(LeI64, le_i64, i64, 8, from_le_bytes, "Little-endian `i64`.");
impl_binary_num!(BeF32, be_f32, f32, 4, from_be_bytes, "Big-endian `f32`.");
impl_binary_num!(BeF64, be_f64, f64, 8, from_be_bytes, "Big-endian `f64`.");
impl_binary_num!(LeF32, le_f32, f32, 4, from_le_bytes, "Little-endian `f32`.");
impl_binary_num!(LeF64, le_f64, f64, 8, from_le_bytes, "Little-endian `f64`.");

// ── Bit-level parsing ────────────────────────────────────────────────────────

/// Types that can accumulate individual bits MSB-first.
///
/// Implemented for `u8`, `u16`, `u32`, `u64`, `u128`. The output of
/// [`take_bits`] is generic over this trait so you choose the width at the
/// call site: `take_bits::<u8, _>(4)` or `take_bits::<u32, _>(24)`.
pub trait BitOutput:
    Copy
    + core::ops::Shl<usize, Output = Self>
    + core::ops::BitOr<Output = Self>
    + core::fmt::Debug
    + PartialEq
{
    /// The zero value.
    fn zero() -> Self;
    /// The one value (used to represent a set bit).
    fn one() -> Self;
}

macro_rules! impl_bit_output {
    ($($t:ty),+) => {
        $(impl BitOutput for $t {
            #[inline] fn zero() -> Self { 0 }
            #[inline] fn one()  -> Self { 1 }
        })+
    };
}
impl_bit_output!(u8, u16, u32, u64, u128);

// ── take_bits ────────────────────────────────────────────────────────────────

/// [`take_bits`].
pub struct TakeBits<O: BitOutput, S: Stream<Token = u8>> {
    n_bits: usize,
    _phantom: PhantomData<fn(S) -> O>,
}

impl<O: BitOutput, S: Stream<Token = u8>> Parser<crate::Bits<S>> for TakeBits<O, S> {
    type Output = O;

    fn parse_next(&self, input: &mut Input<crate::Bits<S>>) -> PResult<crate::Bits<S>, O> {
        if input.stream.len() < self.n_bits {
            return incomplete_or_err(
                &format!("expected {} bits", self.n_bits),
                input.stream.as_slice(),
                input.cursor,
            );
        }
        let mut acc = O::zero();
        for _ in 0..self.n_bits {
            let (bit, w) = input.stream.first().unwrap();
            acc = (acc << 1) | if bit { O::one() } else { O::zero() };
            input.bump(w);
        }
        Ok(acc)
    }
}

/// Reads exactly `n_bits` bits from a [`Bits`](crate::Bits) stream and
/// accumulates them MSB-first into `O` (e.g. `u8`, `u32`).
///
/// # Example
/// ```
/// use nimble_parsec_rs::typed::{take_bits, bits, Parser};
///
/// let result = bits(take_bits::<u8, &[u8]>(4))
///     .parse(b"\xAB".as_ref())
///     .unwrap();
/// assert_eq!(result, 0x0A); // upper nibble of 0xAB
/// ```
pub fn take_bits<O: BitOutput, S: Stream<Token = u8>>(n_bits: usize) -> TakeBits<O, S> {
    TakeBits {
        n_bits,
        _phantom: PhantomData,
    }
}

// ── bit_bool ─────────────────────────────────────────────────────────────────

/// [`bit_bool`].
pub struct BitBool<S: Stream<Token = u8>> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = u8>> Parser<crate::Bits<S>> for BitBool<S> {
    type Output = bool;

    fn parse_next(&self, input: &mut Input<crate::Bits<S>>) -> PResult<crate::Bits<S>, bool> {
        match input.stream.first() {
            Some((bit, w)) => {
                input.bump(w);
                Ok(bit)
            }
            None => incomplete_or_err("expected a bit", input.stream.as_slice(), input.cursor),
        }
    }
}

/// Reads a single bit from a [`Bits`](crate::Bits) stream, yielding `true`
/// for 1 and `false` for 0.
///
/// # Example
/// ```
/// use nimble_parsec_rs::typed::{bit_bool, bits, Parser};
///
/// let bools = bits(bit_bool::<&[u8]>().repeated())
///     .parse(b"\x80".as_ref())
///     .unwrap();
/// assert_eq!(bools, vec![true, false, false, false, false, false, false, false]);
/// ```
pub fn bit_bool<S: Stream<Token = u8>>() -> BitBool<S> {
    BitBool {
        _phantom: PhantomData,
    }
}

// ── bits() — enter bit context ───────────────────────────────────────────────

/// [`bits`].
pub struct BitsOf<P, S: Stream<Token = u8>> {
    inner: P,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = u8>, P: Parser<crate::Bits<S>>> Parser<S> for BitsOf<P, S> {
    type Output = P::Output;

    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        let bits_stream = crate::Bits::new(input.stream());
        let mut bits_input = Input::new(bits_stream);

        let result = self.inner.parse_next(&mut bits_input).map_err(|e| {
            // Convert the bit-context error to a byte-context error.
            match e {
                ParseError::Incomplete(n) => ParseError::Incomplete(n),
                ParseError::Failure(f) => ParseError::Failure(ParseFailure {
                    reason: f.reason,
                    expected: f.expected,
                    rest: input.stream().as_slice(),
                    cursor: input.cursor(),
                }),
            }
        })?;

        // bits_input.cursor().byte_offset counts BITS consumed (not bytes),
        // because Bits::advance_cursor adds 1 per bit to byte_offset.
        let bits_used = bits_input.cursor().byte_offset;
        let bytes_adv = bits_used / 8 + (bits_used % 8 != 0) as usize;
        input.bump(bytes_adv);

        Ok(result)
    }
}

/// Runs parser `inner` on a bit-level view of the current byte stream, then
/// advances the byte stream past all bytes touched by the inner parse.
///
/// The inner parser operates on a [`Bits<S>`](crate::Bits) stream where each
/// token is a `bool` (MSB-first). When it finishes, the outer byte stream
/// advances by `⌈bits_consumed / 8⌉` bytes.
///
/// Use [`byte_aligned`] inside the bit context when you need to guarantee
/// alignment to the next byte boundary before re-entering byte parsing.
///
/// # Example
/// ```
/// use nimble_parsec_rs::typed::{take_bits, bits, Parser};
///
/// // Split one byte into two nibbles.
/// let (hi, lo) = bits(take_bits::<u8, &[u8]>(4).then(take_bits::<u8, &[u8]>(4)))
///     .parse(b"\xAB".as_ref())
///     .unwrap();
/// assert_eq!((hi, lo), (0x0A, 0x0B));
/// ```
pub fn bits<S: Stream<Token = u8>, P: Parser<crate::Bits<S>>>(inner: P) -> BitsOf<P, S> {
    BitsOf {
        inner,
        _phantom: PhantomData,
    }
}

// ── byte_aligned() — exit to byte boundary ──────────────────────────────────

/// [`byte_aligned`].
pub struct ByteAligned<P, S: Stream<Token = u8>> {
    inner: P,
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream<Token = u8>, P: Parser<crate::Bits<S>>> Parser<crate::Bits<S>>
    for ByteAligned<P, S>
{
    type Output = P::Output;

    fn parse_next(&self, input: &mut Input<crate::Bits<S>>) -> PResult<crate::Bits<S>, P::Output> {
        let result = self.inner.parse_next(input)?;
        // Skip the remaining bits in the current byte so the position is on a
        // byte boundary.  If bit_offset is already 0 we are already aligned.
        let leftover = input.stream().bit_offset;
        if leftover > 0 {
            let skip = 8 - leftover as usize;
            input.bump(skip);
        }
        Ok(result)
    }
}

/// Runs `inner` on a [`Bits`](crate::Bits) stream and then advances past any
/// remaining bits in the current byte, ensuring the stream is on a byte
/// boundary when this combinator returns.
///
/// Useful when a field does not fill a whole byte: e.g. after parsing a 3-bit
/// field, `byte_aligned` skips 5 padding bits.
///
/// # Example
/// ```
/// use nimble_parsec_rs::typed::{take_bits, bits, byte_aligned, Parser};
///
/// // Parse 3-bit field; discard remaining 5 bits; parse next full byte.
/// let (field, next_byte) = bits(
///     byte_aligned(take_bits::<u8, &[u8]>(3))
///         .then(take_bits::<u8, &[u8]>(8))
/// )
/// .parse(b"\xE0\xFF".as_ref())
/// .unwrap();
/// assert_eq!(field, 0b111);  // top 3 bits of 0xE0
/// assert_eq!(next_byte, 0xFF);
/// ```
pub fn byte_aligned<S: Stream<Token = u8>, P: Parser<crate::Bits<S>>>(
    inner: P,
) -> ByteAligned<P, S> {
    ByteAligned {
        inner,
        _phantom: PhantomData,
    }
}

// ── utf8_char — text on bytes (Phase 3) ─────────────────────────────────────

/// [`utf8_char`].
pub struct Utf8Char<S: Stream<Token = u8>> {
    _phantom: PhantomData<fn(S)>,
}

/// Decodes one UTF-8 codepoint from a **byte** stream (`&[u8]` or
/// `Partial<&[u8]>`), yielding a `char`. This is the bridge that lets text
/// grammars run on raw byte input — the analogue of Elixir's `utf8_char`.
///
/// Fails (hard error) if the bytes do not form a valid UTF-8 sequence.
/// Returns `Incomplete` at end-of-buffer on partial streams.
pub fn utf8_char<S: Stream<Token = u8>>() -> Utf8Char<S>
where
    S::Slice: AsRef<[u8]>,
{
    Utf8Char {
        _phantom: PhantomData,
    }
}

impl<S: Stream<Token = u8>> Parser<S> for Utf8Char<S>
where
    S::Slice: AsRef<[u8]>,
{
    type Output = char;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, char> {
        match input.stream.first() {
            None => incomplete_or_err(
                "expected UTF-8 character",
                input.stream.as_slice(),
                input.cursor,
            ),
            Some((first_byte, _)) => {
                let seq_len: usize = match first_byte {
                    0x00..=0x7F => 1,
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => {
                        return Err(ParseError::Failure(ParseFailure::rejected(
                            format!("invalid UTF-8 start byte 0x{:02x}", first_byte),
                            input.stream.as_slice(),
                            input.cursor,
                        )))
                    }
                };
                if input.stream.len() < seq_len {
                    return incomplete_or_err(
                        "expected complete UTF-8 sequence",
                        input.stream.as_slice(),
                        input.cursor,
                    );
                }
                let (slice, _) = input.stream.split_at(seq_len);
                let raw = slice.as_ref();
                match std::str::from_utf8(raw) {
                    Ok(s) => {
                        let c = s.chars().next().unwrap();
                        input.bump(seq_len);
                        Ok(c)
                    }
                    Err(_) => Err(ParseError::Failure(ParseFailure::rejected(
                        "invalid UTF-8 byte sequence",
                        input.stream.as_slice(),
                        input.cursor,
                    ))),
                }
            }
        }
    }
}

// ── length_take (Phase 5) ─────────────────────────────────────────────────────

/// [`length_take`].
pub struct LengthTake<P> {
    prefix: P,
}

impl<S: Stream, P: Parser<S, Output = usize>> Parser<S> for LengthTake<P> {
    type Output = S::Slice;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, S::Slice> {
        let n = self.prefix.parse_next(input)?;
        if input.stream.len() < n {
            return incomplete_or_err(
                &format!("expected {} bytes after length prefix", n),
                input.stream.as_slice(),
                input.cursor,
            );
        }
        if !input.stream.is_valid_split(n) {
            return Err(ParseError::Failure(ParseFailure::rejected(
                format!("{} does not land on a valid split boundary", n),
                input.stream.as_slice(),
                input.cursor,
            )));
        }
        let (consumed, _) = input.stream.split_at(n);
        input.bump(n);
        Ok(consumed)
    }
}

/// Runs `prefix` to obtain a byte count `n`, then consumes exactly `n` base
/// units of the stream, yielding them as `S::Slice`.
///
/// Idiomatic replacement for `prefix.flat_map(|n| take(n))` for
/// length-prefixed binary records.
pub fn length_take<P>(prefix: P) -> LengthTake<P> {
    LengthTake { prefix }
}

// ── Tokens<T> — kept for backward compatibility ───────────────────────────────

/// A newtype wrapping `&[T]` that was previously needed because `&[u8]` had a
/// dedicated `Stream` impl that would conflict with a generic `&[T]` impl.
///
/// **Deprecated since 0.2.0.** Pass `&[T]` (e.g. `tokens.as_slice()`) directly;
/// the blanket `impl<T> Stream for &[T]` now handles all element types
/// including `u8`.
///
/// ```ignore
/// // Before (still compiles but deprecated):
/// let ast = my_parser.parse(Tokens(&tokens)).unwrap();
///
/// // After:
/// let ast = my_parser.parse(tokens.as_slice()).unwrap();
/// ```
#[deprecated(
    since = "0.2.0",
    note = "pass `tokens.as_slice()` (type `&[T]`) directly; \
            the blanket `impl Stream for &[T]` now covers all token types"
)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tokens<'i, T>(pub &'i [T]);

#[allow(deprecated)]
impl<T> crate::StreamIsPartial for Tokens<'_, T> {
    const PARTIAL: bool = false;
}

#[allow(deprecated)]
impl<'i, T: Copy + PartialEq + core::fmt::Debug> Stream for Tokens<'i, T> {
    type Token = T;
    type Slice = &'i [T];

    fn first(self) -> Option<(T, usize)> {
        self.0.split_first().map(|(&t, _)| (t, 1))
    }

    fn split_at(self, n: usize) -> (&'i [T], Self) {
        let (a, b) = <[T]>::split_at(self.0, n);
        (a, Tokens(b))
    }

    fn as_slice(self) -> &'i [T] {
        self.0
    }

    fn len(self) -> usize {
        self.0.len()
    }

    fn advance_cursor(self, cursor: Cursor, n: usize) -> Cursor {
        Cursor {
            byte_offset: cursor.byte_offset + n,
            ..cursor
        }
    }

    fn preview(self, max_tokens: usize) -> String {
        format!("{:?}", &self.0[..max_tokens.min(self.0.len())])
    }
}

// ── Eventually ───────────────────────────────────────────────────────────────

/// [`eventually`].
pub struct Eventually<P> {
    inner: P,
}

impl<S: Stream, P: Parser<S>> Parser<S> for Eventually<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        loop {
            let checkpoint = *input;
            if let Ok(out) = self.inner.parse_next(input) {
                return Ok(out);
            }
            *input = checkpoint;
            match input.stream.first() {
                Some((_, w)) => input.bump(w),
                None => {
                    return Err(ParseError::Failure(ParseFailure::expecting(
                        "expected the parser to eventually match",
                        input.stream.as_slice(),
                        input.cursor,
                    )))
                }
            }
        }
    }
}

/// Skips input one token at a time until `parser` matches, returning its
/// output. Fails if end of input is reached first.
pub fn eventually<P>(parser: P) -> Eventually<P> {
    Eventually { inner: parser }
}

// ── Recursion ─────────────────────────────────────────────────────────────────

/// A forward-declared, self-referential parser built by [`recursive`].
/// Cloning shares the same definition via an `Rc`. Recursion depth is bounded
/// by [`DEFAULT_MAX_RECURSION_DEPTH`] (configurable via
/// [`Parser::parse_with_max_depth`]).
///
/// The lifetime `'a` bounds the parsers stored inside: they may hold references
/// that live at least as long as `'a`. For `&'static str` streams `'a = 'static`
/// is the natural choice; for shorter-lived streams (e.g. `&'i str` inside a
/// function) `'a` is inferred to match the stream lifetime.
pub struct Recursive<'a, S: Stream, O> {
    cell: Rc<OnceCell<Box<dyn Parser<S, Output = O> + 'a>>>,
}

impl<'a, S: Stream, O> Clone for Recursive<'a, S, O> {
    fn clone(&self) -> Self {
        Recursive {
            cell: Rc::clone(&self.cell),
        }
    }
}

impl<'a, S: Stream, O> Parser<S> for Recursive<'a, S, O> {
    type Output = O;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, O> {
        let parser = self
            .cell
            .get()
            .expect("recursive parser used before it was defined");
        let budget = crate::RECURSION_BUDGET.with(Cell::get);
        if budget == 0 {
            return Err(ParseError::Failure(ParseFailure::rejected(
                "maximum recursion depth exceeded",
                input.stream.as_slice(),
                input.cursor,
            )));
        }
        crate::RECURSION_BUDGET.with(|b| b.set(budget - 1));
        let result = parser.parse_next(input);
        crate::RECURSION_BUDGET.with(|b| b.set(budget));
        result
    }
}

/// Builds a recursive parser. `build` receives a handle usable within the
/// definition it returns — enabling self-referential grammars.
///
/// The lifetime `'a` is the lifetime bound on the parsers stored inside the
/// recursive definition. For most use cases it is inferred automatically:
/// - With `&'static str` streams all built-in parsers are `'static`, so `'a`
///   defaults to `'static`.
/// - Inside a function returning `impl Parser<&'i str, …>`, `'a` is inferred as
///   `'i`, allowing the inner parsers to hold references scoped to that lifetime.
pub fn recursive<'a, S, O, P, F>(build: F) -> Recursive<'a, S, O>
where
    S: Stream,
    P: Parser<S, Output = O> + 'a,
    F: FnOnce(Recursive<'a, S, O>) -> P,
{
    let handle = Recursive {
        cell: Rc::new(OnceCell::new()),
    };
    let definition = build(handle.clone());
    let _ = handle.cell.set(Box::new(definition));
    handle
}

// ── Generation ────────────────────────────────────────────────────────────────

/// Deterministic source of randomness for [`generate`].
pub struct Gen {
    state: u64,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Gen {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (0 when `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// Coin flip.
    pub fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// A character satisfying `pred`, sampled from a printable pool.
    pub fn char_matching(&mut self, pred: &dyn Fn(char) -> bool) -> char {
        const POOL: &[u8] =
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 _-.,:;/()[]";
        let start = self.below(POOL.len());
        for i in 0..POOL.len() {
            let c = POOL[(start + i) % POOL.len()] as char;
            if pred(c) {
                return c;
            }
        }
        for cp in 0x20u32..0x7f {
            if let Some(c) = char::from_u32(cp) {
                if pred(c) {
                    return c;
                }
            }
        }
        'a'
    }
}

/// Produces a random input string that the parser accepts (for property-based
/// testing / fuzzing). Implemented for the built-in **text** combinators; byte
/// and token-slice parsers do not implement `Generate` (use proptest strategies
/// directly for those). Grammars using [`recursive`] or [`Parser::flat_map`]
/// are not generatable.
pub trait Generate {
    /// Appends one sampled instance of this parser's accepted input to `out`.
    fn generate_into(&self, gen: &mut Gen, out: &mut String);
}

/// Generates a random input accepted by `parser`, seeded by `seed`.
pub fn generate<G: Generate>(parser: &G, seed: u64) -> String {
    let mut gen = Gen::new(seed);
    let mut out = String::new();
    parser.generate_into(&mut gen, &mut out);
    out
}

impl<S: Stream<Token = char> + crate::Compare<&'static str>> Generate for Literal<S, &'static str> {
    fn generate_into(&self, _gen: &mut Gen, out: &mut String) {
        out.push_str(self.lit);
    }
}

impl<S: Stream<Token = char>> Generate for AnyToken<S> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        out.push(gen.char_matching(&|_| true));
    }
}

impl<S: Stream<Token = char>, F: Fn(char) -> bool> Generate for Satisfy<S, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        out.push(gen.char_matching(&self.pred));
    }
}

impl<S: Stream<Token = char>, F: Fn(char) -> bool> Generate for TakeWhile<S, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        let count = self.min + gen.below(3);
        for _ in 0..count {
            out.push(gen.char_matching(&self.pred));
        }
    }
}

impl<S: Stream<Token = char>> Generate for Take<S> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        // Generate ASCII alphanumeric chars (1 byte each on &str streams).
        for _ in 0..self.count {
            out.push(gen.char_matching(&|c: char| c.is_ascii_alphanumeric()));
        }
    }
}

impl<S: Stream<Token = char>> Generate for Rest<S> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

impl<S: Stream> Generate for Eof<S> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

impl<S: Stream<Token = char>> Generate for Integer<S>
where
    S::Slice: AsRef<str>,
{
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        for _ in 0..1 + gen.below(3) {
            out.push((b'0' + gen.below(10) as u8) as char);
        }
    }
}

impl<P: Generate, F> Generate for Map<P, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate, F> Generate for TryMap<P, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate, V> Generate for To<P, V> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate, I, F> Generate for Fold<P, I, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        for _ in 0..gen.below(3) {
            self.inner.generate_into(gen, out);
        }
    }
}

impl<P: Generate> Generate for Ignored<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate> Generate for Labelled<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate> Generate for WithByteOffset<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate> Generate for WithLine<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate> Generate for Debug<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate, F> Generate for PostTraverse<P, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<P: Generate, F> Generate for PreTraverse<P, F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

impl<A: Generate, B: Generate> Generate for Then<A, B> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.first.generate_into(gen, out);
        self.second.generate_into(gen, out);
    }
}

impl<A: Generate, B: Generate> Generate for IgnoreThen<A, B> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.first.generate_into(gen, out);
        self.second.generate_into(gen, out);
    }
}

impl<A: Generate, B: Generate> Generate for ThenIgnore<A, B> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.first.generate_into(gen, out);
        self.second.generate_into(gen, out);
    }
}

impl<A: Generate, B: Generate> Generate for Or<A, B> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        if gen.coin() {
            self.a.generate_into(gen, out);
        } else {
            self.b.generate_into(gen, out);
        }
    }
}

impl<P: Generate> Generate for Opt<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        if gen.coin() {
            self.inner.generate_into(gen, out);
        }
    }
}

impl<P: Generate> Generate for Repeated<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        let mut count = self.min + gen.below(3);
        if let Some(max) = self.max {
            count = count.min(max);
        }
        for _ in 0..count {
            self.inner.generate_into(gen, out);
        }
    }
}

impl<P> Generate for Lookahead<P> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

impl<P> Generate for Not<P> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

impl<P: Generate> Generate for Eventually<P> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.inner.generate_into(gen, out);
    }
}

// ── Choice / Alternatives ─────────────────────────────────────────────────────

/// A set of alternatives for [`choice`] — implemented for homogeneous arrays
/// `[P; N]` and for heterogeneous tuples `(A, B, …)` up to arity 8.
pub trait Alternatives<S: Stream> {
    /// The shared output type of every alternative.
    type Output;
    /// Tries each alternative in order, returning the first success or a
    /// failure unioning the alternatives' expectations.
    fn choice_parse(&self, input: &mut Input<S>) -> PResult<S, Self::Output>;
}

fn choice_failure<S: Stream>(
    reasons: Vec<String>,
    expected: Vec<String>,
    at: Input<S>,
) -> ParseError<S> {
    ParseError::Failure(ParseFailure {
        reason: if reasons.is_empty() {
            "choice has no options".to_string()
        } else {
            reasons.join(" or ")
        },
        expected,
        rest: at.stream.as_slice(),
        cursor: at.cursor,
    })
}

impl<S: Stream, P: Parser<S>, const N: usize> Alternatives<S> for [P; N] {
    type Output = P::Output;
    fn choice_parse(&self, input: &mut Input<S>) -> PResult<S, P::Output> {
        let start = *input;
        let mut reasons = Vec::with_capacity(N);
        let mut expected = Vec::new();
        for parser in self {
            match parser.parse_next(input) {
                Ok(out) => return Ok(out),
                Err(ParseError::Incomplete(n)) => return Err(ParseError::Incomplete(n)),
                Err(ParseError::Failure(err)) => {
                    *input = start;
                    reasons.push(err.reason);
                    expected.extend(err.expected);
                }
            }
        }
        Err(choice_failure(reasons, expected, start))
    }
}

macro_rules! impl_alternatives_tuple {
    ($($idx:tt $param:ident),+) => {
        impl<S: Stream, O, $($param: Parser<S, Output = O>),+> Alternatives<S> for ($($param,)+) {
            type Output = O;
            fn choice_parse(&self, input: &mut Input<S>) -> PResult<S, O> {
                let start = *input;
                let mut reasons = Vec::new();
                let mut expected = Vec::new();
                $(
                    match self.$idx.parse_next(input) {
                        Ok(out) => return Ok(out),
                        Err(ParseError::Incomplete(n)) => return Err(ParseError::Incomplete(n)),
                        Err(ParseError::Failure(err)) => {
                            *input = start;
                            reasons.push(err.reason);
                            expected.extend(err.expected);
                        }
                    }
                )+
                Err(choice_failure(reasons, expected, start))
            }
        }
    };
}

impl_alternatives_tuple!(0 P0, 1 P1);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2, 3 P3);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2, 3 P3, 4 P4);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5, 6 P6);
impl_alternatives_tuple!(0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5, 6 P6, 7 P7);

/// [`choice`].
pub struct ChoiceOf<A> {
    alternatives: A,
}

impl<S: Stream, A: Alternatives<S>> Parser<S> for ChoiceOf<A> {
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, A::Output> {
        self.alternatives.choice_parse(input)
    }
}

/// Tries each alternative in order, returning the first success. Accepts an
/// array `[p; N]` (same parser type) or a tuple `(a, b, …)` up to arity 8.
pub fn choice<A>(alternatives: A) -> ChoiceOf<A> {
    ChoiceOf { alternatives }
}

/// Generation counterpart of [`Alternatives`]: samples one alternative.
pub trait GenerateAlternatives {
    /// Generates input for a randomly chosen alternative.
    fn generate_alt(&self, gen: &mut Gen, out: &mut String);
}

impl<P: Generate, const N: usize> GenerateAlternatives for [P; N] {
    fn generate_alt(&self, gen: &mut Gen, out: &mut String) {
        if N > 0 {
            self[gen.below(N)].generate_into(gen, out);
        }
    }
}

macro_rules! impl_generate_alternatives_tuple {
    ($n:expr; $($idx:tt $param:ident),+) => {
        impl<$($param: Generate),+> GenerateAlternatives for ($($param,)+) {
            fn generate_alt(&self, gen: &mut Gen, out: &mut String) {
                match gen.below($n) {
                    $( $idx => self.$idx.generate_into(gen, out), )+
                    _ => {}
                }
            }
        }
    };
}

impl_generate_alternatives_tuple!(2; 0 P0, 1 P1);
impl_generate_alternatives_tuple!(3; 0 P0, 1 P1, 2 P2);
impl_generate_alternatives_tuple!(4; 0 P0, 1 P1, 2 P2, 3 P3);
impl_generate_alternatives_tuple!(5; 0 P0, 1 P1, 2 P2, 3 P3, 4 P4);
impl_generate_alternatives_tuple!(6; 0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5);
impl_generate_alternatives_tuple!(7; 0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5, 6 P6);
impl_generate_alternatives_tuple!(8; 0 P0, 1 P1, 2 P2, 3 P3, 4 P4, 5 P5, 6 P6, 7 P7);

impl<A: GenerateAlternatives> Generate for ChoiceOf<A> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.alternatives.generate_alt(gen, out);
    }
}

// ── Convenience combinators ───────────────────────────────────────────────────

/// [`delimited`].
pub struct Delimited<A, B, C> {
    open: A,
    content: B,
    close: C,
}

impl<S: Stream, A: Parser<S>, B: Parser<S>, C: Parser<S>> Parser<S> for Delimited<A, B, C> {
    type Output = B::Output;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, B::Output> {
        self.open.parse_next(input)?;
        let out = self.content.parse_next(input)?;
        self.close.parse_next(input)?;
        Ok(out)
    }
}

impl<A: Generate, B: Generate, C: Generate> Generate for Delimited<A, B, C> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        self.open.generate_into(gen, out);
        self.content.generate_into(gen, out);
        self.close.generate_into(gen, out);
    }
}

/// Parses `content` between `open` and `close`, keeping only `content`'s output.
pub fn delimited<A, B, C>(open: A, content: B, close: C) -> Delimited<A, B, C> {
    Delimited {
        open,
        content,
        close,
    }
}

/// [`separated_by`] / [`separated_by1`].
pub struct SeparatedBy<I, Sep> {
    item: I,
    sep: Sep,
    min: usize,
}

impl<S: Stream, I: Parser<S>, Sep: Parser<S>> Parser<S> for SeparatedBy<I, Sep> {
    type Output = Vec<I::Output>;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Vec<I::Output>> {
        let mut items = Vec::new();
        let start = *input;
        match self.item.parse_next(input) {
            Ok(first) => items.push(first),
            Err(ParseError::Incomplete(n)) => return Err(ParseError::Incomplete(n)),
            Err(err) => {
                *input = start;
                if self.min == 0 {
                    return Ok(items);
                }
                return Err(err);
            }
        }
        loop {
            let checkpoint = *input;
            match self.sep.parse_next(input) {
                Err(ParseError::Incomplete(n)) => {
                    *input = checkpoint;
                    if S::PARTIAL {
                        return Err(ParseError::Incomplete(n));
                    }
                    break;
                }
                Err(_) => {
                    *input = checkpoint;
                    break;
                }
                Ok(_) => {}
            }
            match self.item.parse_next(input) {
                Ok(item) => {
                    if input.stream.len() == checkpoint.stream.len() {
                        *input = checkpoint;
                        break;
                    }
                    items.push(item);
                }
                Err(ParseError::Incomplete(n)) => {
                    *input = checkpoint;
                    if S::PARTIAL {
                        return Err(ParseError::Incomplete(n));
                    }
                    break;
                }
                Err(_) => {
                    *input = checkpoint;
                    break;
                }
            }
        }
        Ok(items)
    }
}

impl<I: Generate, Sep: Generate> Generate for SeparatedBy<I, Sep> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        let count = self.min + gen.below(3);
        for i in 0..count {
            if i > 0 {
                self.sep.generate_into(gen, out);
            }
            self.item.generate_into(gen, out);
        }
    }
}

/// Zero or more `item`s separated by `sep` (no trailing separator).
pub fn separated_by<I, Sep>(item: I, sep: Sep) -> SeparatedBy<I, Sep> {
    SeparatedBy { item, sep, min: 0 }
}

/// Like [`separated_by`], but requires at least one item.
pub fn separated_by1<I, Sep>(item: I, sep: Sep) -> SeparatedBy<I, Sep> {
    SeparatedBy { item, sep, min: 1 }
}

/// [`repeated_until`].
pub struct RepeatedUntil<P, E> {
    parser: P,
    end: E,
}

impl<S: Stream, P: Parser<S>, E: Parser<S>> Parser<S> for RepeatedUntil<P, E> {
    type Output = Vec<P::Output>;
    fn parse_next(&self, input: &mut Input<S>) -> PResult<S, Vec<P::Output>> {
        let mut items = Vec::new();
        loop {
            let checkpoint = *input;
            if self.end.parse_next(input).is_ok() {
                *input = checkpoint;
                break;
            }
            *input = checkpoint;
            match self.parser.parse_next(input) {
                Ok(item) => {
                    if input.stream.len() == checkpoint.stream.len() {
                        *input = checkpoint;
                        break;
                    }
                    items.push(item);
                }
                Err(ParseError::Incomplete(n)) => {
                    *input = checkpoint;
                    if S::PARTIAL {
                        return Err(ParseError::Incomplete(n));
                    }
                    break;
                }
                Err(_) => {
                    *input = checkpoint;
                    break;
                }
            }
        }
        Ok(items)
    }
}

impl<P: Generate, E> Generate for RepeatedUntil<P, E> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        for _ in 0..gen.below(3) {
            self.parser.generate_into(gen, out);
        }
    }
}

/// Repeats `parser` until `end` would match (not consumed), yielding
/// `Vec<parser::Output>`. Stops if `parser` fails.
pub fn repeated_until<P, E>(parser: P, end: E) -> RepeatedUntil<P, E> {
    RepeatedUntil { parser, end }
}

/// [`empty`].
pub struct Empty<S: Stream> {
    _phantom: PhantomData<fn(S)>,
}

impl<S: Stream> Parser<S> for Empty<S> {
    type Output = ();
    fn parse_next(&self, _input: &mut Input<S>) -> PResult<S, ()> {
        Ok(())
    }
}

impl<S: Stream> Generate for Empty<S> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

/// Always succeeds without consuming input, yielding `()`.
pub fn empty<S: Stream>() -> Empty<S> {
    Empty {
        _phantom: PhantomData,
    }
}

// ── NimbleParsec terminology module ──────────────────────────────────────────

/// NimbleParsec-terminology aliases over the idiomatic core, for readers
/// porting from Elixir. `use nimble_parsec_rs::nimble::*` gives the
/// NimbleParsec vocabulary as free functions.
pub mod nimble {
    use super::{
        eof, literal, not, Eof, Ignored, Labelled, Literal, Map, Not, Opt, PostTraverse,
        PreTraverse, Repeated, Then, To, WithByteOffset, WithLine,
    };
    use crate::Stream;

    pub use super::{
        any, bit_bool, bits, byte_aligned, bytes, choice, digits, empty, eventually, integer,
        lookahead, recursive, satisfy, take_bits, take_while, BitOutput, Parser,
    };
    pub use crate::{BitSlice, Bits};

    /// NimbleParsec name for [`literal`](super::literal).
    pub fn string<S: Stream + crate::Compare<&'static str>>(
        lit: &'static str,
    ) -> Literal<S, &'static str> {
        literal(lit)
    }

    /// NimbleParsec name for [`eof`](super::eof).
    pub fn eos<S: Stream>() -> Eof<S> {
        eof()
    }

    /// NimbleParsec `concat` — sequence two parsers.
    pub fn concat<S: Stream, A: Parser<S>, B: Parser<S>>(first: A, second: B) -> Then<A, B> {
        first.then(second)
    }

    /// NimbleParsec `optional`.
    pub fn optional<S: Stream, P: Parser<S>>(parser: P) -> Opt<P> {
        parser.optional()
    }

    /// NimbleParsec `repeat`.
    pub fn repeat<S: Stream, P: Parser<S>>(parser: P) -> Repeated<P> {
        parser.repeated()
    }

    /// NimbleParsec `times` / `duplicate` — repeat exactly `n` times.
    pub fn times<S: Stream, P: Parser<S>>(parser: P, n: usize) -> Repeated<P> {
        parser.repeated_in(n, n)
    }

    /// NimbleParsec `duplicate` — repeat exactly `n` times.
    pub fn duplicate<S: Stream, P: Parser<S>>(parser: P, n: usize) -> Repeated<P> {
        parser.repeated_in(n, n)
    }

    /// NimbleParsec `ignore`.
    pub fn ignore<S: Stream, P: Parser<S>>(parser: P) -> Ignored<P> {
        parser.ignored()
    }

    /// NimbleParsec `replace`.
    pub fn replace<S: Stream, P: Parser<S>, V: Clone>(parser: P, value: V) -> To<P, V> {
        parser.to(value)
    }

    /// NimbleParsec `map`.
    pub fn map<S: Stream, P: Parser<S>, F: Fn(P::Output) -> U, U>(parser: P, f: F) -> Map<P, F> {
        parser.map(f)
    }

    /// NimbleParsec `label`.
    pub fn label<S: Stream, P: Parser<S>>(parser: P, label: &'static str) -> Labelled<P> {
        parser.labelled(label)
    }

    /// NimbleParsec `lookahead_not`.
    pub fn lookahead_not<P>(parser: P) -> Not<P> {
        not(parser)
    }

    /// NimbleParsec `byte_offset`.
    pub fn byte_offset<S: Stream, P: Parser<S>>(parser: P) -> WithByteOffset<P> {
        parser.with_byte_offset()
    }

    /// NimbleParsec `line`.
    pub fn line<S: Stream, P: Parser<S>>(parser: P) -> WithLine<P> {
        parser.with_line()
    }

    /// NimbleParsec `debug`.
    pub fn debug<S: Stream, P: Parser<S>>(parser: P, label: &'static str) -> super::Debug<P> {
        parser.debug(label)
    }

    /// NimbleParsec `post_traverse`.
    pub fn post_traverse<S: Stream, P: Parser<S>, F, U>(parser: P, f: F) -> PostTraverse<P, F>
    where
        F: Fn(P::Output, super::Cursor) -> Result<U, String>,
    {
        parser.post_traverse(f)
    }

    /// NimbleParsec `pre_traverse`.
    pub fn pre_traverse<S: Stream, P: Parser<S>, F, U>(parser: P, f: F) -> PreTraverse<P, F>
    where
        F: Fn(P::Output, super::Cursor) -> Result<U, String>,
    {
        parser.pre_traverse(f)
    }
}
