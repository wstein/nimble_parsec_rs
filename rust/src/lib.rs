use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::{Arc, OnceLock};

use num_bigint::BigInt;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

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

/// User-defined parser context threaded forward through successful parses,
/// mirroring NimbleParsec's context map. It is read and updated by
/// `post_traverse`/`pre_traverse`. On failure the caller backtracks with its
/// own context, so [`ParseFailure`] does not carry one.
pub type Context = HashMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct ParseSuccess<'a> {
    pub tokens: Vec<Value>,
    pub rest: &'a str,
    pub cursor: Cursor,
    pub context: Context,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseFailure<'a> {
    pub reason: String,
    pub rest: &'a str,
    pub cursor: Cursor,
}

pub type ParseResult<'a> = Result<ParseSuccess<'a>, ParseFailure<'a>>;

type MapFn = dyn Fn(Value) -> Value + Send + Sync;
type ReduceFn = dyn Fn(Vec<Value>) -> Value + Send + Sync;
type TraverseFn =
    dyn Fn(Vec<Value>, Context, Cursor) -> Result<(Vec<Value>, Context), String> + Send + Sync;
type WhileFn = dyn Fn(&str, Cursor) -> RepeatWhileControl + Send + Sync;

/// The reified parser grammar. Every combinator builds an `Ast` node; the
/// [`Parser`] interpreter ([`run_ast`]) walks it, and `generate` samples from
/// it. Transform combinators embed opaque closures (`map`, `reduce`,
/// `repeat_while`, `post_traverse`, `pre_traverse`); these are run when parsing
/// and skipped when generating, since they shape tokens, not input.
enum Ast {
    Empty,
    Fail(&'static str),
    Str(&'static str),
    AsciiChar {
        predicates: Vec<AsciiPredicate>,
        reason: String,
    },
    Utf8Char {
        predicates: Vec<Utf8Predicate>,
        reason: String,
    },
    Utf8String {
        predicates: Vec<Utf8Predicate>,
        min: usize,
        max: Option<usize>,
    },
    AsciiString {
        predicates: Vec<AsciiPredicate>,
        min: usize,
        max: Option<usize>,
    },
    Bytes(usize),
    Eos,
    Integer {
        min: usize,
        max: Option<usize>,
    },
    Concat(Arc<Ast>, Arc<Ast>),
    Ignore(Arc<Ast>),
    Optional(Arc<Ast>),
    Choice(Vec<Arc<Ast>>),
    Repeat {
        inner: Arc<Ast>,
        min: usize,
        max: Option<usize>,
    },
    Duplicate {
        inner: Arc<Ast>,
        n: usize,
    },
    Eventually(Arc<Ast>),
    Lookahead(Arc<Ast>),
    LookaheadNot(Arc<Ast>),
    RepeatWhile {
        inner: Arc<Ast>,
        while_fn: Arc<WhileFn>,
        min: usize,
        max: Option<usize>,
    },
    Map(Arc<Ast>, Arc<MapFn>),
    Reduce(Arc<Ast>, Arc<ReduceFn>),
    Tag(&'static str, Arc<Ast>),
    UnwrapAndTag(&'static str, Arc<Ast>),
    Wrap(Arc<Ast>),
    Replace(Arc<Ast>, Value),
    Label(Arc<Ast>, &'static str),
    ByteOffset(Arc<Ast>),
    Line(Arc<Ast>),
    Debug(Arc<Ast>),
    PostTraverse(Arc<Ast>, Arc<TraverseFn>),
    PreTraverse(Arc<Ast>, Arc<TraverseFn>),
    Reference(Arc<OnceLock<Arc<Ast>>>),
}

impl std::fmt::Debug for Ast {
    /// Structural debug rendering. Embedded closures print as `<fn>`, and
    /// references are not followed (they print as `Reference(<ref>)`) so the
    /// output stays finite for recursive grammars.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ast::Empty => write!(f, "Empty"),
            Ast::Fail(reason) => write!(f, "Fail({reason:?})"),
            Ast::Str(lit) => write!(f, "Str({lit:?})"),
            Ast::AsciiChar { predicates, .. } => write!(f, "AsciiChar({predicates:?})"),
            Ast::Utf8Char { predicates, .. } => write!(f, "Utf8Char({predicates:?})"),
            Ast::Utf8String {
                predicates,
                min,
                max,
            } => write!(f, "Utf8String({predicates:?}, {min}, {max:?})"),
            Ast::AsciiString {
                predicates,
                min,
                max,
            } => write!(f, "AsciiString({predicates:?}, {min}, {max:?})"),
            Ast::Bytes(n) => write!(f, "Bytes({n})"),
            Ast::Eos => write!(f, "Eos"),
            Ast::Integer { min, max } => write!(f, "Integer({min}, {max:?})"),
            Ast::Concat(left, right) => write!(f, "Concat({left:?}, {right:?})"),
            Ast::Ignore(inner) => write!(f, "Ignore({inner:?})"),
            Ast::Optional(inner) => write!(f, "Optional({inner:?})"),
            Ast::Choice(choices) => f.debug_tuple("Choice").field(choices).finish(),
            Ast::Repeat { inner, min, max } => write!(f, "Repeat({inner:?}, {min}, {max:?})"),
            Ast::Duplicate { inner, n } => write!(f, "Duplicate({inner:?}, {n})"),
            Ast::Eventually(inner) => write!(f, "Eventually({inner:?})"),
            Ast::Lookahead(inner) => write!(f, "Lookahead({inner:?})"),
            Ast::LookaheadNot(inner) => write!(f, "LookaheadNot({inner:?})"),
            Ast::RepeatWhile {
                inner, min, max, ..
            } => write!(f, "RepeatWhile({inner:?}, {min}, {max:?}, <fn>)"),
            Ast::Map(inner, _) => write!(f, "Map({inner:?}, <fn>)"),
            Ast::Reduce(inner, _) => write!(f, "Reduce({inner:?}, <fn>)"),
            Ast::Tag(name, inner) => write!(f, "Tag({name:?}, {inner:?})"),
            Ast::UnwrapAndTag(name, inner) => write!(f, "UnwrapAndTag({name:?}, {inner:?})"),
            Ast::Wrap(inner) => write!(f, "Wrap({inner:?})"),
            Ast::Replace(inner, value) => write!(f, "Replace({inner:?}, {value:?})"),
            Ast::Label(inner, label) => write!(f, "Label({inner:?}, {label:?})"),
            Ast::ByteOffset(inner) => write!(f, "ByteOffset({inner:?})"),
            Ast::Line(inner) => write!(f, "Line({inner:?})"),
            Ast::Debug(inner) => write!(f, "Debug({inner:?})"),
            Ast::PostTraverse(inner, _) => write!(f, "PostTraverse({inner:?}, <fn>)"),
            Ast::PreTraverse(inner, _) => write!(f, "PreTraverse({inner:?}, <fn>)"),
            Ast::Reference(_) => write!(f, "Reference(<ref>)"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Parser {
    ast: Arc<Ast>,
}

impl Parser {
    fn from_ast(ast: Ast) -> Self {
        Self { ast: Arc::new(ast) }
    }

    pub fn run<'a>(&self, input: &'a str, cursor: Cursor, context: Context) -> ParseResult<'a> {
        run_ast(&self.ast, input, cursor, context)
    }

    pub fn parse<'a>(&self, input: &'a str) -> ParseResult<'a> {
        self.run(input, Cursor::default(), Context::new())
    }
}

/// A forward-declarable parser reference enabling recursive grammars, mirroring
/// NimbleParsec's `parsec`. Create one, use [`ParserRef::parser`] inside a
/// definition, then supply that definition with [`ParserRef::define`]. Cloning a
/// `ParserRef` shares the same underlying definition.
///
/// Left recursion is not supported (it loops forever, as in any recursive
/// descent parser); only recurse after consuming input.
#[derive(Clone, Default)]
pub struct ParserRef {
    cell: Arc<OnceLock<Arc<Ast>>>,
}

impl ParserRef {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a parser that resolves to the referenced definition at parse
    /// time. Panics if run before [`ParserRef::define`].
    pub fn parser(&self) -> Parser {
        Parser {
            ast: Arc::new(Ast::Reference(Arc::clone(&self.cell))),
        }
    }

    /// Supplies the referenced definition. Must be called exactly once.
    pub fn define(&self, parser: Parser) {
        self.cell
            .set(parser.ast)
            .expect("parsec reference was already defined");
    }
}

/// Builds a recursive parser. `build` receives a reference to the parser being
/// defined (usable within the returned definition) and returns that definition.
/// Convenience wrapper over [`ParserRef`].
pub fn recursive<F>(build: F) -> Parser
where
    F: FnOnce(Parser) -> Parser,
{
    let reference = ParserRef::new();
    let definition = build(reference.parser());
    reference.define(definition);
    reference.parser()
}

/// Tunes [`generate_with`].
#[derive(Clone, Copy, Debug)]
pub struct GenerateConfig {
    /// Maximum reference-expansion depth before recursion stops, keeping
    /// generation terminating on recursive grammars. Default 16.
    pub max_recursion_depth: usize,
    /// For unbounded repetitions (`max == None`), the random count is drawn from
    /// `min..=min + repeat_window`. Default 3.
    pub repeat_window: usize,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            max_recursion_depth: 16,
            repeat_window: 3,
        }
    }
}

/// Generates a random input string accepted by `parser`, seeded by `seed` for
/// reproducibility, mirroring NimbleParsec's `generate`. For non-recursive
/// grammars the result round-trips (it parses successfully). Recursion is
/// depth-bounded so generation always terminates; zero-width assertions
/// (`lookahead`/`lookahead_not`) contribute no input by design, and
/// `repeat_while` sampling is best-effort because its predicate is opaque.
pub fn generate(parser: &Parser, seed: u64) -> String {
    generate_with(parser, seed, GenerateConfig::default())
}

/// Like [`generate`], but with a caller-supplied [`GenerateConfig`].
pub fn generate_with(parser: &Parser, seed: u64, config: GenerateConfig) -> String {
    let mut rng = StdRng::seed_from_u64(seed);
    generate_ast(&parser.ast, &mut rng, 0, &config)
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
    Parser::from_ast(Ast::Empty)
}

pub fn concat(left: Parser, right: Parser) -> Parser {
    Parser::from_ast(Ast::Concat(left.ast, right.ast))
}

pub fn ignore(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Ignore(parser.ast))
}

