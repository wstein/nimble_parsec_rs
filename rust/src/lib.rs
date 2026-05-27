//! A Rust port of [NimbleParsec](https://github.com/dashbitco/nimble_parsec),
//! a parser-combinator library — with an idiomatic, **typed** surface and
//! **generic input**: `&str`, `&[u8]`, or `Partial<_>` for streaming.
//!
//! Build a parser by composing combinators; each is generic over its output AND
//! its **input stream**, so grammars compose and type-check at compile time.
//! The combinators live in [`typed`] and are re-exported here.
//!
//! ```
//! use nimble_parsec_rs::{digits, literal, Parser};
//!
//! // "(" digits ")" → the number, as a u32.
//! let number = literal("(")
//!     .ignore_then(digits())
//!     .then_ignore(literal(")"))
//!     .map(|ds: &str| ds.parse::<u32>().unwrap());
//!
//! assert_eq!(number.parse("(42)").unwrap(), 42);
//! ```
//!
//! A parse yields the combinator's `Output` or a [`ParseFailure`] carrying a
//! human-readable `reason`, a structured `expected` set, and a [`Cursor`]
//! (line + byte offset). Streaming parses (`Partial<S>`) additionally surface
//! [`Needed`] through [`ParseError::Incomplete`].
#![deny(missing_docs)]

use std::cell::Cell;

pub mod typed;
pub use typed::*;

// ── Cursor ────────────────────────────────────────────────────────────────────

/// Position within the input, tracked as the parse advances. Column is
/// derivable as `byte_offset - line_start_offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// 1-based line number.
    pub line: usize,
    /// Byte offset of the start of the current line.
    pub line_start_offset: usize,
    /// Byte offset from the start of the input.
    pub byte_offset: usize,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            line: 1,
            line_start_offset: 0,
            byte_offset: 0,
        }
    }
}

// ── Stream traits ─────────────────────────────────────────────────────────────

/// Whether this stream may be only a prefix of the full input.
///
/// Non-partial streams (`&str`, `&[u8]`) have `PARTIAL = false`: reaching the
/// end of input is a hard failure. [`Partial<S>`] has `PARTIAL = true`:
/// reaching the end of the current buffer yields [`Needed`] so the caller can
/// append more data and retry from a saved checkpoint.
pub trait StreamIsPartial {
    /// `true` for [`Partial<S>`]; `false` for complete-input streams.
    const PARTIAL: bool;
}

/// A position-trackable, backtrack-capable input source. Implemented on
/// borrowed slices so the input lifetime is carried by `Self` — no GATs, stable
/// Rust. The `Copy` bound makes snapshotting (`let saved = *input`) the
/// backtracking primitive, free of allocation.
///
/// To make the parser accept a custom token sequence, implement this trait (and
/// [`StreamIsPartial`]) for your type.
pub trait Stream: Copy + StreamIsPartial {
    /// One unit from this stream: `char` for text, `u8` for bytes, `T` for
    /// `&[T]` token slices.
    type Token: Copy + PartialEq + core::fmt::Debug;
    /// A borrowed sub-range of this stream — typically the same type as
    /// `Self` for slice types (e.g. `&'i str` when `Self = &'i str`).
    type Slice: Copy + PartialEq + core::fmt::Debug;

    /// Returns the first token and its width in *base units* (bytes for
    /// `char`/`u8`, 1 for `T`), or `None` if the stream is empty.
    fn first(self) -> Option<(Self::Token, usize)>;

    /// Splits the stream at base-unit offset `n`: `(consumed_slice, remainder)`.
    ///
    /// # Panics
    /// Panics if `n > self.len()`, and for `&str` streams if `n` does not
    /// land on a UTF-8 codepoint boundary (same as `str::split_at`). Use
    /// [`Stream::is_valid_split`] to guard before calling.
    fn split_at(self, n: usize) -> (Self::Slice, Self);

    /// Converts the entire remaining stream into a slice. Used to populate
    /// [`ParseFailure::rest`] at the point of failure.
    fn as_slice(self) -> Self::Slice;

    /// The number of remaining base units (bytes for `&str`/`&[u8]`, elements
    /// for `&[T]`).
    fn len(self) -> usize;

