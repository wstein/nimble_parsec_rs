//! Typed parser combinators — RFC 0001, phase 2: the typed `Parser<Output>` core.
//!
//! A [`Parser`] is generic over its `Output`, so combinators compose and
//! type-check at compile time with no runtime `Value` tagging: [`literal`]
//! yields `&str`, [`Parser::then`] yields a tuple, [`Parser::repeated`] yields a
//! `Vec`, and a `.map` threads any output type through. This is the foundation
//! of the planned typed surface (see `docs/rfcs/0001-typed-parser.md`); it lives
//! beside the `Value`-based API during the migration and reuses the same
//! structured [`ParseFailure`] and recursion-depth cap.
//!
//! ```
//! use nimble_parsec_rs::typed::{digits, literal, Parser};
//!
//! // "(" digits ")", keeping only the digits, parsed as a number.
//! let inner = literal("(")
//!     .ignore_then(digits())
//!     .then_ignore(literal(")"))
//!     .map(|ds: &str| ds.parse::<u32>().unwrap());
//! assert_eq!(inner.parse("(42)").unwrap(), 42);
//! ```

use std::cell::OnceCell;
use std::rc::Rc;

use crate::{Cursor, ParseFailure, DEFAULT_MAX_RECURSION_DEPTH};

/// The result of a typed parse step.
pub type PResult<'i, O> = Result<O, ParseFailure<'i>>;

/// Parser input: the unconsumed text and the current position. It is `Copy`, so
/// a combinator snapshots it before a fallible attempt and restores it on
/// failure — the backtracking primitive the alternation/repetition combinators
/// rely on.
#[derive(Clone, Copy, Debug)]
pub struct Input<'i> {
    rest: &'i str,
    cursor: Cursor,
}

impl<'i> Input<'i> {
    /// Wraps `text`, positioned at its start.
    pub fn new(text: &'i str) -> Self {
        Self {
            rest: text,
            cursor: Cursor::default(),
        }
    }

    /// The still-unconsumed input.
    pub fn rest(&self) -> &'i str {
        self.rest
    }

    /// The current position.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Advances past `consumed`, which must be the current prefix of [`rest`](Self::rest).
    fn bump(&mut self, consumed: &str) {
        self.cursor = crate::advance(self.cursor, consumed);
        self.rest = &self.rest[consumed.len()..];
    }
}