pub fn string(lit: &'static str) -> Parser {
    Parser::from_ast(Ast::Str(lit))
}

pub fn ascii_char(predicates: Vec<AsciiPredicate>) -> Parser {
    let reason = format!("expected {}", describe_ascii(&predicates));
    Parser::from_ast(Ast::AsciiChar { predicates, reason })
}

pub fn utf8_char(predicates: Vec<Utf8Predicate>) -> Parser {
    let reason = format!("expected {}", describe_utf8(&predicates));
    Parser::from_ast(Ast::Utf8Char { predicates, reason })
}

pub fn utf8_string(predicates: Vec<Utf8Predicate>, min: usize, max: Option<usize>) -> Parser {
    Parser::from_ast(Ast::Utf8String {
        predicates,
        min,
        max,
    })
}

pub fn ascii_string(predicates: Vec<AsciiPredicate>, min: usize, max: Option<usize>) -> Parser {
    Parser::from_ast(Ast::AsciiString {
        predicates,
        min,
        max,
    })
}

/// Consumes exactly `count` bytes and emits them as a string.
///
/// `count` must fall on a UTF-8 character boundary of the input, since results
/// are returned as `&str`; otherwise the parser fails.
pub fn bytes(count: usize) -> Parser {
    Parser::from_ast(Ast::Bytes(count))
}

