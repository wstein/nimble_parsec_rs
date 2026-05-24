use std::ops::RangeInclusive;
use std::sync::Arc;

use num_bigint::BigInt;

pub use nimble_parsec_rs_macro::compile_parser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub line_start_offset: usize,
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

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An arbitrary-precision integer, mirroring NimbleParsec's BEAM integers,
    /// which are unbounded. Produced by the `integer` and `ascii_char`
    /// combinators (the latter emits the matched byte as its codepoint).
    Int(BigInt),
    Str(String),
    /// A list of values wrapping a combinator's results, produced by `wrap`.
    List(Vec<Value>),
    /// A tagged list of values, produced by `tag`.
    Tagged(String, Vec<Value>),
    /// A tagged single value, produced by `unwrap_and_tag`.
    KeyValue(String, Box<Value>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseSuccess<'a> {
    pub tokens: Vec<Value>,
    pub rest: &'a str,
    pub cursor: Cursor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseFailure<'a> {
    pub reason: String,
    pub rest: &'a str,
    pub cursor: Cursor,
}

pub type ParseResult<'a> = Result<ParseSuccess<'a>, ParseFailure<'a>>;

type ParserFn = dyn for<'a> Fn(&'a str, Cursor) -> ParseResult<'a> + Send + Sync;

#[derive(Clone)]
pub struct Parser {
    f: Arc<ParserFn>,
}

impl Parser {
    pub fn new<F>(f: F) -> Self
    where
        F: for<'a> Fn(&'a str, Cursor) -> ParseResult<'a> + Send + Sync + 'static,
    {
        Self { f: Arc::new(f) }
    }

    pub fn run<'a>(&self, input: &'a str, cursor: Cursor) -> ParseResult<'a> {
        (self.f)(input, cursor)
    }

    pub fn parse<'a>(&self, input: &'a str) -> ParseResult<'a> {
        self.run(input, Cursor::default())
    }
}

#[derive(Clone, Debug)]
pub enum AsciiPredicate {
    Any,
    Range(RangeInclusive<u8>),
    Char(u8),
    NotRange(RangeInclusive<u8>),
    NotChar(u8),
}