    /// `true` when there are no remaining base units.
    fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if `n` is a valid split point. For byte streams this is
    /// always `n <= self.len()`; for `&str` streams it additionally requires
    /// `n` to land on a UTF-8 character boundary.
    fn is_valid_split(self, n: usize) -> bool {
        n <= self.len()
    }

    /// Advances `cursor` past the first `n` base units of this stream, scanning
    /// for newlines so line/column tracking stays accurate.
    fn advance_cursor(self, cursor: Cursor, n: usize) -> Cursor;

    /// A human-readable preview of the first `max_tokens` tokens, for debug
    /// output and error messages.
    fn preview(self, max_tokens: usize) -> String;
}

// ── impl Stream for &str ─────────────────────────────────────────────────────

impl StreamIsPartial for &str {
    const PARTIAL: bool = false;
}

impl<'i> Stream for &'i str {
    type Token = char;
    type Slice = &'i str;

    fn first(self) -> Option<(char, usize)> {
        self.chars().next().map(|c| (c, c.len_utf8()))
    }

    fn split_at(self, n: usize) -> (&'i str, Self) {
        str::split_at(self, n)
    }

    fn as_slice(self) -> &'i str {
        self
    }

    fn len(self) -> usize {
        str::len(self)
    }

    fn is_valid_split(self, n: usize) -> bool {
        n <= self.len() && self.is_char_boundary(n)
    }

    fn advance_cursor(self, cursor: Cursor, n: usize) -> Cursor {
        advance(cursor, &self.as_bytes()[..n])
    }

    fn preview(self, max_tokens: usize) -> String {
        self.chars().take(max_tokens).collect()
    }
}

// ── impl Stream for &[u8] ────────────────────────────────────────────────────

impl StreamIsPartial for &[u8] {
    const PARTIAL: bool = false;
}

impl<'i> Stream for &'i [u8] {
    type Token = u8;
    type Slice = &'i [u8];

    fn first(self) -> Option<(u8, usize)> {
        self.first().map(|&b| (b, 1))
    }

    fn split_at(self, n: usize) -> (&'i [u8], Self) {
        <[u8]>::split_at(self, n)
    }

    fn as_slice(self) -> &'i [u8] {
        self
    }

    fn len(self) -> usize {
        <[u8]>::len(self)
    }

    fn advance_cursor(self, cursor: Cursor, n: usize) -> Cursor {
        advance(cursor, &self[..n])
    }

    fn preview(self, max_tokens: usize) -> String {
        let mut s = String::new();
        for &b in self.iter().take(max_tokens) {
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            } else {
                s.push_str(&format!("\\x{b:02x}"));
            }
        }
        s
    }
}

// ── Partial<S> — streaming wrapper ───────────────────────────────────────────

/// Wraps any complete-input stream to declare it **may be a prefix** of the
/// full data. Token-consuming combinators consult [`StreamIsPartial::PARTIAL`]
/// at compile time: on a `Partial` stream reaching end-of-buffer they return
/// `Err(ParseError::Incomplete(Needed))` instead of a hard failure, so the
/// caller can refill the buffer and retry from a saved checkpoint.
///
/// # Caller loop (winnow-style sliding window)
/// ```text
/// loop {
///     match parser.parse_partial(Partial(&buf[offset..])) {
///         Ok((out, rest))             => { /* consume */ }
///         Err(ParseError::Incomplete) => { /* append more bytes; retry */ }
///         Err(ParseError::Failure(f)) => { /* real error */ break }
///     }
/// }
/// ```
///
/// # Memory note
/// `Partial<&[u8]>` still borrows the whole current buffer — it does not free
/// consumed bytes automatically. True bounded-memory streaming requires the
/// caller to maintain a sliding window and drop already-consumed prefixes.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Partial<S>(pub S);

impl<S: StreamIsPartial> StreamIsPartial for Partial<S> {
    const PARTIAL: bool = true;
}

impl<S: Stream> Stream for Partial<S> {
    type Token = S::Token;
    type Slice = S::Slice;

    fn first(self) -> Option<(S::Token, usize)> {
        self.0.first()
    }