/// Succeeds only at the end of the input, emitting no tokens.
pub fn eos() -> Parser {
    Parser::from_ast(Ast::Eos)
}

pub fn integer_exact(n: usize) -> Parser {
    integer_range(n, Some(n))
}

pub fn integer_min(min: usize) -> Parser {
    integer_range(min, None)
}

pub fn integer_range(min: usize, max: Option<usize>) -> Parser {
    Parser::from_ast(Ast::Integer { min, max })
}

pub fn optional(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Optional(parser.ast))
}

/// Tries each parser in order, returning the first success. If all fail, the
/// branch failure messages are aggregated (joined with " or "), like
/// NimbleParsec, rather than surfacing only the first.
pub fn choice(parsers: Vec<Parser>) -> Parser {
    Parser::from_ast(Ast::Choice(parsers.into_iter().map(|p| p.ast).collect()))
}

/// Applies `parser` between `min` and `max` (inclusive) times.
///
/// A successful iteration that consumes no input ends the repetition rather
/// than looping forever: earlier iterations are kept and the `min` bound is
/// still enforced afterwards. This favors making progress over failing the
/// whole parse, while still guaranteeing termination.
pub fn repeat(parser: Parser, min: usize, max: Option<usize>) -> Parser {
    Parser::from_ast(Ast::Repeat {
        inner: parser.ast,
        min,
        max,
    })
}

pub fn times(parser: Parser, options: TimesOptions) -> Parser {
    if let Some(max) = options.max {
        if max < options.min {
            return Parser::from_ast(Ast::Fail("invalid times options: max must be >= min"));
        }
    }

    repeat(parser, options.min, options.max)
}

/// Parses `parser` exactly `n` times in sequence, concatenating the results,
/// like NimbleParsec's `duplicate`. With `n == 0` it matches nothing.
pub fn duplicate(parser: Parser, n: usize) -> Parser {
    Parser::from_ast(Ast::Duplicate {
        inner: parser.ast,
        n,
    })
}

/// Skips input one codepoint at a time until `parser` matches, then returns
/// that match; the skipped prefix is discarded. Mirrors NimbleParsec's
/// `eventually`. Fails if the inner parser never matches before end of input.
pub fn eventually(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Eventually(parser.ast))
}

pub fn lookahead(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Lookahead(parser.ast))
}

pub fn lookahead_not(parser: Parser) -> Parser {
    Parser::from_ast(Ast::LookaheadNot(parser.ast))
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
    Parser::from_ast(Ast::RepeatWhile {
        inner: parser.ast,
        while_fn: Arc::new(while_fn),
        min,
        max,
    })
}

/// Maps `f` over each result token individually, like NimbleParsec's `map`.
pub fn map<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Value) -> Value + Send + Sync + 'static,
{
    Parser::from_ast(Ast::Map(parser.ast, Arc::new(f)))
}

/// Reduces all result tokens into a single token via `f`, like NimbleParsec's
/// `reduce`.
pub fn reduce<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Vec<Value>) -> Value + Send + Sync + 'static,
{
    Parser::from_ast(Ast::Reduce(parser.ast, Arc::new(f)))
}

/// Low-level transform: runs `parser`, then calls `f` with the results, the
/// threaded context, and the position *after* the combinator, mirroring
/// NimbleParsec's `post_traverse`. `f` returns the new results and context, or
/// an error message that fails the parse. `map`, `reduce`, `wrap`, `replace`,
/// `tag`, `line`, and `byte_offset` are higher-level forms of this.
pub fn post_traverse<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Vec<Value>, Context, Cursor) -> Result<(Vec<Value>, Context), String>
        + Send
        + Sync
        + 'static,
{
    Parser::from_ast(Ast::PostTraverse(parser.ast, Arc::new(f)))
}