/// Codepoint membership constraints for `utf8_char` and `utf8_string`,
/// mirroring NimbleParsec's range list (`min..max`, a codepoint, or their
/// `{:not, ...}` negations). An empty set accepts any codepoint.
#[derive(Clone, Debug)]
pub enum Utf8Predicate {
    Any,
    Range(RangeInclusive<char>),
    Char(char),
    NotRange(RangeInclusive<char>),
    NotChar(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimesOptions {
    pub min: usize,
    pub max: Option<usize>,
}

impl TimesOptions {
    pub fn exact(n: usize) -> Self {
        Self {
            min: n,
            max: Some(n),
        }
    }

    pub fn min_max(min: usize, max: usize) -> Self {
        Self {
            min,
            max: Some(max),
        }
    }

    pub fn min_only(min: usize) -> Self {
        Self { min, max: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatWhileControl {
    Cont,
    Halt,
}

pub fn empty() -> Parser {
    Parser::new(|input, cursor| {
        Ok(ParseSuccess {
            tokens: Vec::new(),
            rest: input,
            cursor,
        })
    })
}

pub fn concat(left: Parser, right: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let mut left_ok = left.run(input, cursor)?;
        let right_ok = right.run(left_ok.rest, left_ok.cursor)?;
        left_ok.tokens.extend(right_ok.tokens);

        Ok(ParseSuccess {
            tokens: left_ok.tokens,
            rest: right_ok.rest,
            cursor: right_ok.cursor,
        })
    })
}

pub fn ignore(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: Vec::new(),
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

pub fn string(lit: &'static str) -> Parser {
    Parser::new(move |input, cursor| {
        if let Some(rest) = input.strip_prefix(lit) {
            let cursor = advance(cursor, lit);
            Ok(ParseSuccess {
                tokens: vec![Value::Str(lit.to_string())],
                rest,
                cursor,
            })
        } else {
            Err(ParseFailure {
                reason: format!("expected string \"{}\"", lit),
                rest: input,
                cursor,
            })
        }
    })
}

pub fn ascii_char(predicates: Vec<AsciiPredicate>) -> Parser {
    Parser::new(move |input, cursor| {
        let Some(&b) = input.as_bytes().first() else {
            return Err(ParseFailure {
                reason: "expected ASCII character".to_string(),
                rest: input,
                cursor,
            });
        };

        if b > 0x7f {
            return Err(ParseFailure {
                reason: "expected ASCII character".to_string(),
                rest: input,
                cursor,
            });
        }

        if !matches_ascii(b, &predicates) {
            return Err(ParseFailure {
                reason: "expected ASCII character in allowed range".to_string(),
                rest: input,
                cursor,
            });
        }

        let consumed = &input[..1];
        let rest = &input[1..];
        let cursor = advance(cursor, consumed);
        Ok(ParseSuccess {
            tokens: vec![Value::Int(BigInt::from(b))],
            rest,
            cursor,
        })
    })
}

pub fn utf8_char(predicates: Vec<Utf8Predicate>) -> Parser {
    Parser::new(move |input, cursor| {
        let Some(ch) = input.chars().next() else {
            return Err(ParseFailure {
                reason: "expected utf8 codepoint".to_string(),
                rest: input,
                cursor,
            });
        };

        if !matches_utf8(ch, &predicates) {
            return Err(ParseFailure {
                reason: "expected utf8 codepoint in allowed range".to_string(),
                rest: input,
                cursor,
            });
        }

        let consumed = &input[..ch.len_utf8()];
        let rest = &input[ch.len_utf8()..];
        let cursor = advance(cursor, consumed);
        Ok(ParseSuccess {
            tokens: vec![Value::Int(BigInt::from(ch as u32))],
            rest,
            cursor,
        })
    })
}

pub fn utf8_string(predicates: Vec<Utf8Predicate>, min: usize, max: Option<usize>) -> Parser {
    Parser::new(move |input, cursor| {
        let mut consumed_end = 0;
        let mut taken = 0usize;

        for (idx, ch) in input.char_indices() {
            if let Some(max) = max {
                if taken >= max {
                    break;
                }
            }

            if !matches_utf8(ch, &predicates) {
                break;
            }

            consumed_end = idx + ch.len_utf8();
            taken += 1;
        }

        if taken < min {
            return Err(ParseFailure {
                reason: "expected utf8 string with minimum length".to_string(),
                rest: input,
                cursor,
            });
        }

        let consumed = &input[..consumed_end];
        let rest = &input[consumed_end..];
        let cursor = advance(cursor, consumed);

        Ok(ParseSuccess {
            tokens: vec![Value::Str(consumed.to_string())],
            rest,
            cursor,
        })
    })
}

pub fn ascii_string(predicates: Vec<AsciiPredicate>, min: usize, max: Option<usize>) -> Parser {
    Parser::new(move |input, cursor| {
        let bytes = input.as_bytes();
        let mut taken = 0usize;
        let mut i = 0usize;

        while i < bytes.len() {
            if let Some(max) = max {
                if taken >= max {
                    break;
                }
            }

            let b = bytes[i];
            if b > 0x7f || !matches_ascii(b, &predicates) {
                break;
            }

            i += 1;
            taken += 1;
        }

        if taken < min {
            return Err(ParseFailure {
                reason: "expected ascii string with minimum length".to_string(),
                rest: input,
                cursor,
            });
        }

        let consumed = &input[..i];
        let rest = &input[i..];
        let cursor = advance(cursor, consumed);
        Ok(ParseSuccess {
            tokens: vec![Value::Str(consumed.to_string())],
            rest,
            cursor,
        })
    })
}

/// Consumes exactly `count` bytes and emits them as a string.
///
/// `count` must fall on a UTF-8 character boundary of the input, since results
/// are returned as `&str`; otherwise the parser fails.
pub fn bytes(count: usize) -> Parser {
    Parser::new(move |input, cursor| match input.get(..count) {
        Some(consumed) => {
            let rest = &input[count..];
            let cursor = advance(cursor, consumed);
            Ok(ParseSuccess {
                tokens: vec![Value::Str(consumed.to_string())],
                rest,
                cursor,
            })
        }
        None => Err(ParseFailure {
            reason: format!("expected {count} bytes"),
            rest: input,
            cursor,
        }),
    })
}

/// Succeeds only at the end of the input, emitting no tokens.
pub fn eos() -> Parser {
    Parser::new(|input, cursor| {
        if input.is_empty() {
            Ok(ParseSuccess {
                tokens: Vec::new(),
                rest: input,
                cursor,
            })
        } else {
            Err(ParseFailure {
                reason: "expected end of string".to_string(),
                rest: input,
                cursor,
            })
        }
    })
}

pub fn integer_exact(n: usize) -> Parser {
    integer_range(n, Some(n))
}

pub fn integer_min(min: usize) -> Parser {
    integer_range(min, None)
}

pub fn integer_range(min: usize, max: Option<usize>) -> Parser {
    Parser::new(move |input, cursor| {
        let bytes = input.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            if let Some(max) = max {
                if i >= max {
                    break;
                }
            }

            if bytes[i].is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }

        if i < min {
            return Err(ParseFailure {
                reason: "expected integer".to_string(),
                rest: input,
                cursor,
            });
        }

        let consumed = &input[..i];
        let rest = &input[i..];
        let cursor = advance(cursor, consumed);
        // `consumed` is a non-empty run of ASCII digits, so parsing into an
        // arbitrary-precision integer is infallible and never overflows.
        let value = consumed
            .parse::<BigInt>()
            .expect("digit run is a valid integer");

        Ok(ParseSuccess {
            tokens: vec![Value::Int(value)],
            rest,
            cursor,
        })
    })
}

pub fn optional(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| match parser.run(input, cursor) {
        Ok(ok) => Ok(ok),
        Err(_) => Ok(ParseSuccess {
            tokens: Vec::new(),
            rest: input,
            cursor,
        }),
    })
}

pub fn choice(parsers: Vec<Parser>) -> Parser {
    Parser::new(move |input, cursor| {
        let mut first_error: Option<ParseFailure<'_>> = None;

        for parser in &parsers {
            match parser.run(input, cursor) {
                Ok(ok) => return Ok(ok),
                Err(err) if first_error.is_none() => first_error = Some(err),
                Err(_) => {}
            }
        }

        Err(first_error.unwrap_or(ParseFailure {
            reason: "choice has no options".to_string(),
            rest: input,
            cursor,
        }))
    })
}

/// Applies `parser` between `min` and `max` (inclusive) times.
///
/// A successful iteration that consumes no input ends the repetition rather
/// than looping forever: earlier iterations are kept and the `min` bound is
/// still enforced afterwards. This favors making progress over failing the
/// whole parse, while still guaranteeing termination.
pub fn repeat(parser: Parser, min: usize, max: Option<usize>) -> Parser {
    Parser::new(move |input, cursor| {
        let mut rest = input;
        let mut cur = cursor;
        let mut tokens = Vec::new();
        let mut count = 0usize;

        loop {
            if let Some(max) = max {
                if count >= max {
                    break;
                }
            }

            match parser.run(rest, cur) {
                Ok(ok) => {
                    if ok.rest.len() == rest.len() {
                        // No input consumed: stop instead of spinning forever.
                        break;
                    }
                    tokens.extend(ok.tokens);
                    rest = ok.rest;
                    cur = ok.cursor;
                    count += 1;
                }
                Err(err) => {
                    if count < min {
                        return Err(err);
                    }
                    break;
                }
            }
        }

        if count < min {
            return Err(ParseFailure {
                reason: "repeat did not reach minimum repetitions".to_string(),
                rest,
                cursor: cur,
            });
        }

        Ok(ParseSuccess {
            tokens,
            rest,
            cursor: cur,
        })
    })
}

pub fn times(parser: Parser, options: TimesOptions) -> Parser {
    if let Some(max) = options.max {
        if max < options.min {
            return Parser::new(move |input, cursor| {
                Err(ParseFailure {
                    reason: "invalid times options: max must be >= min".to_string(),
                    rest: input,
                    cursor,
                })
            });
        }
    }

    repeat(parser, options.min, options.max)
}

/// Parses `parser` exactly `n` times in sequence, concatenating the results,
/// like NimbleParsec's `duplicate`. With `n == 0` it matches nothing.
pub fn duplicate(parser: Parser, n: usize) -> Parser {
    Parser::new(move |input, cursor| {
        let mut rest = input;
        let mut cur = cursor;
        let mut tokens = Vec::new();

        for _ in 0..n {
            let ok = parser.run(rest, cur)?;
            tokens.extend(ok.tokens);
            rest = ok.rest;
            cur = ok.cursor;
        }

        Ok(ParseSuccess {
            tokens,
            rest,
            cursor: cur,
        })
    })
}

/// Skips input one codepoint at a time until `parser` matches, then returns
/// that match; the skipped prefix is discarded. Mirrors NimbleParsec's
/// `eventually`. Fails if the inner parser never matches before end of input.
pub fn eventually(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let mut rest = input;
        let mut cur = cursor;

        loop {
            if let Ok(ok) = parser.run(rest, cur) {
                return Ok(ok);
            }

            match rest.chars().next() {
                Some(ch) => {
                    let consumed = &rest[..ch.len_utf8()];
                    cur = advance(cur, consumed);
                    rest = &rest[ch.len_utf8()..];
                }
                None => {
                    return Err(ParseFailure {
                        reason: "expected combinator to eventually match".to_string(),
                        rest: input,
                        cursor,
                    });
                }
            }
        }
    })
}

