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

    /// Runs the parser over the whole `text`, requiring all input to be consumed.
    fn parse(&self, text: &'i str) -> PResult<'i, Self::Output>
    where
        Self: Sized,
    {
        let prev = crate::RECURSION_BUDGET.with(|b| b.replace(DEFAULT_MAX_RECURSION_DEPTH));
        let mut input = Input::new(text);
        let result = self.parse_next(&mut input);
        crate::RECURSION_BUDGET.with(|b| b.set(prev));
        let output = result?;
        if input.rest.is_empty() {
            Ok(output)
        } else {
            Err(ParseFailure::expecting(
                "expected end of input",
                input.rest,
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
        let prev = crate::RECURSION_BUDGET.with(|b| b.replace(DEFAULT_MAX_RECURSION_DEPTH));
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
        }
    }

    /// Repeats `self` at least `min` times, collecting the outputs.
    fn repeated_at_least(self, min: usize) -> Repeated<Self>
    where
        Self: Sized,
    {
        Repeated { inner: self, min }
    }

    /// Overrides the failure message (and the structured expectation) with `label`.
    fn labelled(self, label: &'static str) -> Labelled<Self>
    where
        Self: Sized,
    {
        Labelled { inner: self, label }
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

/// [`Parser::repeated`] / [`Parser::repeated_at_least`].
pub struct Repeated<P> {
    inner: P,
    min: usize,
}

impl<'i, P: Parser<'i>> Parser<'i> for Repeated<P> {
    type Output = Vec<P::Output>;
    fn parse_next(&self, input: &mut Input<'i>) -> PResult<'i, Vec<P::Output>> {
        let mut out = Vec::new();
        loop {
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
