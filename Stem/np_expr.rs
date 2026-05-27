// SPDX-License-Identifier: Apache-2.0
//
// Expression top-level tokenizer — a `nimble_parsec_rs` port of the
// `Stem.Expression` grammar (the BEAM's NimbleParsec `paren_chunk` /
// `top_level_text_part`). It splits a tag's inner text into top-level `Tok`s
// (text runs plus the `|` / `||` / `&&` / `,` / `=` / `:` / whitespace
// separators), treating quoted strings, parenthesised sub-expressions, and
// bracketed literal keys as atomic — so separators inside them are part of the
// text, not delimiters. The downstream structural parser is unchanged; this
// only replaces the hand-written `scan_top_level` scanner.
//
// ─────────────────────────────────────────────────────────────────────────────
// Read alongside `np_lexer.rs`: together the two files are a worked example of
// `nimble_parsec_rs`. Where the lexer leans on `post_traverse` and `tag`, this
// file is the home of the crate's *recursion* support — `recursive` builds the
// self-referential `paren` grammar that the lexer never needs. It also shows the
// binary `.or` (sugar for a two-way `choice`), `.optional`, and `Utf8Predicate`.
//
// Combinators demonstrated here (see `np_lexer.rs` for the full coverage map):
//   recursive / ParserRef     — the balanced-paren grammar refers to itself
//   choice / `.or`            — ordered alternation (n-way and the 2-way sugar)
//   optional (`.optional`)    — tolerate a missing closing delimiter
//   lookahead_not             — "any char that doesn't start an atomic chunk"
//   repeat (`.repeated`)      — maximal runs
//   concat (`.then`), ignore (`.ignored`), reduce (`.reduce`), tag (`.tagged`)
//   utf8_char + Utf8Predicate — the whitespace class

use nimble_parsec_rs::{
    choice, lookahead_not, recursive, string, utf8_char, Parser, Utf8Predicate, Value,
};

use crate::Tok;

// Concatenate a combinator's emitted values (literal `string` parts as-is,
// `utf8_char` codepoints as their char) back into the matched source text — the
// `reduce` callback that turns an atomic chunk into one `Value::Str`.
fn values_to_string(tokens: Vec<Value>) -> Value {
    let mut text = String::new();
    for value in tokens {
        match value {
            Value::Str(part) => text.push_str(&part),
            Value::Int(codepoint) => {
                if let Some(c) = codepoint
                    .to_string()
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                {
                    text.push(c);
                }
            }
            _ => {}
        }
    }
    Value::Str(text)
}

// A double/single-quoted chunk: the delimiter, a run where `\` escapes the next
// char, then the closing delimiter (tolerated-optional, like the BEAM). Atomic.
// The inner two-way alternation is written with the fluent `.or` (a `choice` of
// exactly two); `.optional` makes the closing delimiter forgiving.
fn quoted(delim: &'static str) -> Parser {
    string(delim)
        .then(
            string("\\")
                .then(utf8_char(vec![]))
                .or(lookahead_not(string(delim)).then(utf8_char(vec![])))
                .repeated(0, None),
        )
        .then(string(delim).optional())
        .reduce(values_to_string)
}

// A bracketed literal key `[ ... ]`: content runs to the first `]`, no nesting
// or escapes. Atomic.
fn bracket() -> Parser {
    string("[")
        .then(
            lookahead_not(string("]"))
                .then(utf8_char(vec![]))
                .repeated(0, None),
        )
        .then(string("]").optional())
        .reduce(values_to_string)
}

// A parenthesised sub-expression: balanced parens with nested quotes/brackets,
// reduced to its raw source. `recursive` hands the closure a handle to the
// parser being defined, so `paren` can appear inside its own body — mirroring
// `Stem.Expression.paren_chunk`. (Recursion depth is bounded by the crate; see
// `Parser::parse_with_max_depth`.)
fn paren() -> Parser {
    recursive(|paren| {
        string("(")
            .then(
                choice(vec![
                    paren,
                    quoted("\""),
                    quoted("'"),
                    bracket(),
                    lookahead_not(choice(vec![
                        string("("),
                        string(")"),
                        string("\""),
                        string("'"),
                    ]))
                    .then(utf8_char(vec![])),
                ])
                .repeated(0, None),
            )
            .then(string(")").optional())
            .reduce(values_to_string)
    })
}

// One non-separator character (separators and atomic openers are handled
// elsewhere). `&&` is excluded as a unit, but a lone `&` is ordinary text.
fn text_char() -> Parser {
    lookahead_not(choice(vec![
        string("&&"),
        string("|"),
        string(","),
        string("="),
        string(":"),
        string("\t"),
        string("\n"),
        string("\r"),
        string(" "),
        string("\""),
        string("'"),
        string("("),
        string("["),
    ]))
    .then(utf8_char(vec![]))
}

