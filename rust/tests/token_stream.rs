//! Two-phase parsing: a lexer produces `Vec<Token>`, then a grammar consumes
//! `Tokens<Token>` via the built-in `Stream` impl for `Tokens<'_, T>`.
//!
//! This tests the [`Tokens`] custom stream — the `Stream<Token = T>` path for
//! arbitrary token sequences, as opposed to `&str` (Token = char) or `&[u8]`
//! (Token = u8). The idiomatic use case is wiring a hand-written or generated
//! lexer into a second parser stage.
//!
//! The grammar is a minimal arithmetic expression language:
//!
//!   expr   = term   ( ('+' | '-') term  )*
//!   term   = factor ( ('*' | '/') factor)*
//!   factor = Number | '(' expr ')'
//!
//! Because general-purpose parser combinators in this crate are not `Clone`
//! (they contain closures), each grammar level is wrapped in `recursive` —
//! `Recursive<'a, S, O>` IS `Clone` (backed by `Rc`), making it safe to share
//! a handle across the sequencing operators that consume their arguments.

use nimble_parsec_rs::typed::{any, recursive, satisfy, take, Parser, Tokens};

// ── Token definition ──────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq)]
enum Token {
    Number(i64),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Eof,
}

// ── Lexer ─────────────────────────────────────────────────────────────────────

/// Turns a `&str` into `Vec<Token>` terminated by `Token::Eof`. Ignores
/// whitespace.
fn lex(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' => {
                chars.next();
            }
            '+' => {
                chars.next();
                tokens.push(Token::Plus);
            }
            '-' => {
                chars.next();
                tokens.push(Token::Minus);
            }
            '*' => {
                chars.next();
                tokens.push(Token::Star);
            }
            '/' => {
                chars.next();
                tokens.push(Token::Slash);
            }
            '(' => {
                chars.next();
                tokens.push(Token::LParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RParen);
            }
            '0'..='9' => {
                let mut n: i64 = 0;
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        n = n * 10 + (d as i64 - '0' as i64);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Number(n));
            }
            other => panic!("unexpected char: {other:?}"),
        }
    }
    tokens.push(Token::Eof);
    tokens
}

// ── Grammar helpers ───────────────────────────────────────────────────────────

fn tok<'i, F>(pred: F) -> impl Parser<Tokens<'i, Token>, Output = Token>
where
    F: Fn(Token) -> bool,
{
    satisfy("token", move |t: Token| pred(t))
}

fn number<'i>() -> impl Parser<Tokens<'i, Token>, Output = i64> {
    satisfy("number", |t: Token| matches!(t, Token::Number(_))).map(|t| {
        let Token::Number(n) = t else { unreachable!() };
        n
    })
}

// ── Expression grammar ────────────────────────────────────────────────────────

// Each grammar level wraps itself in `recursive` so its type is
// `Recursive<'_, Tokens<Token>, i64>`, which implements `Clone`. This lets the
// same sub-parser be used multiple times inside a combinator chain (e.g. both
// sides of a binary operator repetition).

fn expr<'i>() -> impl Parser<Tokens<'i, Token>, Output = i64> {
    recursive(|e| {
        // ── factor = Number | '(' expr ')' ────────────────────────────────
        let factor = recursive(|_f| {
            number::<'i>().or(tok(|t| t == Token::LParen)
                .ignore_then(e.clone())
                .then_ignore(tok(|t| t == Token::RParen)))
        });

        // ── term = factor ( ('*' | '/') factor )* ─────────────────────────
        let term = recursive(move |_t| {
            factor
                .clone()
                .then(
                    tok(|t| matches!(t, Token::Star | Token::Slash))
                        .then(factor.clone())
                        .repeated(),
                )
                .map(|(first, rest): (i64, Vec<(Token, i64)>)| {
                    rest.into_iter().fold(first, |acc, (op, val)| match op {
                        Token::Star => acc * val,
                        Token::Slash => acc / val,
                        _ => unreachable!(),
                    })
                })
        });

        // ── expr = term ( ('+' | '-') term )* ─────────────────────────────
        term.clone()
            .then(
                tok(|t| matches!(t, Token::Plus | Token::Minus))
                    .then(term.clone())
                    .repeated(),
            )
            .map(|(first, rest): (i64, Vec<(Token, i64)>)| {
                rest.into_iter().fold(first, |acc, (op, val)| match op {
                    Token::Plus => acc + val,
                    Token::Minus => acc - val,
                    _ => unreachable!(),
                })
            })
    })
}