    fn split_at(self, n: usize) -> (S::Slice, Self) {
        let (slice, rest) = self.0.split_at(n);
        (slice, Partial(rest))
    }

    fn as_slice(self) -> S::Slice {
        self.0.as_slice()
    }

    fn len(self) -> usize {
        self.0.len()
    }

    fn is_valid_split(self, n: usize) -> bool {
        self.0.is_valid_split(n)
    }

    fn advance_cursor(self, cursor: Cursor, n: usize) -> Cursor {
        self.0.advance_cursor(cursor, n)
    }

    fn preview(self, max_tokens: usize) -> String {
        self.0.preview(max_tokens)
    }
}

// ── Compare<Pat> — literal matching ──────────────────────────────────────────

/// Matches a fixed pattern against the front of the stream, returning the
/// number of base units consumed on success. Implemented separately for
/// `(&str, &str)` and `(&[u8], &[u8])`, so a `&str` pattern on a byte stream
/// is a **compile-time type error** rather than silent reinterpretation.
pub trait Compare<Pat> {
    /// Returns `Some(n)` if this stream starts with `pat`, where `n` is the
    /// number of base units consumed. Returns `None` on mismatch.
    fn starts_with_pat(self, pat: Pat) -> Option<usize>;
}

impl Compare<&str> for &str {
    fn starts_with_pat(self, pat: &str) -> Option<usize> {
        self.starts_with(pat).then_some(pat.len())
    }
}

impl Compare<&[u8]> for &[u8] {
    fn starts_with_pat(self, pat: &[u8]) -> Option<usize> {
        self.starts_with(pat).then_some(pat.len())
    }
}

impl Compare<u8> for &[u8] {
    fn starts_with_pat(self, pat: u8) -> Option<usize> {
        self.first().filter(|&&b| b == pat).map(|_| 1)
    }
}

impl<S: Stream + Compare<Pat>, Pat: Copy> Compare<Pat> for Partial<S> {
    fn starts_with_pat(self, pat: Pat) -> Option<usize> {
        self.0.starts_with_pat(pat)
    }
}

// ── Streaming error types ─────────────────────────────────────────────────────

/// How much more input a streaming parse needs to make progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needed {
    /// The parser cannot determine the exact amount needed.
    Unknown,
    /// At least this many additional base units are needed.
    Size(core::num::NonZeroUsize),
}

// ── ParseFailure ──────────────────────────────────────────────────────────────

/// A failed parse: why it failed, what was expected, where it failed, and
/// what remained in the input at that point.
///
/// Generic over the stream type `S` so that `rest` carries the right slice
/// type: `&str` for text grammars, `&[u8]` for binary grammars. For existing
/// text-only code, `ParseFailure<&str>` is the drop-in replacement for the
/// former `ParseFailure<'_>`.
pub struct ParseFailure<S: Stream> {
    /// Human-readable failure message.
    pub reason: String,
    /// Structured set of what the parser expected at [`cursor`](Self::cursor).
    pub expected: Vec<String>,
    /// The remaining input at the point of failure.
    pub rest: S::Slice,
    /// Position at the point of failure.
    pub cursor: Cursor,
}

// Manual trait impls to avoid spurious `S: Clone/Debug/PartialEq` bounds.
impl<S: Stream> Clone for ParseFailure<S> {
    fn clone(&self) -> Self {
        ParseFailure {
            reason: self.reason.clone(),
            expected: self.expected.clone(),
            rest: self.rest,
            cursor: self.cursor,
        }
    }
}

impl<S: Stream> core::fmt::Debug for ParseFailure<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ParseFailure")
            .field("reason", &self.reason)
            .field("expected", &self.expected)
            .field("rest", &self.rest)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl<S: Stream> PartialEq for ParseFailure<S> {
    fn eq(&self, other: &Self) -> bool {
        self.reason == other.reason
            && self.expected == other.expected
            && self.rest == other.rest
            && self.cursor == other.cursor
    }
}