// A maximal run of text: atomic chunks and plain chars, reduced to one string.
fn text_part() -> Parser {
    choice(vec![
        quoted("\""),
        quoted("'"),
        paren(),
        bracket(),
        text_char(),
    ])
    .repeated(1, None)
    .reduce(values_to_string)
    .tagged("text")
}

// A separator literal: matched, its token discarded (`.ignored`), and emitted as
// an empty tagged marker the assembler maps to a `Tok`.
fn separator(literal: &'static str, tag: &'static str) -> Parser {
    string(literal).ignored().tagged(tag)
}

fn whitespace() -> Parser {
    utf8_char(vec![
        Utf8Predicate::Char('\t'),
        Utf8Predicate::Char('\n'),
        Utf8Predicate::Char('\r'),
        Utf8Predicate::Char(' '),
    ])
    .tagged("ws")
}

fn top() -> Parser {
    choice(vec![
        // `||`/`&&` before `|` (maximal munch); a lone `&` falls to text.
        string("||").tagged("reserved"),
        string("&&").tagged("reserved"),
        separator("|", "pipe"),
        separator(",", "comma"),
        separator("=", "eq"),
        separator(":", "colon"),
        whitespace(),
        text_part(),
    ])
    .repeated(0, None)
}

// Tokenize a tag's inner text to top-level `Tok`s, matching `scan_top_level`.
pub(crate) fn scan_top_level(source: &str) -> Vec<Tok> {
    match top().parse(source) {
        Ok(success) => success
            .tokens
            .into_iter()
            .filter_map(tok_from_value)
            .collect(),
        // The grammar consumes every char (any non-separator is text), so a parse
        // failure is unreachable; degrade to a single text token rather than panic.
        Err(_) => vec![Tok::Text(source.to_string())],
    }
}

fn tok_from_value(value: Value) -> Option<Tok> {
    let Value::Tagged(name, items) = value else {
        return None;
    };
    let first_str = || match items.first() {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    };
    Some(match name.as_str() {
        "text" => Tok::Text(first_str()),
        "pipe" => Tok::Pipe,
        "comma" => Tok::Comma,
        "eq" => Tok::Eq,
        "colon" => Tok::Colon,
        "reserved" => match first_str().as_str() {
            "&&" => Tok::Reserved("&&"),
            _ => Tok::Reserved("||"),
        },
        "ws" => {
            let c = match items.first() {
                Some(Value::Int(cp)) => cp.to_string().parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            Tok::Ws(c.unwrap_or(' '))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tok::{Colon, Comma, Eq, Pipe, Reserved, Text, Ws};

    fn text(s: &str) -> Tok {
        Text(s.to_string())
    }

    #[test]
    fn splits_pipes_and_whitespace() {
        assert_eq!(
            scan_top_level("name | upcase"),
            vec![text("name"), Ws(' '), Pipe, Ws(' '), text("upcase")]
        );
    }

    #[test]
    fn reserved_operators_are_maximal_munch_but_lone_amp_is_text() {
        assert_eq!(
            scan_top_level("a || b"),
            vec![text("a"), Ws(' '), Reserved("||"), Ws(' '), text("b")]
        );
        assert_eq!(
            scan_top_level("a&&b"),
            vec![text("a"), Reserved("&&"), text("b")]
        );
        // A lone `&` (not `&&`) is ordinary text.
        assert_eq!(
            scan_top_level("a & b"),
            vec![text("a"), Ws(' '), text("&"), Ws(' '), text("b")]
        );
    }

    #[test]
    fn quotes_parens_brackets_are_atomic() {
        // Separators inside quoted/paren/bracket chunks are part of the text.
        assert_eq!(
            scan_top_level("default 'a b' c"),
            vec![text("default"), Ws(' '), text("'a b'"), Ws(' '), text("c")]
        );
        assert_eq!(scan_top_level("[first-name]"), vec![text("[first-name]")]);
        assert_eq!(
            scan_top_level("upcase (trim name)"),
            vec![text("upcase"), Ws(' '), text("(trim name)")]
        );
        assert_eq!(scan_top_level("[a|b]"), vec![text("[a|b]")]);
    }

    #[test]
    fn keyword_argument_separators() {
        assert_eq!(
            scan_top_level("t key=value"),
            vec![text("t"), Ws(' '), text("key"), Eq, text("value")]
        );
        assert_eq!(scan_top_level("a:b"), vec![text("a"), Colon, text("b")]);
        assert_eq!(scan_top_level("a,b"), vec![text("a"), Comma, text("b")]);
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(scan_top_level("").is_empty());
    }
}
