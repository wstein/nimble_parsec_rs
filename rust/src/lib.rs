//! A Rust port of [NimbleParsec](https://github.com/dashbitco/nimble_parsec),
//! a parser-combinator library — with an idiomatic, **typed** surface.
//!
//! Build a parser by composing combinators; each is generic over its output, so
//! grammars compose and type-check at compile time with no runtime tagging. The
//! combinators live in [`typed`] and are re-exported here.
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
//! (line + byte offset).
#![deny(missing_docs)]

use std::cell::Cell;

pub mod typed;
pub use typed::*;

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

/// A failed parse: why it failed and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseFailure<'a> {
    /// Human-readable failure message. For a `choice` this is the alternatives'
    /// messages joined with `" or "`; it always renders [`expected`](Self::expected)
    /// when that set is non-empty.
    pub reason: String,
    /// The token or character-class descriptions the parser was looking for at
    /// [`cursor`](Self::cursor), aggregated across `choice` alternatives (e.g.
    /// `["expected \"a\"", "expected an integer"]`). Empty for failures that are
    /// not simple expectations — negative assertions (`not`), semantic rejections
    /// (`try_map`), or structural limits (the recursion cap).
    pub expected: Vec<String>,
    /// The input at the point of failure.
    pub rest: &'a str,
    /// Position at the point of failure.
    pub cursor: Cursor,
}

impl<'a> ParseFailure<'a> {
    /// Builds a failure where the parser expected a specific token or character
    /// class. `what` (e.g. `expected "x"`) becomes both the [`reason`](Self::reason)
    /// and the single entry of the [`expected`](Self::expected) set.
    pub fn expecting(what: impl Into<String>, rest: &'a str, cursor: Cursor) -> Self {
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
    pub fn rejected(reason: impl Into<String>, rest: &'a str, cursor: Cursor) -> Self {
        ParseFailure {
            reason: reason.into(),
            expected: Vec::new(),
            rest,
            cursor,
        }
    }
}

impl std::fmt::Display for ParseFailure<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (line {}, byte offset {})",
            self.reason, self.cursor.line, self.cursor.byte_offset
        )
    }
}

impl std::error::Error for ParseFailure<'_> {}

/// Default cap on parser recursion depth (the number of [`typed::recursive`]
/// crossings on a single parse path). Reaching it yields a [`ParseFailure`]
/// rather than overflowing the native call stack on pathologically nested input.
///
/// Sized for release builds on a 2 MiB thread stack (the tokio-worker default),
/// where each level costs well under 1 KiB and 256 levels leaves a comfortable
/// margin, while far exceeding any realistic grammar nesting. Note that *debug*
/// builds have much larger stack frames, so a debug parse of input nested
/// hundreds deep may exhaust the stack before the cap; tune it via
/// [`typed::Parser::parse_with_max_depth`] if you run untrusted input through a
/// debug build, and up for legitimately deep grammars.
pub const DEFAULT_MAX_RECURSION_DEPTH: usize = 256;

thread_local! {
    // Remaining recursion budget for the parse running on this thread. Set at the
    // start of each `parse*` call and restored on return (so a nested `parse`
    // from inside a transform closure is re-entrancy safe). Only `recursive`
    // spends from it: that is the sole point where *input* drives unbounded
    // recursion — `repeated` is an iterative loop, and `then`/`or` only recurse
    // to the static (build-time) grammar depth, bounded by the grammar itself.
    static RECURSION_BUDGET: Cell<usize> = const { Cell::new(DEFAULT_MAX_RECURSION_DEPTH) };
}

/// Advances `cursor` past `consumed`, updating the line and byte offset.
fn advance(cursor: Cursor, consumed: &str) -> Cursor {
    let consumed_bytes = consumed.as_bytes();
    let mut new_line = cursor.line;
    let mut line_start = cursor.line_start_offset;

    for (idx, b) in consumed_bytes.iter().enumerate() {
        if *b == b'\n' {
            new_line += 1;
            line_start = cursor.byte_offset + idx + 1;
        }
    }

    Cursor {
        line: new_line,
        line_start_offset: line_start,
        byte_offset: cursor.byte_offset + consumed_bytes.len(),
    }
}