/// Like [`post_traverse`], but `f` receives the position *before* the
/// combinator runs, mirroring NimbleParsec's `pre_traverse`.
pub fn pre_traverse<F>(parser: Parser, f: F) -> Parser
where
    F: Fn(Vec<Value>, Context, Cursor) -> Result<(Vec<Value>, Context), String>
        + Send
        + Sync
        + 'static,
{
    Parser::from_ast(Ast::PreTraverse(parser.ast, Arc::new(f)))
}

pub fn tag(name: &'static str, parser: Parser) -> Parser {
    Parser::from_ast(Ast::Tag(name, parser.ast))
}

/// Tags a single result token, like NimbleParsec's `unwrap_and_tag`. Fails if
/// the combinator does not emit exactly one token.
pub fn unwrap_and_tag(name: &'static str, parser: Parser) -> Parser {
    Parser::from_ast(Ast::UnwrapAndTag(name, parser.ast))
}

/// Wraps all result tokens into a single list value, like NimbleParsec's `wrap`.
pub fn wrap(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Wrap(parser.ast))
}

/// Replaces all result tokens with a single constant `value`, like
/// NimbleParsec's `replace`.
pub fn replace(parser: Parser, value: Value) -> Parser {
    Parser::from_ast(Ast::Replace(parser.ast, value))
}

/// Replaces the failure message of `parser` with `expected <label>`, like
/// NimbleParsec's `label`. The failure position is preserved; success passes
/// through unchanged.
pub fn label(parser: Parser, label: &'static str) -> Parser {
    Parser::from_ast(Ast::Label(parser.ast, label))
}

/// Wraps `parser`'s results with the trailing byte offset, like NimbleParsec's
/// `byte_offset`. Emits a single pair `List([List(results), Int(offset)])`,
/// where `offset` is the byte offset after the wrapped combinator.
pub fn byte_offset(parser: Parser) -> Parser {
    Parser::from_ast(Ast::ByteOffset(parser.ast))
}

/// Wraps `parser`'s results with the trailing line position, like NimbleParsec's
/// `line`. Emits a single pair `List([List(results), List([line, line_offset])])`,
/// where `line_offset` is the byte offset immediately after the last newline.
pub fn line(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Line(parser.ast))
}

/// Prints the parser state around `parser` to stderr (the input before, and the
/// result after) and passes the result through unchanged, like NimbleParsec's
/// `debug`.
pub fn debug(parser: Parser) -> Parser {
    Parser::from_ast(Ast::Debug(parser.ast))
}

