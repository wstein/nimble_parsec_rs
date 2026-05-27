//! End-to-end proof that the typed API expresses a real grammar idiomatically
//! (RFC 0001 phase 4): a template lexer equivalent to Stem's `np_lexer`, built
//! entirely from typed combinators and producing a typed token `enum` — no
//! `Value` tagging, no post-hoc `match` on dynamic terms.

use nimble_parsec_rs::typed::{any, literal, not, Parser};

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Tag(String),
}

// A run of characters up to (but not consuming) `stop`, collected into a String.
// This is the typed equivalent of the `lookahead_not(stop) |> utf8_char` +
// `reduce(to_string)` idiom in the Value-based lexer.
fn chars_until<'i>(stop: &'static str) -> impl Parser<'i, Output = String> {
    not(literal(stop))
        .ignore_then(any())
        .repeated()
        .map(|chars: Vec<char>| chars.into_iter().collect())
}

// `{{! … }}` — a comment, lexed and then dropped (`None`).
fn comment<'i>() -> impl Parser<'i, Output = Option<Token>> {
    literal("{{!")
        .ignore_then(chars_until("}}"))
        .then_ignore(literal("}}"))
        .map(|_| None)
}

// `{{ … }}` — a tag; its inner text is kept.
fn tag<'i>() -> impl Parser<'i, Output = Option<Token>> {
    literal("{{")
        .ignore_then(chars_until("}}"))
        .then_ignore(literal("}}"))
        .map(|inner| Some(Token::Tag(inner)))
}

// A maximal run of text up to the next `{{`.
fn text<'i>() -> impl Parser<'i, Output = Option<Token>> {
    not(literal("{{"))
        .ignore_then(any())
        .repeated_at_least(1)
        .map(|chars: Vec<char>| Some(Token::Text(chars.into_iter().collect())))
}

// The whole document: comments (dropped) before tags before text, repeated and
// the dropped comments filtered out.
fn template<'i>() -> impl Parser<'i, Output = Vec<Token>> {
    comment()
        .or(tag())
        .or(text())
        .repeated()
        .map(|items: Vec<Option<Token>>| items.into_iter().flatten().collect())
}

#[test]
fn lexes_text_and_tags_into_typed_tokens() {
    let tokens = template().parse("Hi {{name}}!").unwrap();
    assert_eq!(
        tokens,
        vec![
            Token::Text("Hi ".to_string()),
            Token::Tag("name".to_string()),
            Token::Text("!".to_string()),
        ]
    );
}

#[test]
fn comments_are_dropped_and_surrounding_text_kept() {
    let tokens = template().parse("a{{! hidden }}b{{x}}").unwrap();
    assert_eq!(
        tokens,
        vec![
            Token::Text("a".to_string()),
            Token::Text("b".to_string()),
            Token::Tag("x".to_string()),
        ]
    );
}

#[test]
fn an_unterminated_tag_fails_to_parse() {
    // `chars_until("}}")` consumes to end without finding `}}`, so the trailing
    // `literal("}}")` fails and (since text can't start at `{{`) so does the
    // whole parse — input is left unconsumed.
    assert!(template().parse("ok {{oops").is_err());
}