fn eval(input: &str) -> i64 {
    let tokens = lex(input);
    // Bind the result to a local before the borrow of `tokens` ends.
    let result = expr()
        .then_ignore(tok(|t| t == Token::Eof))
        .parse(Tokens(&tokens))
        .unwrap();
    result
}

// ── Expression grammar tests ──────────────────────────────────────────────────

#[test]
fn token_stream_single_number() {
    assert_eq!(eval("42"), 42);
    assert_eq!(eval("0"), 0);
}

#[test]
fn token_stream_addition_and_subtraction() {
    assert_eq!(eval("1 + 2"), 3);
    assert_eq!(eval("10 - 3"), 7);
    assert_eq!(eval("1 + 2 + 3"), 6);
    assert_eq!(eval("5 - 2 - 1"), 2);
}

#[test]
fn token_stream_multiplication_and_division() {
    assert_eq!(eval("3 * 4"), 12);
    assert_eq!(eval("8 / 2"), 4);
    assert_eq!(eval("2 * 3 * 4"), 24);
}

#[test]
fn token_stream_operator_precedence() {
    assert_eq!(eval("2 + 3 * 4"), 14); // '*' binds tighter than '+'
    assert_eq!(eval("10 - 2 * 3"), 4);
}

#[test]
fn token_stream_parenthesized_expressions() {
    assert_eq!(eval("(2 + 3) * 4"), 20);
    assert_eq!(eval("(10 - 2) * (3 + 1)"), 32);
}

#[test]
fn token_stream_nested_parentheses() {
    assert_eq!(eval("((2 + 3))"), 5);
    assert_eq!(eval("(1 + (2 * (3 + 4)))"), 15);
}

#[test]
fn token_stream_whitespace_is_ignored() {
    assert_eq!(eval("  1   +   2  "), 3);
    assert_eq!(eval("1+2"), 3);
}

#[test]
fn token_stream_complex_expression() {
    // ((2 + 3) * (4 - 1)) / 5 = (5 * 3) / 5 = 3
    assert_eq!(eval("(2 + 3) * (4 - 1) / 5"), 3);
}

#[test]
fn tokens_stream_error_on_extra_tokens() {
    // "1 2": parses '1', but '2' is not an Eof — should fail.
    let tokens = lex("1 2");
    let result = expr()
        .then_ignore(tok(|t| t == Token::Eof))
        .parse(Tokens(&tokens));
    assert!(result.is_err());
}

// ── Raw Stream-level tests ────────────────────────────────────────────────────

#[test]
fn tokens_stream_any_consumes_one_token() {
    let tokens = lex("42");
    let (tok_val, _rest) = any::<Tokens<Token>>()
        .parse_partial(Tokens(&tokens))
        .unwrap();
    assert_eq!(tok_val, Token::Number(42));
}

#[test]
fn tokens_stream_take_returns_slice() {
    let tokens = lex("1 + 2");
    // `take(2)` on `Tokens<Token>` yields `&[Token; 2]`.
    let (first_two, _rest) = take::<Tokens<Token>>(2)
        .parse_partial(Tokens(&tokens))
        .unwrap();
    assert_eq!(first_two.len(), 2);
    assert_eq!(first_two[0], Token::Number(1));
    assert_eq!(first_two[1], Token::Plus);
}

#[test]
fn tokens_stream_repeated_collect_all_numbers() {
    let tokens = lex("1 + 22 + 333");
    // collect every `Number` token, ignoring others via `.optional()`.
    let numbers = number()
        .optional()
        .then_ignore(satisfy::<Tokens<Token>, _>("non-eof", |t: Token| t != Token::Eof).optional())
        .repeated()
        .map(|opts: Vec<Option<i64>>| opts.into_iter().flatten().collect::<Vec<_>>())
        .then_ignore(tok(|t| t == Token::Eof))
        .parse(Tokens(&tokens))
        .unwrap();
    assert_eq!(numbers, vec![1, 22, 333]);
}

#[test]
fn tokens_stream_satisfy_matches_specific_variant() {
    let tokens = lex("+ -");
    let pluses = satisfy::<Tokens<Token>, _>("plus", |t: Token| t == Token::Plus)
        .repeated()
        .parse_partial(Tokens(&tokens))
        .unwrap()
        .0;
    assert_eq!(pluses.len(), 1); // one '+', then '-' stops it
}

#[test]
fn tokens_stream_empty_token_sequence() {
    // Just the Eof sentinel — should parse OK with `tok(Eof)`.
    let tokens = lex("");
    let result = tok(|t: Token| t == Token::Eof).parse(Tokens(&tokens));
    assert!(result.is_ok());
}
