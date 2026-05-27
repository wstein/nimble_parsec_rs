// SPDX-License-Identifier: Apache-2.0
//
// Expression top-level tokenizer — a `nimble_parsec_rs` port of the
// `Stem.Expression` grammar (the BEAM's NimbleParsec `paren_chunk` /
// `top_level_text_part`; see `Stem/parser.ex` for the sibling lexer). Built on
// the crate's **typed** `Parser<Output>` API: each combinator yields a typed
// value, so `top` produces a `Vec<Tok>` directly with no `Value` tagging or
// post-parse decode.
//
// It splits a tag's inner text into top-level `Tok`s (text runs plus the
// `|` / `||` / `&&` / `,` / `=` / `:` / whitespace separators), treating quoted
// strings, parenthesised sub-expressions, and bracketed literal keys as atomic —
// so separators inside them are part of the text, not delimiters.
//
// NimbleParsec terminology is kept where the crate offers it
// (`nimble_parsec_rs::nimble`): `string` / `choice` / `lookahead_not` / `repeat`
// / `recursive`. The typed leaves/methods fill the rest: `utf8_char([])` → `any`,
// the whitespace class → `one_of`, `reduce(to_string)` → `.map`, and `tag(..)` is
// unnecessary because the output is already a typed `Tok`.

use nimble_parsec_rs::nimble::{any, choice, lookahead_not, recursive, repeat, string, Parser};
use nimble_parsec_rs::typed::one_of;

use crate::Tok;

// A double/single-quoted chunk: the delimiter, a run where `\` escapes the next
// char, then the closing delimiter (tolerated-optional, like the BEAM). Atomic;
// reduced back to its raw source `String` (the backslash escapes are preserved).
fn quoted<'i>(delim: &'static str) -> impl Parser<'i, Output = String> {
    let content = choice((
        string("\\")
            .then(any())
            .map(|(bs, c): (&str, char)| format!("{bs}{c}")),
        lookahead_not(string(delim))
            .ignore_then(any())
            .map(|c| c.to_string()),
    ));
    string(delim)
        .then(content.repeated())
        .then(string(delim).optional())
        .map(join_chunk)
}

// A bracketed literal key `[ ... ]`: content runs to the first `]`, no nesting
// or escapes. Atomic.
fn bracket<'i>() -> impl Parser<'i, Output = String> {
    string("[")
        .then(
            lookahead_not(string("]"))
                .ignore_then(any())
                .map(|c| c.to_string())
                .repeated(),
        )
        .then(string("]").optional())
        .map(join_chunk)
}

// Reassembles `(opener, inner fragments, optional closer)` into the chunk's raw
// source — the typed form of `reduce({List, :to_string, []})`.
fn join_chunk((head, close): ((&str, Vec<String>), Option<&str>)) -> String {
    let (open, parts) = head;
    let mut source = String::from(open);
    parts.iter().for_each(|part| source.push_str(part));
    source.push_str(close.unwrap_or(""));
    source
}

// A parenthesised sub-expression: balanced parens with nested quotes/brackets,
// reduced to its raw source. `recursive` hands the closure a handle to the parser
// being defined, so `paren` can appear inside its own body — mirroring
// `Stem.Expression.paren_chunk`. (Recursion depth is bounded by the crate; see
// `Parser::parse_with_max_depth`.)
fn paren<'i>() -> impl Parser<'i, Output = String> {
    recursive(|paren| {
        // Any char that doesn't open/close a paren or start a quote.
        let other_char = lookahead_not(choice([
            string("("),
            string(")"),
            string("\""),
            string("'"),
        ]))
        .ignore_then(any())
        .map(|c| c.to_string());

        string("(")
            .then(choice((paren, quoted("\""), quoted("'"), bracket(), other_char)).repeated())
            .then(string(")").optional())
            .map(join_chunk)
    })
}

// One non-separator character (separators and atomic openers are handled
// elsewhere). `&&` is excluded as a unit, but a lone `&` is ordinary text.
fn text_char<'i>() -> impl Parser<'i, Output = char> {
    lookahead_not(choice([
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
    .ignore_then(any())
}

// A maximal run of text: atomic chunks and plain chars, reduced to one `Tok::Text`.
fn text_part<'i>() -> impl Parser<'i, Output = Tok> {
    choice((
        quoted("\""),
        quoted("'"),
        paren(),
        bracket(),
        text_char().map(|c| c.to_string()),
    ))
    .repeated_at_least(1)
    .map(|frags: Vec<String>| Tok::Text(frags.concat()))
}

// The top level: zero or more separators / whitespace / text runs. `||`/`&&` are
// tried before `|` (maximal munch); a lone `&` falls through to text.
fn top<'i>() -> impl Parser<'i, Output = Vec<Tok>> {
    repeat(choice((
        string("||").map(|_| Tok::Reserved("||")),
        string("&&").map(|_| Tok::Reserved("&&")),
        string("|").map(|_| Tok::Pipe),
        string(",").map(|_| Tok::Comma),
        string("=").map(|_| Tok::Eq),
        string(":").map(|_| Tok::Colon),
        one_of(" \t\n\r").map(Tok::Ws),
        text_part(),
    )))
}

// Tokenize a tag's inner text to top-level `Tok`s, matching `scan_top_level`.
pub(crate) fn scan_top_level(source: &str) -> Vec<Tok> {
    // The grammar consumes every char (any non-separator is text), so it always
    // succeeds and consumes all input; degrade to a single text token rather than
    // panic if that invariant is ever broken.
    match top().parse_partial(source) {
        Ok((toks, _rest)) => toks,
        Err(_) => vec![Tok::Text(source.to_string())],
    }
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