impl<S: Stream> ParseFailure<S> {
    /// Builds a failure where the parser expected a specific token or character
    /// class. `what` (e.g. `"expected \"x\""`) becomes both the
    /// [`reason`](Self::reason) and the single entry of [`expected`](Self::expected).
    pub fn expecting(what: impl Into<String>, rest: S::Slice, cursor: Cursor) -> Self {
        let what = what.into();
        ParseFailure {
            reason: what.clone(),
            expected: vec![what],
            rest,
            cursor,
        }
    }

    /// Builds a failure that is not a simple token expectation — a negative
    /// assertion, a semantic/validation rejection, or a structural limit. Its
    /// [`expected`](Self::expected) set is empty.
    pub fn rejected(reason: impl Into<String>, rest: S::Slice, cursor: Cursor) -> Self {
        ParseFailure {
            reason: reason.into(),
            expected: Vec::new(),
            rest,
            cursor,
        }
    }
}

impl<S: Stream> core::fmt::Display for ParseFailure<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} (line {}, byte offset {})",
            self.reason, self.cursor.line, self.cursor.byte_offset
        )
    }
}

impl<S: Stream> std::error::Error for ParseFailure<S> {}

// ── ParseError ────────────────────────────────────────────────────────────────

/// The error type for a streaming or complete-input parse step. Complete-input
/// parses (non-`Partial` streams) will never produce [`Incomplete`](ParseError::Incomplete);
/// the [`Parser::parse`] method converts it to a [`ParseFailure`] for you.
pub enum ParseError<S: Stream> {
    /// A hard parse failure (wrong token, failed predicate, end of complete input, …).
    Failure(ParseFailure<S>),
    /// The current buffer ended before the parser could finish. The caller
    /// should append more data and retry from a saved checkpoint.
    Incomplete(Needed),
}

impl<S: Stream> From<ParseFailure<S>> for ParseError<S> {
    fn from(f: ParseFailure<S>) -> Self {
        ParseError::Failure(f)
    }
}

impl<S: Stream> Clone for ParseError<S> {
    fn clone(&self) -> Self {
        match self {
            ParseError::Failure(f) => ParseError::Failure(f.clone()),
            ParseError::Incomplete(n) => ParseError::Incomplete(*n),
        }
    }
}

impl<S: Stream> core::fmt::Debug for ParseError<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Failure(e) => f.debug_tuple("Failure").field(e).finish(),
            ParseError::Incomplete(n) => f.debug_tuple("Incomplete").field(n).finish(),
        }
    }
}

impl<S: Stream> PartialEq for ParseError<S> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ParseError::Failure(a), ParseError::Failure(b)) => a == b,
            (ParseError::Incomplete(a), ParseError::Incomplete(b)) => a == b,
            _ => false,
        }
    }
}

impl<S: Stream> core::fmt::Display for ParseError<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::Failure(e) => core::fmt::Display::fmt(e, f),
            ParseError::Incomplete(Needed::Unknown) => {
                write!(f, "incomplete input (need more data)")
            }
            ParseError::Incomplete(Needed::Size(n)) => {
                write!(f, "incomplete input (need at least {} more base units)", n)
            }
        }
    }
}

impl<S: Stream> std::error::Error for ParseError<S> {}

// ── Recursion cap ─────────────────────────────────────────────────────────────

/// Default cap on parser recursion depth (the number of [`typed::recursive`]
/// crossings on a single parse path). Reaching it yields a [`ParseFailure`]
/// rather than overflowing the native call stack on pathologically nested input.
pub const DEFAULT_MAX_RECURSION_DEPTH: usize = 256;

thread_local! {
    static RECURSION_BUDGET: Cell<usize> = const { Cell::new(DEFAULT_MAX_RECURSION_DEPTH) };
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Advances `cursor` past `consumed` bytes, tracking newlines.
pub(crate) fn advance(cursor: Cursor, consumed: &[u8]) -> Cursor {
    let mut new_line = cursor.line;
    let mut line_start = cursor.line_start_offset;
    for (idx, &b) in consumed.iter().enumerate() {
        if b == b'\n' {
            new_line += 1;
            line_start = cursor.byte_offset + idx + 1;
        }
    }
    Cursor {
        line: new_line,
        line_start_offset: line_start,
        byte_offset: cursor.byte_offset + consumed.len(),
    }
}