/// A parser producing a typed `Output` from [`Input`].
///
/// Implement [`parse_next`](Parser::parse_next) (advance the input, or return a
/// [`ParseFailure`] leaving it for the caller to restore); the composition
/// methods are provided. The leaf constructors ([`literal`], [`any`],
/// [`satisfy`], …) and combinators return named zero-cost types, so grammars are
/// fully monomorphized.
pub trait Parser<'i> {
    /// The value this parser produces on success.
    type Output;

    /// Parses from the front of `input`, advancing it past what was consumed. On
    /// failure the input is left as the implementation chose to leave it;
    /// backtracking combinators snapshot and restore it themselves.
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Self::Output>;

    /// Runs the parser over the whole `text`, requiring all input to be consumed,
    /// with the default recursion cap ([`DEFAULT_MAX_RECURSION_DEPTH`]).
    fn parse(&self, text: &'i str) -> PResult<'i, Self::Output>
    where
        Self: Sized,
    {
        self.parse_with_max_depth(text, DEFAULT_MAX_RECURSION_DEPTH)
    }

    /// Like [`parse`](Parser::parse), but caps recursion depth at `max_depth`.
    fn parse_with_max_depth(&self, text: &'i str, max_depth: usize) -> PResult<'i, Self::Output>
    where
        Self: Sized,
    {
        let (output, rest) = self.parse_partial_with_max_depth(text, max_depth)?;
        if rest.is_empty() {
            Ok(output)
        } else {
            // Re-derive the position of the leftover for the error.
            let mut input = Input::new(text);
            input.bump(&text[..text.len() - rest.len()]);
            Err(ParseFailure::expecting(
                "expected end of input",
                rest,
                input.cursor,
            ))
        }
    }

    /// Runs the parser, returning the output and the unconsumed remainder rather
    /// than requiring all input to be consumed.
    fn parse_partial(&self, text: &'i str) -> Result<(Self::Output, &'i str), ParseFailure<'i>>
    where
        Self: Sized,
    {
        self.parse_partial_with_max_depth(text, DEFAULT_MAX_RECURSION_DEPTH)
    }

    /// Like [`parse_partial`](Parser::parse_partial), but caps recursion depth at
    /// `max_depth`.
    fn parse_partial_with_max_depth(
        &self,
        text: &'i str,
        max_depth: usize,
    ) -> Result<(Self::Output, &'i str), ParseFailure<'i>>
    where
        Self: Sized,
    {
        // Set the budget for this parse and restore the previous one on return,
        // so a `parse` invoked from within a transform closure is re-entrant.
        let prev = crate::RECURSION_BUDGET.with(|b| b.replace(max_depth));
        let mut input = Input::new(text);
        let result = self.parse_next(&mut input);
        crate::RECURSION_BUDGET.with(|b| b.set(prev));
        Ok((result?, input.rest))
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
        P: Parser<'i>,
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
        P: Parser<'i>,
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
        P: Parser<'i>,
    {
        ThenIgnore {
            first: self,
            second: next,
        }
    }

    /// Tries `self`; if it fails (consuming nothing), tries `alt`. Both branches
    /// must produce the same output type.
    fn or<P>(self, alt: P) -> Or<Self, P>
    where
        Self: Sized,
        P: Parser<'i, Output = Self::Output>,
    {
        Or { a: self, b: alt }
    }

    /// Makes `self` optional, yielding `None` (and consuming nothing) on failure.
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

    /// Repeats `self` at least `min` times, collecting the outputs.
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

    /// Repeats `self` between `min` and `max` times (inclusive), collecting the
    /// outputs.
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

    /// Replaces the output with a clone of `value` (NimbleParsec's `replace`).
    fn to<V: Clone>(self, value: V) -> To<Self, V>
    where
        Self: Sized,
    {
        To { inner: self, value }
    }

    /// Repeats `self` zero or more times, folding the outputs into an accumulator
    /// seeded by `init` (NimbleParsec's `reduce`, done without the intermediate
    /// `Vec` that `self.repeated().map(…)` would allocate).
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

    /// Transforms the output with a fallible `f`; returning `Err(message)` fails
    /// the parse (NimbleParsec's validating `post_traverse`).
    fn try_map<U, F>(self, f: F) -> TryMap<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output) -> Result<U, String>,
    {
        TryMap { inner: self, f }
    }

    /// Overrides the failure message (and the structured expectation) with `label`.
    fn labelled(self, label: &'static str) -> Labelled<Self>
    where
        Self: Sized,
    {
        Labelled { inner: self, label }
    }

    /// Pairs the output with the byte offset reached after the match
    /// (NimbleParsec's `byte_offset`).
    fn with_byte_offset(self) -> WithByteOffset<Self>
    where
        Self: Sized,
    {
        WithByteOffset { inner: self }
    }

    /// Pairs the output with the position reached after the match — `(1-based
    /// line, byte offset of the start of that line)` (NimbleParsec's `line`).
    fn with_line(self) -> WithLine<Self>
    where
        Self: Sized,
    {
        WithLine { inner: self }
    }

    /// Traces this parser to stderr (the position on entry and the outcome on
    /// exit), passing the output through unchanged (NimbleParsec's `debug`).
    /// `label` identifies the parser in the trace.
    fn debug(self, label: &'static str) -> Debug<Self>
    where
        Self: Sized,
    {
        Debug { inner: self, label }
    }

    /// Transforms the output with `f`, which also receives the [`Cursor`]
    /// **after** the match and may fail the parse by returning `Err(message)`
    /// (NimbleParsec's `post_traverse`).
    ///
    /// User *context* (a symbol table, counters, …) is threaded by capturing
    /// interior-mutable state (`Cell` / `RefCell`) in `f`: the same captured
    /// state is shared across every invocation and across sibling combinators,
    /// enabling context-dependent parsing. (As in winnow, captured state is not
    /// rolled back when an enclosing `or` / `optional` backtracks.)
    fn post_traverse<U, F>(self, f: F) -> PostTraverse<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output, Cursor) -> Result<U, String>,
    {
        PostTraverse { inner: self, f }
    }

    /// Like [`post_traverse`](Parser::post_traverse), but `f` receives the
    /// [`Cursor`] from **before** the match — handy for tagging a result with
    /// its start position (NimbleParsec's `pre_traverse`).
    fn pre_traverse<U, F>(self, f: F) -> PreTraverse<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Output, Cursor) -> Result<U, String>,
    {
        PreTraverse { inner: self, f }
    }
}

// ── Combinators ──────────────────────────────────────────────────────────────

/// [`Parser::map`].
pub struct Map<P, F> {
    inner: P,
    f: F,
}

impl<'i, P, F, U> Parser<'i> for Map<P, F>
where
    P: Parser<'i>,
    F: Fn(P::Output) -> U,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, U> {
        let out = self.inner.parse_next(input)?;
        Ok((self.f)(out))
    }
}

/// [`Parser::ignored`].
pub struct Ignored<P> {
    inner: P,
}

impl<'i, P: Parser<'i>> Parser<'i> for Ignored<P> {
    type Output = ();
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, ()> {
        self.inner.parse_next(input)?;
        Ok(())
    }
}

