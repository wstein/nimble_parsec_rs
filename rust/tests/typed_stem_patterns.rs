//! Compile-and-run verification of the typed patterns used to migrate Stem's
//! `np_lexer` / `np_expr` reference grammars: `byte_offset` + `map` into a typed
//! struct/enum (carrying the end offset, replacing the old `post_traverse`
//! end-position injection), heterogeneous tuple `choice`, `try_map` validation
//! inside a `choice` branch (the raw-block name check), a many-arm homogeneous
//! array `choice` under `lookahead_not`, and `recursive` yielding a `String`.

use nimble_parsec_rs::nimble::{
    any, byte_offset, choice, lookahead_not, recursive, repeat, string, Parser,
};
use nimble_parsec_rs::typed::{one_of, take_while1};

#[derive(Debug, PartialEq)]
enum Lexeme {
    Text(String),
    Tag(String),
    Raw(String),
}

fn chars_until(stop: &'static str) -> impl Parser<&'static str, Output = String> {
    repeat(lookahead_not(string(stop)).ignore_then(any()))
        .map(|cs: Vec<char>| cs.into_iter().collect())
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

// `{{ ... }}` → a Tag lexeme paired with the end byte offset.
fn tag() -> impl Parser<&'static str, Output = (Lexeme, usize)> {
    byte_offset(
        string("{{")
            .ignore_then(chars_until("}}"))
            .then_ignore(string("}}")),
    )
    .map(|(inner, end)| (Lexeme::Tag(inner), end))
}

// `{{{{#name}}}}…{{{{/name}}}}` with the open/close name match validated in a
// `try_map` — a failed match makes the branch fail and `choice` backtrack.
fn raw() -> impl Parser<&'static str, Output = (Lexeme, usize)> {
    byte_offset(
        string("{{{{#")
            .ignore_then(take_while1(is_name))
            .then_ignore(string("}}}}"))
            .then(chars_until("{{{{/"))
            .then_ignore(string("{{{{/"))
            .then(take_while1(is_name))
            .then_ignore(string("}}}}"))
            .try_map(|((open, content), close): ((&str, String), &str)| {
                if open == close {
                    Ok(content)
                } else {
                    Err(format!("raw block {open:?} closed by {close:?}"))
                }
            }),
    )
    .map(|(content, end)| (Lexeme::Raw(content), end))
}

fn text() -> impl Parser<&'static str, Output = (Lexeme, usize)> {
    byte_offset(
        lookahead_not(string("{{"))
            .ignore_then(any())
            .repeated_at_least(1)
            .map(|cs: Vec<char>| cs.into_iter().collect::<String>()),
    )
    .map(|(run, end)| (Lexeme::Text(run), end))
}

fn lex() -> impl Parser<&'static str, Output = Vec<(Lexeme, usize)>> {
    repeat(choice((raw(), tag(), text())))
}

#[test]
fn lexer_patterns_produce_typed_lexemes_with_offsets() {
    let units = lex().parse("Hi {{x}}{{{{#r}}}}y{{{{/r}}}}").unwrap();
    let kinds: Vec<&Lexeme> = units.iter().map(|(u, _)| u).collect();
    assert_eq!(
        kinds,
        vec![
            &Lexeme::Text("Hi ".to_string()),
            &Lexeme::Tag("x".to_string()),
            &Lexeme::Raw("y".to_string()),
        ]
    );
    // The paired offsets are the running unit boundaries.
    assert_eq!(units[0].1, 3); // "Hi "
    assert_eq!(units[1].1, 8); // through "{{x}}"
}

#[test]
fn raw_block_name_mismatch_makes_choice_fall_through() {
    // open "r" vs close "s" → the `try_map` rejects, so the `raw` branch fails and
    // `choice` backtracks: the input is then lexed as ordinary tags/text, with no
    // `Raw` lexeme produced (mirroring the Value-based lexer's fall-through).
    let units = lex().parse("{{{{#r}}}}y{{{{/s}}}}").unwrap();
    assert!(
        !units.iter().any(|(u, _)| matches!(u, Lexeme::Raw(_))),
        "name mismatch must not yield a Raw lexeme: {units:?}"
    );
}

// ── Expression-side patterns: recursive → String, 8-arm tuple choice → enum ──

#[derive(Debug, PartialEq)]
enum Tok {
    Text(String),
    Ws(char),
    Pipe,
}

// Balanced parens captured as their raw source — `recursive` yielding `String`.
fn paren() -> impl Parser<&'static str, Output = String> {
    recursive(|paren| {
        string("(")
            .then(
                choice((
                    paren,
                    lookahead_not(choice([string("("), string(")")]))
                        .ignore_then(any())
                        .map(|c: char| c.to_string()),
                ))
                .repeated(),
            )
            .then(string(")").optional())
            .map(
                |((open, parts), close): ((&str, Vec<String>), Option<&str>)| {
                    let mut s = String::from(open);
                    parts.iter().for_each(|p| s.push_str(p));
                    s.push_str(close.unwrap_or(""));
                    s
                },
            )
    })
}

fn text_char() -> impl Parser<&'static str, Output = char> {
    // A many-arm homogeneous array `choice` under `lookahead_not`.
    lookahead_not(choice([string("|"), string(" "), string("(")])).ignore_then(any())
}

fn text_part() -> impl Parser<&'static str, Output = Tok> {
    choice((paren(), text_char().map(|c| c.to_string())))
        .repeated_at_least(1)
        .map(|frags: Vec<String>| Tok::Text(frags.concat()))
}

fn top() -> impl Parser<&'static str, Output = Vec<Tok>> {
    // Eight arms — the arity `np_expr`'s `top` needs, and the tuple-`choice` max.
    repeat(choice((
        string("||").map(|_| Tok::Pipe),
        string("&&").map(|_| Tok::Pipe),
        string("|").map(|_| Tok::Pipe),
        string(",").map(|_| Tok::Pipe),
        string("=").map(|_| Tok::Pipe),
        string(":").map(|_| Tok::Pipe),
        one_of(" \t").map(Tok::Ws),
        text_part(),
    )))
}

#[test]
fn expression_patterns_split_top_level_with_atomic_parens() {
    assert_eq!(
        top().parse("a | (b c)").unwrap(),
        vec![
            Tok::Text("a".to_string()),
            Tok::Ws(' '),
            Tok::Pipe,
            Tok::Ws(' '),
            Tok::Text("(b c)".to_string()), // the paren chunk is atomic
        ]
    );
}