pub fn lookahead(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        parser.run(input, cursor).map(|_| ParseSuccess {
            tokens: Vec::new(),
            rest: input,
            cursor,
        })
    })
}

pub fn lookahead_not(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| match parser.run(input, cursor) {
        Ok(_) => Err(ParseFailure {
            reason: "did not expect lookahead parser to match".to_string(),
            rest: input,
            cursor,
        }),
        Err(_) => Ok(ParseSuccess {
            tokens: Vec::new(),
            rest: input,
            cursor,
        }),
    })
}

/// Applies `parser` while `while_fn` returns [`RepeatWhileControl::Cont`],
/// between `min` and `max` (inclusive) times.
///
/// Like [`repeat`], a successful iteration that consumes no input ends the
/// repetition instead of looping forever; the `min` bound is enforced once the
/// loop stops.
pub fn repeat_while<F>(parser: Parser, while_fn: F, min: usize, max: Option<usize>) -> Parser
where
    F: Fn(&str, Cursor) -> RepeatWhileControl + Send + Sync + 'static,
{
    Parser::new(move |input, cursor| {
        let mut rest = input;
        let mut cur = cursor;
        let mut tokens = Vec::new();
        let mut count = 0usize;

        loop {
            if let Some(max) = max {
                if count >= max {
                    break;
                }
            }

            match while_fn(rest, cur) {
                RepeatWhileControl::Halt => break,
                RepeatWhileControl::Cont => {}
            }

            match parser.run(rest, cur) {
                Ok(ok) => {
                    if ok.rest.len() == rest.len() {
                        // No input consumed: stop instead of spinning forever.
                        break;
                    }
                    tokens.extend(ok.tokens);
                    rest = ok.rest;
                    cur = ok.cursor;
                    count += 1;
                }
                Err(_) => break,
            }
        }

        if count < min {
            return Err(ParseFailure {
                reason: "repeat_while did not reach minimum repetitions".to_string(),
                rest,
                cursor: cur,
            });
        }

        Ok(ParseSuccess {
            tokens,
            rest,
            cursor: cur,
        })
    })
}