/// [`Parser::then`].
pub struct Then<A, B> {
    first: A,
    second: B,
}

impl<'i, A: Parser<'i>, B: Parser<'i>> Parser<'i> for Then<A, B> {
    type Output = (A::Output, B::Output);
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, (A::Output, B::Output)> {
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

impl<'i, A: Parser<'i>, B: Parser<'i>> Parser<'i> for IgnoreThen<A, B> {
    type Output = B::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, B::Output> {
        self.first.parse_next(input)?;
        self.second.parse_next(input)
    }
}

/// [`Parser::then_ignore`].
pub struct ThenIgnore<A, B> {
    first: A,
    second: B,
}

impl<'i, A: Parser<'i>, B: Parser<'i>> Parser<'i> for ThenIgnore<A, B> {
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, A::Output> {
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

impl<'i, A, B> Parser<'i> for Or<A, B>
where
    A: Parser<'i>,
    B: Parser<'i, Output = A::Output>,
{
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, A::Output> {
        let start = *input;
        match self.a.parse_next(input) {
            Ok(out) => Ok(out),
            Err(first) => {
                *input = start;
                match self.b.parse_next(input) {
                    Ok(out) => Ok(out),
                    Err(second) => {
                        *input = start;
                        // Union the branches' expectations, join their messages.
                        let mut expected = first.expected;
                        expected.extend(second.expected);
                        Err(ParseFailure {
                            reason: format!("{} or {}", first.reason, second.reason),
                            expected,
                            rest: start.rest,
                            cursor: start.cursor,
                        })
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

impl<'i, P: Parser<'i>> Parser<'i> for Opt<P> {
    type Output = Option<P::Output>;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Option<P::Output>> {
        let start = *input;
        match self.inner.parse_next(input) {
            Ok(out) => Ok(Some(out)),
            Err(_) => {
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

impl<'i, P: Parser<'i>> Parser<'i> for Repeated<P> {
    type Output = Vec<P::Output>;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Vec<P::Output>> {
        let mut out = Vec::new();
        loop {
            if self.max.is_some_and(|max| out.len() >= max) {
                break;
            }
            let start = *input;
            match self.inner.parse_next(input) {
                Ok(item) => {
                    if input.rest.len() == start.rest.len() {
                        // A non-advancing match would loop forever: drop it and stop.
                        *input = start;
                        break;
                    }
                    out.push(item);
                }
                Err(err) => {
                    *input = start;
                    if out.len() < self.min {
                        return Err(err);
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

impl<'i, P: Parser<'i>, V: Clone> Parser<'i> for To<P, V> {
    type Output = V;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, V> {
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

impl<'i, P, I, F, A> Parser<'i> for Fold<P, I, F>
where
    P: Parser<'i>,
    I: Fn() -> A,
    F: Fn(A, P::Output) -> A,
{
    type Output = A;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, A> {
        let mut acc = (self.init)();
        loop {
            let checkpoint = *input;
            match self.inner.parse_next(input) {
                Ok(item) => {
                    if input.rest.len() == checkpoint.rest.len() {
                        *input = checkpoint; // non-advancing match: stop, don't loop
                        break;
                    }
                    acc = (self.f)(acc, item);
                }
                Err(_) => {
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

impl<'i, P, F, U> Parser<'i> for TryMap<P, F>
where
    P: Parser<'i>,
    F: Fn(P::Output) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, U> {
        let out = self.inner.parse_next(input)?;
        (self.f)(out).map_err(|message| ParseFailure::rejected(message, input.rest, input.cursor))
    }
}

/// [`lookahead`].
pub struct Lookahead<P> {
    inner: P,
}

impl<'i, P: Parser<'i>> Parser<'i> for Lookahead<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, P::Output> {
        // Zero-width: run the inner parser, then restore the input position.
        let start = *input;
        let out = self.inner.parse_next(input);
        *input = start;
        out
    }
}

/// Succeeds with `parser`'s output **without consuming input** (positive
/// lookahead), or fails if `parser` fails.
pub fn lookahead<'i, P: Parser<'i>>(parser: P) -> Lookahead<P> {
    Lookahead { inner: parser }
}

/// [`not`].
pub struct Not<P> {
    inner: P,
}

impl<'i, P: Parser<'i>> Parser<'i> for Not<P> {
    type Output = ();
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, ()> {
        let start = *input;
        let matched = self.inner.parse_next(input).is_ok();
        *input = start;
        if matched {
            Err(ParseFailure::rejected(
                "did not expect the lookahead parser to match",
                start.rest,
                start.cursor,
            ))
        } else {
            Ok(())
        }
    }
}

/// Succeeds (consuming nothing) only if `parser` fails — negative lookahead, the
/// typed analogue of `lookahead_not`.
pub fn not<'i, P: Parser<'i>>(parser: P) -> Not<P> {
    Not { inner: parser }
}

/// [`Parser::labelled`].
pub struct Labelled<P> {
    inner: P,
    label: &'static str,
}

impl<'i, P: Parser<'i>> Parser<'i> for Labelled<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, P::Output> {
        self.inner
            .parse_next(input)
            .map_err(|err| ParseFailure::expecting(self.label, err.rest, err.cursor))
    }
}

/// [`Parser::with_byte_offset`].
pub struct WithByteOffset<P> {
    inner: P,
}

impl<'i, P: Parser<'i>> Parser<'i> for WithByteOffset<P> {
    type Output = (P::Output, usize);
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, (P::Output, usize)> {
        let out = self.inner.parse_next(input)?;
        Ok((out, input.cursor.byte_offset))
    }
}

/// [`Parser::with_line`].
pub struct WithLine<P> {
    inner: P,
}

impl<'i, P: Parser<'i>> Parser<'i> for WithLine<P> {
    type Output = (P::Output, (usize, usize));
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, (P::Output, (usize, usize))> {
        let out = self.inner.parse_next(input)?;
        Ok((out, (input.cursor.line, input.cursor.line_start_offset)))
    }
}

/// [`Parser::debug`].
pub struct Debug<P> {
    inner: P,
    label: &'static str,
}

impl<'i, P: Parser<'i>> Parser<'i> for Debug<P> {
    type Output = P::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, P::Output> {
        let before = input.cursor;
        let preview: String = input.rest.chars().take(24).collect();
        eprintln!(
            "[nimble_parsec_rs] {}: enter at line {}, byte {} — rest {preview:?}",
            self.label, before.line, before.byte_offset
        );
        let result = self.inner.parse_next(input);
        match &result {
            Ok(_) => eprintln!(
                "[nimble_parsec_rs] {}: ok, now at byte {}",
                self.label, input.cursor.byte_offset
            ),
            Err(err) => eprintln!("[nimble_parsec_rs] {}: failed — {}", self.label, err.reason),
        }
        result
    }
}

/// [`Parser::post_traverse`].
pub struct PostTraverse<P, F> {
    inner: P,
    f: F,
}

impl<'i, P, F, U> Parser<'i> for PostTraverse<P, F>
where
    P: Parser<'i>,
    F: Fn(P::Output, Cursor) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, U> {
        let out = self.inner.parse_next(input)?;
        (self.f)(out, input.cursor)
            .map_err(|message| ParseFailure::rejected(message, input.rest, input.cursor))
    }
}

/// [`Parser::pre_traverse`].
pub struct PreTraverse<P, F> {
    inner: P,
    f: F,
}

impl<'i, P, F, U> Parser<'i> for PreTraverse<P, F>
where
    P: Parser<'i>,
    F: Fn(P::Output, Cursor) -> Result<U, String>,
{
    type Output = U;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, U> {
        let before = input.cursor;
        let out = self.inner.parse_next(input)?;
        (self.f)(out, before)
            .map_err(|message| ParseFailure::rejected(message, input.rest, input.cursor))
    }
}

// ── Leaves ───────────────────────────────────────────────────────────────────

/// [`literal`].
pub struct Literal {
    lit: &'static str,
}

impl<'i> Parser<'i> for Literal {
    type Output = &'i str;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, &'i str> {
        let rest = input.rest;
        if rest.starts_with(self.lit) {
            let consumed = &rest[..self.lit.len()];
            input.bump(consumed);
            Ok(consumed)
        } else {
            Err(ParseFailure::expecting(
                format!("expected {:?}", self.lit),
                rest,
                input.cursor,
            ))
        }
    }
}

/// Matches the exact string `lit`, yielding the consumed slice.
pub fn literal(lit: &'static str) -> Literal {
    Literal { lit }
}

/// [`any`].
pub struct AnyChar;

impl<'i> Parser<'i> for AnyChar {
    type Output = char;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, char> {
        let rest = input.rest;
        match rest.chars().next() {
            Some(c) => {
                input.bump(&rest[..c.len_utf8()]);
                Ok(c)
            }
            None => Err(ParseFailure::expecting(
                "expected any character",
                rest,
                input.cursor,
            )),
        }
    }
}

/// Matches any single character.
pub fn any() -> AnyChar {
    AnyChar
}

/// [`satisfy`].
pub struct Satisfy<F> {
    pred: F,
    label: &'static str,
}

impl<'i, F: Fn(char) -> bool> Parser<'i> for Satisfy<F> {
    type Output = char;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, char> {
        let rest = input.rest;
        match rest.chars().next() {
            Some(c) if (self.pred)(c) => {
                input.bump(&rest[..c.len_utf8()]);
                Ok(c)
            }
            _ => Err(ParseFailure::expecting(self.label, rest, input.cursor)),
        }
    }
}

/// Matches a single character satisfying `pred`; `label` describes it on failure.
pub fn satisfy<F: Fn(char) -> bool>(label: &'static str, pred: F) -> Satisfy<F> {
    Satisfy { pred, label }
}

/// [`take_while`] / [`take_while1`].
pub struct TakeWhile<F> {
    pred: F,
    min: usize,
}

impl<'i, F: Fn(char) -> bool> Parser<'i> for TakeWhile<F> {
    type Output = &'i str;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, &'i str> {
        let rest = input.rest;
        let mut end = 0;
        let mut count = 0;
        for (idx, c) in rest.char_indices() {
            if (self.pred)(c) {
                end = idx + c.len_utf8();
                count += 1;
            } else {
                break;
            }
        }
        if count < self.min {
            return Err(ParseFailure::expecting(
                "expected at least one matching character",
                rest,
                input.cursor,
            ));
        }
        let consumed = &rest[..end];
        input.bump(consumed);
        Ok(consumed)
    }
}

/// Consumes the maximal run of characters satisfying `pred` (possibly empty),
/// yielding the consumed slice.
pub fn take_while<F: Fn(char) -> bool>(pred: F) -> TakeWhile<F> {
    TakeWhile { pred, min: 0 }
}

/// Like [`take_while`], but requires at least one character.
pub fn take_while1<F: Fn(char) -> bool>(pred: F) -> TakeWhile<F> {
    TakeWhile { pred, min: 1 }
}

/// A run of one or more ASCII digits, yielding the consumed slice.
pub fn digits() -> TakeWhile<fn(char) -> bool> {
    TakeWhile {
        pred: |c: char| c.is_ascii_digit(),
        min: 1,
    }
}

/// [`eof`].
pub struct Eof;

impl<'i> Parser<'i> for Eof {
    type Output = ();
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, ()> {
        if input.rest.is_empty() {
            Ok(())
        } else {
            Err(ParseFailure::expecting(
                "expected end of input",
                input.rest,
                input.cursor,
            ))
        }
    }
}

/// Matches only at the end of input.
pub fn eof() -> Eof {
    Eof
}

/// Matches a single character contained in `set`.
pub fn one_of(set: &'static str) -> Satisfy<impl Fn(char) -> bool> {
    satisfy("one of an expected set", move |c| set.contains(c))
}

/// Matches a single character **not** contained in `set`.
pub fn none_of(set: &'static str) -> Satisfy<impl Fn(char) -> bool> {
    satisfy("a character outside an excluded set", move |c| {
        !set.contains(c)
    })
}

/// A set of alternatives for [`choice`] — implemented for arrays `[P; N]`
/// (same parser type) and for tuples `(A, B, …)` up to arity 8 (different parser
/// types, one shared `Output`).
pub trait Alternatives<'i> {
    /// The shared output type of every alternative.
    type Output;
    /// Tries each alternative in order, returning the first success or a failure
    /// unioning the alternatives' expectations.
    fn choice_parse(&self, input: &mut Input<'i>) -> PResult<'i, Self::Output>;
}

fn choice_failure<'i>(
    reasons: Vec<String>,
    expected: Vec<String>,
    at: Input<'i>,
) -> ParseFailure<'i> {
    ParseFailure {
        reason: if reasons.is_empty() {
            "choice has no options".to_string()
        } else {
            reasons.join(" or ")
        },
        expected,
        rest: at.rest,
        cursor: at.cursor,
    }
}

impl<'i, P: Parser<'i>, const N: usize> Alternatives<'i> for [P; N] {
    type Output = P::Output;
    fn choice_parse(&self, input: &mut Input<'i>) -> PResult<'i, P::Output> {
        let start = *input;
        let mut reasons = Vec::with_capacity(N);
        let mut expected = Vec::new();
        for parser in self {
            match parser.parse_next(input) {
                Ok(out) => return Ok(out),
                Err(err) => {
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
        impl<'i, O, $($param: Parser<'i, Output = O>),+> Alternatives<'i> for ($($param,)+) {
            type Output = O;
            fn choice_parse(&self, input: &mut Input<'i>) -> PResult<'i, O> {
                let start = *input;
                let mut reasons = Vec::new();
                let mut expected = Vec::new();
                $(
                    match self.$idx.parse_next(input) {
                        Ok(out) => return Ok(out),
                        Err(err) => {
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

impl<'i, A: Alternatives<'i>> Parser<'i> for ChoiceOf<A> {
    type Output = A::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, A::Output> {
        self.alternatives.choice_parse(input)
    }
}

/// Tries each alternative in order, returning the first success. Accepts an
/// array `[p; N]` (same parser type) or a tuple `(a, b, …)` up to arity 8
/// (different parser types sharing one `Output`).
pub fn choice<'i, A: Alternatives<'i>>(alternatives: A) -> ChoiceOf<A> {
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

// ── Convenience combinators ──────────────────────────────────────────────────

/// [`delimited`].
pub struct Delimited<A, B, C> {
    open: A,
    content: B,
    close: C,
}

impl<'i, A: Parser<'i>, B: Parser<'i>, C: Parser<'i>> Parser<'i> for Delimited<A, B, C> {
    type Output = B::Output;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, B::Output> {
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
/// Sugar for `open.ignore_then(content).then_ignore(close)`.
pub fn delimited<'i, A, B, C>(open: A, content: B, close: C) -> Delimited<A, B, C>
where
    A: Parser<'i>,
    B: Parser<'i>,
    C: Parser<'i>,
{
    Delimited {
        open,
        content,
        close,
    }
}

/// [`separated_by`] / [`separated_by1`].
pub struct SeparatedBy<I, S> {
    item: I,
    sep: S,
    min: usize,
}

impl<'i, I: Parser<'i>, S: Parser<'i>> Parser<'i> for SeparatedBy<I, S> {
    type Output = Vec<I::Output>;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Vec<I::Output>> {
        let mut items = Vec::new();
        let start = *input;
        match self.item.parse_next(input) {
            Ok(first) => items.push(first),
            Err(err) => {
                *input = start;
                if self.min == 0 {
                    return Ok(items);
                }
                return Err(err);
            }
        }
        loop {
            // A separator that isn't followed by an item is not consumed (no
            // trailing separator), so restore to before it and stop.
            let checkpoint = *input;
            if self.sep.parse_next(input).is_err() {
                *input = checkpoint;
                break;
            }
            match self.item.parse_next(input) {
                Ok(item) => {
                    if input.rest.len() == checkpoint.rest.len() {
                        *input = checkpoint; // no progress: avoid looping forever
                        break;
                    }
                    items.push(item);
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

impl<I: Generate, S: Generate> Generate for SeparatedBy<I, S> {
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

/// Zero or more `item`s separated by `sep` (no trailing separator), yielding
/// `Vec<item::Output>`.
pub fn separated_by<'i, I, S>(item: I, sep: S) -> SeparatedBy<I, S>
where
    I: Parser<'i>,
    S: Parser<'i>,
{
    SeparatedBy { item, sep, min: 0 }
}

/// Like [`separated_by`], but requires at least one item.
pub fn separated_by1<'i, I, S>(item: I, sep: S) -> SeparatedBy<I, S>
where
    I: Parser<'i>,
    S: Parser<'i>,
{
    SeparatedBy { item, sep, min: 1 }
}

/// [`repeated_until`].
pub struct RepeatedUntil<P, E> {
    parser: P,
    end: E,
}

impl<'i, P: Parser<'i>, E: Parser<'i>> Parser<'i> for RepeatedUntil<P, E> {
    type Output = Vec<P::Output>;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Vec<P::Output>> {
        let mut items = Vec::new();
        loop {
            let checkpoint = *input;
            // Stop when the terminator matches — without consuming it.
            if self.end.parse_next(input).is_ok() {
                *input = checkpoint;
                break;
            }
            *input = checkpoint;
            match self.parser.parse_next(input) {
                Ok(item) => {
                    if input.rest.len() == checkpoint.rest.len() {
                        *input = checkpoint;
                        break;
                    }
                    items.push(item);
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
        // The terminator is the following parser's job, not ours.
        for _ in 0..gen.below(3) {
            self.parser.generate_into(gen, out);
        }
    }
}

/// Repeats `parser` until `end` would match (the terminator is **not**
/// consumed), yielding `Vec<parser::Output>`. Sugar for the
/// `not(end).ignore_then(parser).repeated()` idiom; also stops if `parser` fails.
pub fn repeated_until<'i, P, E>(parser: P, end: E) -> RepeatedUntil<P, E>
where
    P: Parser<'i>,
    E: Parser<'i>,
{
    RepeatedUntil { parser, end }
}

// ── Recursion ──────────────────────────────────────────────────────────────

/// A forward-declared, self-referential parser, enabling recursive grammars
/// (the typed analogue of [`crate::recursive`]). Built by [`recursive`]; cloning
/// shares the same definition. Recursion depth is bounded by the crate-wide cap
/// ([`crate::DEFAULT_MAX_RECURSION_DEPTH`]), returning a [`ParseFailure`] rather
/// than overflowing the stack.
pub struct Recursive<'i, O> {
    cell: Rc<OnceCell<Box<dyn Parser<'i, Output = O> + 'i>>>,
}

impl<'i, O> Clone for Recursive<'i, O> {
    fn clone(&self) -> Self {
        Self {
            cell: Rc::clone(&self.cell),
        }
    }
}

impl<'i, O> Parser<'i> for Recursive<'i, O> {
    type Output = O;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, O> {
        let parser = self
            .cell
            .get()
            .expect("recursive parser used before it was defined");
        // Spend one unit of the shared recursion budget per reference crossing,
        // failing gracefully at zero instead of overflowing the stack.
        let budget = crate::RECURSION_BUDGET.with(std::cell::Cell::get);
        if budget == 0 {
            return Err(ParseFailure::rejected(
                "maximum recursion depth exceeded",
                input.rest,
                input.cursor,
            ));
        }
        crate::RECURSION_BUDGET.with(|b| b.set(budget - 1));
        let result = parser.parse_next(input);
        crate::RECURSION_BUDGET.with(|b| b.set(budget));
        result
    }
}

/// Builds a recursive parser. `build` receives a handle usable within the
/// definition it returns, mirroring [`crate::recursive`].
pub fn recursive<'i, O, P, F>(build: F) -> Recursive<'i, O>
where
    P: Parser<'i, Output = O> + 'i,
    F: FnOnce(Recursive<'i, O>) -> P,
{
    let handle = Recursive {
        cell: Rc::new(OnceCell::new()),
    };
    let definition = build(handle.clone());
    let _ = handle.cell.set(Box::new(definition));
    handle
}

// ── Generation ───────────────────────────────────────────────────────────────

/// Deterministic source of randomness for [`generate`]. Opaque; seeded by
/// `generate`'s `seed`. Implementations of [`Generate`] thread one of these to
/// sample alternatives, repetition counts, and characters.
pub struct Gen {
    state: u64,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Gen {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    // splitmix64 — a tiny, dependency-free PRNG, ample for sampling grammars.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (0 when `n == 0`).
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    // A character satisfying `pred`, sampled from a printable pool (then a wider
    // ASCII scan). Best-effort: a predicate matching no printable ASCII yields a
    // fallback that may not round-trip.
    fn char_matching(&mut self, pred: &dyn Fn(char) -> bool) -> char {
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

/// Produces a random input that the parser accepts, mirroring NimbleParsec's
/// `generate`. Implemented for the built-in combinators except [`Recursive`], so
/// [`generate`] type-checks only for **non-recursive** grammars.
///
/// Generation is best-effort: it round-trips for grammars built from literals,
/// sequencing, alternation, optionality, and repetition, but negative assertions
/// ([`not`]) and predicates that exclude printable ASCII may yield input the
/// parser then rejects.
pub trait Generate {
    /// Appends one sampled instance of this parser's accepted input to `out`.
    fn generate_into(&self, gen: &mut Gen, out: &mut String);
}

/// Generates a random input accepted by `parser`, seeded by `seed` for
/// reproducibility. Only available for non-recursive grammars (see [`Generate`]).
pub fn generate<G: Generate>(parser: &G, seed: u64) -> String {
    let mut gen = Gen::new(seed);
    let mut out = String::new();
    parser.generate_into(&mut gen, &mut out);
    out
}

impl Generate for Literal {
    fn generate_into(&self, _gen: &mut Gen, out: &mut String) {
        out.push_str(self.lit);
    }
}

impl Generate for AnyChar {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        out.push(gen.char_matching(&|_| true));
    }
}

impl<F: Fn(char) -> bool> Generate for Satisfy<F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        out.push(gen.char_matching(&self.pred));
    }
}

impl<F: Fn(char) -> bool> Generate for TakeWhile<F> {
    fn generate_into(&self, gen: &mut Gen, out: &mut String) {
        let count = self.min + gen.below(3);
        for _ in 0..count {
            out.push(gen.char_matching(&self.pred));
        }
    }
}

impl Generate for Eof {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
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

// Zero-width assertions contribute no input — and need no inner `Generate`, so a
// grammar can still be generatable even with a non-generatable assertion inside.
impl<P> Generate for Lookahead<P> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}

impl<P> Generate for Not<P> {
    fn generate_into(&self, _gen: &mut Gen, _out: &mut String) {}
}
