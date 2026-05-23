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
    /// which are unbounded. Produced by the `integer` combinators.
    Int(BigInt),
    Str(String),
    Char(char),
    Tagged(String, Vec<Value>),
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
        if input.starts_with(lit) {
            let rest = &input[lit.len()..];
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
            tokens: vec![Value::Char(char::from(b))],
            rest,
            cursor,
        })
    })
}

pub fn utf8_string(min: usize, max: Option<usize>) -> Parser {
    Parser::new(move |input, cursor| {
        let mut chars = input.char_indices();
        let mut consumed_end = 0;
        let mut taken = 0usize;

        while let Some((idx, ch)) = chars.next() {
            if let Some(max) = max {
                if taken >= max {
                    break;
                }
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
                        return Err(ParseFailure {
                            reason: "repeat parser consumed no input".to_string(),
                            rest,
                            cursor: cur,
                        });
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
                        return Err(ParseFailure {
                            reason: "repeat_while parser consumed no input".to_string(),
                            rest,
                            cursor: cur,
                        });
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

pub fn map<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Vec<Value>) -> Vec<Value> + Send + Sync + 'static,
{
    Parser::new(move |input, cursor| {
        let ok = parser.run(input, cursor)?;
        Ok(ParseSuccess {
            tokens: f(ok.tokens),
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

fn matches_ascii(b: u8, predicates: &[AsciiPredicate]) -> bool {
    if predicates.is_empty() {
        return true;
    }

    let has_positive = predicates.iter().any(|p| {
        matches!(
            p,
            AsciiPredicate::Any | AsciiPredicate::Range(_) | AsciiPredicate::Char(_)
        )
    });

    let positive_match = if has_positive {
        predicates.iter().any(|p| match p {
            AsciiPredicate::Any => true,
            AsciiPredicate::Range(r) => r.contains(&b),
            AsciiPredicate::Char(c) => *c == b,
            AsciiPredicate::NotRange(_) | AsciiPredicate::NotChar(_) => false,
        })
    } else {
        true
    };

    let negative_match = predicates.iter().any(|p| match p {
        AsciiPredicate::NotRange(r) => r.contains(&b),
        AsciiPredicate::NotChar(c) => *c == b,
        AsciiPredicate::Any | AsciiPredicate::Range(_) | AsciiPredicate::Char(_) => false,
    });

    positive_match && !negative_match
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