/// Maps `f` over each result token individually, like NimbleParsec's `map`.
pub fn map<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Value) -> Value + Send + Sync + 'static,
{
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: ok.tokens.into_iter().map(&f).collect(),
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Reduces all result tokens into a single token via `f`, like NimbleParsec's
/// `reduce`.
pub fn reduce<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Vec<Value>) -> Value + Send + Sync + 'static,
{
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: vec![f(ok.tokens)],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

pub fn tag(name: &'static str, parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: vec![Value::Tagged(name.to_string(), ok.tokens)],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Tags a single result token, like NimbleParsec's `unwrap_and_tag`. Fails if
/// the combinator does not emit exactly one token.
pub fn unwrap_and_tag(name: &'static str, parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        let mut tokens = ok.tokens;
        if tokens.len() != 1 {
            return Err(ParseFailure {
                reason: format!("expected exactly one token to unwrap_and_tag as \"{name}\""),
                rest: input,
                cursor,
            });
        }
        let value = tokens.pop().expect("length checked above");
        Ok(ParseSuccess {
            tokens: vec![Value::KeyValue(name.to_string(), Box::new(value))],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Wraps all result tokens into a single list value, like NimbleParsec's `wrap`.
pub fn wrap(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: vec![Value::List(ok.tokens)],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Replaces all result tokens with a single constant `value`, like
/// NimbleParsec's `replace`.
pub fn replace(parser: Parser, value: Value) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: vec![value.clone()],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Replaces the failure message of `parser` with `expected <label>`, like
/// NimbleParsec's `label`. The failure position is preserved; success passes
/// through unchanged.
pub fn label(parser: Parser, label: &'static str) -> Parser {
    Parser::new(move |input, cursor| {
        parser.run(input, cursor).map_err(|err| ParseFailure {
            reason: format!("expected {label}"),
            rest: err.rest,
            cursor: err.cursor,
        })
    })
}

/// Wraps `parser`'s results with the trailing byte offset, like NimbleParsec's
/// `byte_offset`. Emits a single pair `List([List(results), Int(offset)])`,
/// where `offset` is the byte offset after the wrapped combinator.
pub fn byte_offset(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        let token = Value::List(vec![
            Value::List(ok.tokens),
            Value::Int(BigInt::from(ok.cursor.byte_offset)),
        ]);
        Ok(ParseSuccess {
            tokens: vec![token],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Wraps `parser`'s results with the trailing line position, like NimbleParsec's
/// `line`. Emits a single pair `List([List(results), List([line, line_offset])])`,
/// where `line_offset` is the byte offset immediately after the last newline.
pub fn line(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        let position = Value::List(vec![
            Value::Int(BigInt::from(ok.cursor.line)),
            Value::Int(BigInt::from(ok.cursor.line_start_offset)),
        ]);
        let token = Value::List(vec![Value::List(ok.tokens), position]);
        Ok(ParseSuccess {
            tokens: vec![token],
            rest: ok.rest,
            cursor: ok.cursor,
        })
    })
}

/// Prints the parser state around `parser` to stderr (the input before, and the
/// result after) and passes the result through unchanged, like NimbleParsec's
/// `debug`.
pub fn debug(parser: Parser) -> Parser {
    Parser::new(move |input, cursor| {
        eprintln!("debug: parsing {input:?} at {cursor:?}");
        let result = parser.run(input, cursor);
        match &result {
            Ok(ok) => eprintln!("debug: ok tokens={:?} rest={:?}", ok.tokens, ok.rest),
            Err(err) => eprintln!("debug: error {:?}", err.reason),
        }
        result
    })
}

/// Shared positive/negative membership rule for character predicates.
///
/// Each item is `(is_negative, contains)`. A value is accepted when it hits at
/// least one positive predicate (or there are none) and no negative predicate.
/// An empty set therefore accepts any value, matching NimbleParsec's `[]`.
fn matches_ranges(predicates: impl IntoIterator<Item = (bool, bool)>) -> bool {
    let mut has_positive = false;
    let mut positive_hit = false;
    let mut negative_hit = false;

    for (is_negative, contains) in predicates {
        if is_negative {
            negative_hit |= contains;
        } else {
            has_positive = true;
            positive_hit |= contains;
        }
    }

    (!has_positive || positive_hit) && !negative_hit
}

fn matches_ascii(b: u8, predicates: &[AsciiPredicate]) -> bool {
    matches_ranges(predicates.iter().map(|p| match p {
        AsciiPredicate::Any => (false, true),
        AsciiPredicate::Range(r) => (false, r.contains(&b)),
        AsciiPredicate::Char(c) => (false, *c == b),
        AsciiPredicate::NotRange(r) => (true, r.contains(&b)),
        AsciiPredicate::NotChar(c) => (true, *c == b),
    }))
}

fn matches_utf8(ch: char, predicates: &[Utf8Predicate]) -> bool {
    matches_ranges(predicates.iter().map(|p| match p {
        Utf8Predicate::Any => (false, true),
        Utf8Predicate::Range(r) => (false, r.contains(&ch)),
        Utf8Predicate::Char(c) => (false, *c == ch),
        Utf8Predicate::NotRange(r) => (true, r.contains(&ch)),
        Utf8Predicate::NotChar(c) => (true, *c == ch),
    }))
}

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