fn run_ast<'a>(
    ast: &Arc<Ast>,
    input: &'a str,
    cursor: Cursor,
    context: Context,
) -> ParseResult<'a> {
    match ast.as_ref() {
        Ast::Empty => Ok(ParseSuccess {
            tokens: Vec::new(),
            rest: input,
            cursor,
            context,
        }),

        Ast::Fail(reason) => Err(ParseFailure {
            reason: (*reason).to_string(),
            rest: input,
            cursor,
        }),

        Ast::Str(lit) => {
            if let Some(rest) = input.strip_prefix(*lit) {
                Ok(ParseSuccess {
                    tokens: vec![Value::Str((*lit).to_string())],
                    rest,
                    cursor: advance(cursor, lit),
                    context,
                })
            } else {
                Err(ParseFailure {
                    reason: format!("expected string \"{}\"", lit),
                    rest: input,
                    cursor,
                })
            }
        }

        Ast::AsciiChar { predicates, reason } => {
            let Some(&b) = input.as_bytes().first() else {
                return Err(ParseFailure {
                    reason: reason.clone(),
                    rest: input,
                    cursor,
                });
            };
            if b > 0x7f || !matches_ascii(b, predicates) {
                return Err(ParseFailure {
                    reason: reason.clone(),
                    rest: input,
                    cursor,
                });
            }
            let consumed = &input[..1];
            Ok(ParseSuccess {
                tokens: vec![Value::Int(BigInt::from(b))],
                rest: &input[1..],
                cursor: advance(cursor, consumed),
                context,
            })
        }

        Ast::Utf8Char { predicates, reason } => {
            let Some(ch) = input.chars().next() else {
                return Err(ParseFailure {
                    reason: reason.clone(),
                    rest: input,
                    cursor,
                });
            };
            if !matches_utf8(ch, predicates) {
                return Err(ParseFailure {
                    reason: reason.clone(),
                    rest: input,
                    cursor,
                });
            }
            let consumed = &input[..ch.len_utf8()];
            Ok(ParseSuccess {
                tokens: vec![Value::Int(BigInt::from(ch as u32))],
                rest: &input[ch.len_utf8()..],
                cursor: advance(cursor, consumed),
                context,
            })
        }

        Ast::Utf8String {
            predicates,
            min,
            max,
        } => {
            let mut consumed_end = 0;
            let mut taken = 0usize;
            for (idx, ch) in input.char_indices() {
                if let Some(max) = max {
                    if taken >= *max {
                        break;
                    }
                }
                if !matches_utf8(ch, predicates) {
                    break;
                }
                consumed_end = idx + ch.len_utf8();
                taken += 1;
            }
            if taken < *min {
                return Err(ParseFailure {
                    reason: "expected utf8 string with minimum length".to_string(),
                    rest: input,
                    cursor,
                });
            }
            let consumed = &input[..consumed_end];
            Ok(ParseSuccess {
                tokens: vec![Value::Str(consumed.to_string())],
                rest: &input[consumed_end..],
                cursor: advance(cursor, consumed),
                context,
            })
        }

        Ast::AsciiString {
            predicates,
            min,
            max,
        } => {
            let raw = input.as_bytes();
            let mut taken = 0usize;
            let mut i = 0usize;
            while i < raw.len() {
                if let Some(max) = max {
                    if taken >= *max {
                        break;
                    }
                }
                let b = raw[i];
                if b > 0x7f || !matches_ascii(b, predicates) {
                    break;
                }
                i += 1;
                taken += 1;
            }
            if taken < *min {
                return Err(ParseFailure {
                    reason: "expected ascii string with minimum length".to_string(),
                    rest: input,
                    cursor,
                });
            }
            let consumed = &input[..i];
            Ok(ParseSuccess {
                tokens: vec![Value::Str(consumed.to_string())],
                rest: &input[i..],
                cursor: advance(cursor, consumed),
                context,
            })
        }

        Ast::Bytes(count) => match input.get(..*count) {
            Some(consumed) => Ok(ParseSuccess {
                tokens: vec![Value::Str(consumed.to_string())],
                rest: &input[*count..],
                cursor: advance(cursor, consumed),
                context,
            }),
            None => Err(ParseFailure {
                reason: format!("expected {count} bytes"),
                rest: input,
                cursor,
            }),
        },

        Ast::Eos => {
            if input.is_empty() {
                Ok(ParseSuccess {
                    tokens: Vec::new(),
                    rest: input,
                    cursor,
                    context,
                })
            } else {
                Err(ParseFailure {
                    reason: "expected end of string".to_string(),
                    rest: input,
                    cursor,
                })
            }
        }

        Ast::Integer { min, max } => {
            let raw = input.as_bytes();
            let mut i = 0usize;
            while i < raw.len() {
                if let Some(max) = max {
                    if i >= *max {
                        break;
                    }
                }
                if raw[i].is_ascii_digit() {
                    i += 1;
                } else {
                    break;
                }
            }
            if i < *min {
                return Err(ParseFailure {
                    reason: "expected integer".to_string(),
                    rest: input,
                    cursor,
                });
            }
            let consumed = &input[..i];
            let value = consumed
                .parse::<BigInt>()
                .expect("digit run is a valid integer");
            Ok(ParseSuccess {
                tokens: vec![Value::Int(value)],
                rest: &input[i..],
                cursor: advance(cursor, consumed),
                context,
            })
        }

        Ast::Concat(left, right) => {
            let mut left_ok = run_ast(left, input, cursor, context)?;
            let right_ok = run_ast(right, left_ok.rest, left_ok.cursor, left_ok.context)?;
            left_ok.tokens.extend(right_ok.tokens);
            Ok(ParseSuccess {
                tokens: left_ok.tokens,
                rest: right_ok.rest,
                cursor: right_ok.cursor,
                context: right_ok.context,
            })
        }

        Ast::Ignore(inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: Vec::new(),
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Optional(inner) => match run_ast(inner, input, cursor, context.clone()) {
            Ok(ok) => Ok(ok),
            Err(_) => Ok(ParseSuccess {
                tokens: Vec::new(),
                rest: input,
                cursor,
                context,
            }),
        },

        Ast::Choice(choices) => {
            let mut reasons = Vec::with_capacity(choices.len());
            for choice in choices {
                match run_ast(choice, input, cursor, context.clone()) {
                    Ok(ok) => return Ok(ok),
                    Err(err) => reasons.push(err.reason),
                }
            }
            let reason = if reasons.is_empty() {
                "choice has no options".to_string()
            } else {
                reasons.join(" or ")
            };
            Err(ParseFailure {
                reason,
                rest: input,
                cursor,
            })
        }

        Ast::Repeat { inner, min, max } => {
            let mut rest = input;
            let mut cur = cursor;
            let mut ctx = context;
            let mut tokens = Vec::new();
            let mut count = 0usize;
            loop {
                if let Some(max) = max {
                    if count >= *max {
                        break;
                    }
                }
                match run_ast(inner, rest, cur, ctx.clone()) {
                    Ok(ok) => {
                        if ok.rest.len() == rest.len() {
                            break;
                        }
                        tokens.extend(ok.tokens);
                        rest = ok.rest;
                        cur = ok.cursor;
                        ctx = ok.context;
                        count += 1;
                    }
                    Err(err) => {
                        if count < *min {
                            return Err(err);
                        }
                        break;
                    }
                }
            }
            if count < *min {
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
                context: ctx,
            })
        }

        Ast::Duplicate { inner, n } => {
            let mut rest = input;
            let mut cur = cursor;
            let mut ctx = context;
            let mut tokens = Vec::new();
            for _ in 0..*n {
                let ok = run_ast(inner, rest, cur, ctx)?;
                tokens.extend(ok.tokens);
                rest = ok.rest;
                cur = ok.cursor;
                ctx = ok.context;
            }
            Ok(ParseSuccess {
                tokens,
                rest,
                cursor: cur,
                context: ctx,
            })
        }

        Ast::Eventually(inner) => {
            let mut rest = input;
            let mut cur = cursor;
            loop {
                if let Ok(ok) = run_ast(inner, rest, cur, context.clone()) {
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
        }

        Ast::Lookahead(inner) => {
            run_ast(inner, input, cursor, context.clone()).map(|_| ParseSuccess {
                tokens: Vec::new(),
                rest: input,
                cursor,
                context,
            })
        }

        Ast::LookaheadNot(inner) => match run_ast(inner, input, cursor, context.clone()) {
            Ok(_) => Err(ParseFailure {
                reason: "did not expect lookahead parser to match".to_string(),
                rest: input,
                cursor,
            }),
            Err(_) => Ok(ParseSuccess {
                tokens: Vec::new(),
                rest: input,
                cursor,
                context,
            }),
        },

        Ast::RepeatWhile {
            inner,
            while_fn,
            min,
            max,
        } => {
            let mut rest = input;
            let mut cur = cursor;
            let mut ctx = context;
            let mut tokens = Vec::new();
            let mut count = 0usize;
            loop {
                if let Some(max) = max {
                    if count >= *max {
                        break;
                    }
                }
                match while_fn(rest, cur) {
                    RepeatWhileControl::Halt => break,
                    RepeatWhileControl::Cont => {}
                }
                match run_ast(inner, rest, cur, ctx.clone()) {
                    Ok(ok) => {
                        if ok.rest.len() == rest.len() {
                            break;
                        }
                        tokens.extend(ok.tokens);
                        rest = ok.rest;
                        cur = ok.cursor;
                        ctx = ok.context;
                        count += 1;
                    }
                    Err(_) => break,
                }
            }
            if count < *min {
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
                context: ctx,
            })
        }

        Ast::Map(inner, f) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: ok.tokens.into_iter().map(|v| f(v)).collect(),
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Reduce(inner, f) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: vec![f(ok.tokens)],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Tag(name, inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: vec![Value::Tagged((*name).to_string(), ok.tokens)],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::UnwrapAndTag(name, inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
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
                tokens: vec![Value::KeyValue((*name).to_string(), Box::new(value))],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Wrap(inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: vec![Value::List(ok.tokens)],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Replace(inner, value) => {
            let ok = run_ast(inner, input, cursor, context)?;
            Ok(ParseSuccess {
                tokens: vec![value.clone()],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Label(inner, lbl) => {
            run_ast(inner, input, cursor, context).map_err(|err| ParseFailure {
                reason: format!("expected {lbl}"),
                rest: err.rest,
                cursor: err.cursor,
            })
        }

        Ast::ByteOffset(inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
            let token = Value::List(vec![
                Value::List(ok.tokens),
                Value::Int(BigInt::from(ok.cursor.byte_offset)),
            ]);
            Ok(ParseSuccess {
                tokens: vec![token],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Line(inner) => {
            let ok = run_ast(inner, input, cursor, context)?;
            let position = Value::List(vec![
                Value::Int(BigInt::from(ok.cursor.line)),
                Value::Int(BigInt::from(ok.cursor.line_start_offset)),
            ]);
            let token = Value::List(vec![Value::List(ok.tokens), position]);
            Ok(ParseSuccess {
                tokens: vec![token],
                rest: ok.rest,
                cursor: ok.cursor,
                context: ok.context,
            })
        }

        Ast::Debug(inner) => {
            eprintln!("debug: parsing {input:?} at {cursor:?}");
            let result = run_ast(inner, input, cursor, context);
            match &result {
                Ok(ok) => eprintln!("debug: ok tokens={:?} rest={:?}", ok.tokens, ok.rest),
                Err(err) => eprintln!("debug: error {:?}", err.reason),
            }
            result
        }

        Ast::PostTraverse(inner, f) => {
            let ok = run_ast(inner, input, cursor, context)?;
            match f(ok.tokens, ok.context, ok.cursor) {
                Ok((tokens, context)) => Ok(ParseSuccess {
                    tokens,
                    rest: ok.rest,
                    cursor: ok.cursor,
                    context,
                }),
                Err(reason) => Err(ParseFailure {
                    reason,
                    rest: ok.rest,
                    cursor: ok.cursor,
                }),
            }
        }

        Ast::PreTraverse(inner, f) => {
            let before = cursor;
            let ok = run_ast(inner, input, cursor, context)?;
            match f(ok.tokens, ok.context, before) {
                Ok((tokens, context)) => Ok(ParseSuccess {
                    tokens,
                    rest: ok.rest,
                    cursor: ok.cursor,
                    context,
                }),
                Err(reason) => Err(ParseFailure {
                    reason,
                    rest: ok.rest,
                    cursor: ok.cursor,
                }),
            }
        }

        Ast::Reference(cell) => {
            let inner = cell
                .get()
                .expect("parsec reference used before it was defined");
            run_ast(inner, input, cursor, context)
        }
    }
}

fn generate_ast(ast: &Arc<Ast>, rng: &mut StdRng, depth: usize, config: &GenerateConfig) -> String {
    match ast.as_ref() {
        Ast::Empty | Ast::Fail(_) | Ast::Eos => String::new(),

        Ast::Str(lit) => (*lit).to_string(),

        Ast::AsciiChar { predicates, .. } => gen_ascii_byte(predicates, rng)
            .map(|b| (b as char).to_string())
            .unwrap_or_default(),

        Ast::Utf8Char { predicates, .. } => gen_utf8_char(predicates, rng)
            .map(String::from)
            .unwrap_or_default(),

        Ast::AsciiString {
            predicates,
            min,
            max,
        } => {
            let mut out = String::new();
            for _ in 0..gen_count(*min, *max, rng, config) {
                match gen_ascii_byte(predicates, rng) {
                    Some(b) => out.push(b as char),
                    None => break,
                }
            }
            out
        }

        Ast::Utf8String {
            predicates,
            min,
            max,
        } => {
            let mut out = String::new();
            for _ in 0..gen_count(*min, *max, rng, config) {
                match gen_utf8_char(predicates, rng) {
                    Some(c) => out.push(c),
                    None => break,
                }
            }
            out
        }

        Ast::Bytes(n) => (0..*n)
            .map(|_| (b'a' + rng.gen_range(0..26)) as char)
            .collect(),

        Ast::Integer { min, max } => {
            let lo = (*min).max(1);
            let count = gen_count(lo, max.map(|m| m.max(lo)), rng, config).max(1);
            (0..count)
                .map(|_| (b'0' + rng.gen_range(0..10)) as char)
                .collect()
        }

        Ast::Concat(left, right) => {
            generate_ast(left, rng, depth, config) + &generate_ast(right, rng, depth, config)
        }

        Ast::Ignore(inner)
        | Ast::Eventually(inner)
        | Ast::Map(inner, _)
        | Ast::Reduce(inner, _)
        | Ast::Tag(_, inner)
        | Ast::UnwrapAndTag(_, inner)
        | Ast::Wrap(inner)
        | Ast::Replace(inner, _)
        | Ast::Label(inner, _)
        | Ast::ByteOffset(inner)
        | Ast::Line(inner)
        | Ast::Debug(inner)
        | Ast::PostTraverse(inner, _)
        | Ast::PreTraverse(inner, _) => generate_ast(inner, rng, depth, config),

        // Zero-width assertions consume no input, so they contribute nothing.
        Ast::Lookahead(_) | Ast::LookaheadNot(_) => String::new(),

        Ast::Optional(inner) => {
            if depth < config.max_recursion_depth && rng.gen_bool(0.5) {
                generate_ast(inner, rng, depth, config)
            } else {
                String::new()
            }
        }

        Ast::Choice(choices) => {
            if choices.is_empty() {
                return String::new();
            }
            let idx = if depth >= config.max_recursion_depth {
                // Prefer a branch that cannot recurse, so generation terminates.
                choices
                    .iter()
                    .position(|c| !contains_reference(c))
                    .unwrap_or(0)
            } else {
                rng.gen_range(0..choices.len())
            };
            generate_ast(&choices[idx], rng, depth, config)
        }

        Ast::Repeat { inner, min, max } => {
            let mut out = String::new();
            for _ in 0..gen_count(*min, *max, rng, config) {
                out.push_str(&generate_ast(inner, rng, depth, config));
            }
            out
        }

        Ast::Duplicate { inner, n } => {
            let mut out = String::new();
            for _ in 0..*n {
                out.push_str(&generate_ast(inner, rng, depth, config));
            }
            out
        }

        // The while predicate is opaque, so a count above the minimum may parse
        // short; this is best-effort sampling within the configured window.
        Ast::RepeatWhile {
            inner, min, max, ..
        } => {
            let mut out = String::new();
            for _ in 0..gen_count(*min, *max, rng, config) {
                out.push_str(&generate_ast(inner, rng, depth, config));
            }
            out
        }

        Ast::Reference(cell) => {
            if depth >= config.max_recursion_depth {
                String::new()
            } else {
                match cell.get() {
                    Some(inner) => generate_ast(inner, rng, depth + 1, config),
                    None => String::new(),
                }
            }
        }
    }
}

/// Chooses a repetition count within `[min, max]`, defaulting an unbounded
/// `max` to `config.repeat_window` above `min`.
fn gen_count(min: usize, max: Option<usize>, rng: &mut StdRng, config: &GenerateConfig) -> usize {
    let upper = max.unwrap_or(min + config.repeat_window).max(min);
    if upper == min {
        min
    } else {
        rng.gen_range(min..=upper)
    }
}

fn gen_ascii_byte(predicates: &[AsciiPredicate], rng: &mut StdRng) -> Option<u8> {
    let candidates: Vec<u8> = (0u8..=0x7f)
        .filter(|b| matches_ascii(*b, predicates))
        .collect();
    if candidates.is_empty() {
        None
    } else {
        Some(candidates[rng.gen_range(0..candidates.len())])
    }
}

fn gen_utf8_char(predicates: &[Utf8Predicate], rng: &mut StdRng) -> Option<char> {
    let mut candidates: Vec<char> = Vec::new();
    for p in predicates {
        match p {
            Utf8Predicate::Range(r) => {
                candidates.push(*r.start());
                candidates.push(*r.end());
            }
            Utf8Predicate::Char(c) => candidates.push(*c),
            _ => {}
        }
    }
    if candidates.is_empty() {
        candidates.extend(['a', 'b', 'c', '0', '9', ' ']);
    }
    candidates.retain(|c| matches_utf8(*c, predicates));
    if candidates.is_empty() {
        None
    } else {
        Some(candidates[rng.gen_range(0..candidates.len())])
    }
}

/// Detects whether `ast` directly contains a reference node, without following
/// references (so the walk stays finite even for recursive grammars).
fn contains_reference(ast: &Arc<Ast>) -> bool {
    match ast.as_ref() {
        Ast::Reference(_) => true,
        Ast::Concat(a, b) => contains_reference(a) || contains_reference(b),
        Ast::Choice(choices) => choices.iter().any(contains_reference),
        Ast::Ignore(i)
        | Ast::Optional(i)
        | Ast::Eventually(i)
        | Ast::Lookahead(i)
        | Ast::LookaheadNot(i)
        | Ast::Wrap(i)
        | Ast::ByteOffset(i)
        | Ast::Line(i)
        | Ast::Debug(i)
        | Ast::Map(i, _)
        | Ast::Reduce(i, _)
        | Ast::Replace(i, _)
        | Ast::Label(i, _)
        | Ast::Tag(_, i)
        | Ast::UnwrapAndTag(_, i)
        | Ast::PostTraverse(i, _)
        | Ast::PreTraverse(i, _)
        | Ast::Repeat { inner: i, .. }
        | Ast::Duplicate { inner: i, .. }
        | Ast::RepeatWhile { inner: i, .. } => contains_reference(i),
        _ => false,
    }
}

/// Renders a printable byte as `"x"` (matching Elixir's `inspect/1` of a
/// one-character binary) or, for control bytes, as `byte N`.
fn quote_byte(b: u8) -> String {
    if b.is_ascii_graphic() || b == b' ' {
        format!("\"{}\"", b as char)
    } else {
        format!("byte {b}")
    }
}

/// Renders a printable codepoint as `"x"`, or a control codepoint as
/// `codepoint N`.
fn quote_char(c: char) -> String {
    if c.is_control() {
        format!("codepoint {}", c as u32)
    } else {
        format!("\"{c}\"")
    }
}

/// Composes a NimbleParsec-style label: `<prefix> <incl> or <incl>, and not
/// <excl>`. An empty constraint set yields just the prefix.
fn compose_label(prefix: &str, inclusive: Vec<String>, exclusive: Vec<String>) -> String {
    let mut label = prefix.to_string();
    if !inclusive.is_empty() {
        label.push(' ');
        label.push_str(&inclusive.join(" or "));
    }
    for excl in exclusive {
        label.push_str(", and not ");
        label.push_str(&excl);
    }
    label
}

fn describe_ascii(predicates: &[AsciiPredicate]) -> String {
    let mut inclusive = Vec::new();
    let mut exclusive = Vec::new();
    for p in predicates {
        match p {
            AsciiPredicate::Any => {}
            AsciiPredicate::Range(r) => inclusive.push(format!(
                "in the range {} to {}",
                quote_byte(*r.start()),
                quote_byte(*r.end())
            )),
            AsciiPredicate::Char(c) => inclusive.push(format!("equal to {}", quote_byte(*c))),
            AsciiPredicate::NotRange(r) => exclusive.push(format!(
                "in the range {} to {}",
                quote_byte(*r.start()),
                quote_byte(*r.end())
            )),
            AsciiPredicate::NotChar(c) => exclusive.push(format!("equal to {}", quote_byte(*c))),
        }
    }
    compose_label("ASCII character", inclusive, exclusive)
}

fn describe_utf8(predicates: &[Utf8Predicate]) -> String {
    let mut inclusive = Vec::new();
    let mut exclusive = Vec::new();
    for p in predicates {
        match p {
            Utf8Predicate::Any => {}
            Utf8Predicate::Range(r) => inclusive.push(format!(
                "in the range {} to {}",
                quote_char(*r.start()),
                quote_char(*r.end())
            )),
            Utf8Predicate::Char(c) => inclusive.push(format!("equal to {}", quote_char(*c))),
            Utf8Predicate::NotRange(r) => exclusive.push(format!(
                "in the range {} to {}",
                quote_char(*r.start()),
                quote_char(*r.end())
            )),
            Utf8Predicate::NotChar(c) => exclusive.push(format!("equal to {}", quote_char(*c))),
        }
    }
    compose_label("utf8 codepoint", inclusive, exclusive)
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
