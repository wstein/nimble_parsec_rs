// SPDX-License-Identifier: Apache-2.0
//
// Combinator lexer — a `nimble_parsec_rs` port of `Stem.Parser`'s `do_lex`
// (see `Stem/parser.ex`), built on the crate's **typed** `Parser<Output>` API.
// `do_lex` (the combinator grammar) recognizes the raw lexical units (text runs,
// comments, raw blocks, `{{{ }}}` and `{{ }}` tags) and yields a typed `Lexeme`
// directly — no dynamic `Value` tagging or post-parse decode. `tokenize` then
// folds the `Lexeme`s into the `Token` stream, applying trim markers, backslash
// escapes, and tag classification, mirroring Elixir's `assemble_tokens`.
//
// Mapping from the Elixir NimbleParsec grammar (`Stem.Parser`):
//   * `string` / `ignore` / `repeat` / `lookahead_not` / `choice`  — kept as the
//     NimbleParsec-named free functions via `nimble_parsec_rs::nimble`.
//   * `utf8_char([])`                 → `any()`            (a single character)
//   * `ascii_string(ranges, min: 1)`  → `take_while1(pred)`
//   * `reduce({List, :to_string, []})`→ `.map(collect)`    (chars → String)
//   * `post_traverse(inject_end_pos)` → `byte_offset(..)`  (pairs the end offset)
//   * `tag(:name)`                    → `.map(|..| Lexeme { .. })` (a typed value)
//   * `post_traverse(validate_..)`    → `.try_map(..)`      (validate + reduce)

use nimble_parsec_rs::nimble::{any, byte_offset, choice, lookahead_not, repeat, string, Parser};
use nimble_parsec_rs::typed::take_while1;

use crate::{classify, extract_trim, flush_text, trim_trailing_text, CompileError, Token};

// One raw lexical unit and the byte offset just past it. The start of a unit is
// the end of the previous one (the units tile the source with no gaps).
#[derive(Debug)]
struct Lexeme {
    kind: LexKind,
    end: usize,
}

#[derive(Debug)]
enum LexKind {
    // A run of text not starting a `{{` tag.
    Text(String),
    // `{{!-- … --}}` / `{{! … }}`; dropped during assembly (text merges across).
    BlockComment,
    InlineComment,
    // `{{{{#name}}}}…{{{{/name}}}}`; the verbatim content (open/close names matched).
    RawBlock(String),
    // The inner text of a `{{{ … }}}` raw tag / a `{{ … }}` standard tag.
    RawTag(String),
    StandardTag(String),
}

// ── do_lex grammar (mirrors `Stem.Parser`'s combinators) ─────────────────────

// The character class for a tag/raw-block name: `[A-Za-z0-9_-]` — the Elixir
// `ascii_string([?a..?z, ?A..?Z, ?0..?9, ?_, ?-], min: 1)`.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn collect_string(chars: Vec<char>) -> String {
    chars.into_iter().collect()
}

// A run of characters up to (but not consuming) `stop`, reduced to a `String` —
// the typed form of `repeat(lookahead_not(stop) |> utf8_char([]))` +
// `reduce({List, :to_string, []})`.
fn chars_until<'i>(stop: &'static str) -> impl Parser<'i, Output = String> {
    repeat(lookahead_not(string(stop)).ignore_then(any())).map(collect_string)
}

// A maximal run of text, stopping before the next `{{` (at least one character).
// `byte_offset` pairs the text with the end position, then `.map` builds the
// `Lexeme` — replacing the Elixir `post_traverse(inject_end_pos) |> tag(:text)`.
fn text_chunk<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        lookahead_not(string("{{"))
            .ignore_then(any())
            .repeated_at_least(1)
            .map(collect_string),
    )
    .map(|(text, end)| Lexeme {
        kind: LexKind::Text(text),
        end,
    })
}

// `{{!-- … --}}` — opener, body, closer. The body is parsed (to find the closer)
// but discarded; only the unit and its end position survive.
fn block_comment<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        string("{{!--")
            .ignore_then(chars_until("--}}"))
            .then_ignore(string("--}}")),
    )
    .map(|(_body, end)| Lexeme {
        kind: LexKind::BlockComment,
        end,
    })
}

// `{{! … }}` — the single-brace inline comment variant.
fn inline_comment<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        string("{{!")
            .ignore_then(chars_until("}}"))
            .then_ignore(string("}}")),
    )
    .map(|(_body, end)| Lexeme {
        kind: LexKind::InlineComment,
        end,
    })
}

// `{{{{#name}}}}…{{{{/name}}}}` — `try_map` validates that the open/close names
// match (mirroring Elixir's `validate_and_reduce_raw_block`) and keeps only the
// content; a mismatch fails the branch and `choice` falls through.
fn raw_block<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        string("{{{{#")
            .ignore_then(take_while1(is_name_char))
            .then_ignore(string("}}}}"))
            .then(chars_until("{{{{/"))
            .then_ignore(string("{{{{/"))
            .then(take_while1(is_name_char))
            .then_ignore(string("}}}}"))
            .try_map(|((open, content), close): ((&str, String), &str)| {
                if open == close {
                    Ok(content)
                } else {
                    Err(format!(
                        "raw block open `{{{{{{{{#{open}}}}}}}}}` is closed by `{{{{{{{{/{close}}}}}}}}}`"
                    ))
                }
            }),
    )
    .map(|(content, end)| Lexeme {
        kind: LexKind::RawBlock(content),
        end,
    })
}

// `{{{ … }}}` — a triple-brace raw tag; its inner text is kept verbatim.
fn raw_tag<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        string("{{{")
            .ignore_then(chars_until("}}}"))
            .then_ignore(string("}}}")),
    )
    .map(|(inner, end)| Lexeme {
        kind: LexKind::RawTag(inner),
        end,
    })
}

// `{{ … }}` — the ordinary double-brace tag; inner text kept for classification.
fn standard_tag<'i>() -> impl Parser<'i, Output = Lexeme> {
    byte_offset(
        string("{{")
            .ignore_then(chars_until("}}"))
            .then_ignore(string("}}")),
    )
    .map(|(inner, end)| Lexeme {
        kind: LexKind::StandardTag(inner),
        end,
    })
}

// The whole document: zero or more lexical units. `choice` tries the alternatives
// in order (most-specific delimiters first so `{{{{` beats `{{{` beats `{{`, and
// text is the catch-all); `repeat` tiles the source into a `Vec<Lexeme>`.
fn do_lex<'i>() -> impl Parser<'i, Output = Vec<Lexeme>> {
    repeat(choice((
        block_comment(),
        inline_comment(),
        raw_block(),
        raw_tag(),
        standard_tag(),
        text_chunk(),
    )))
}

fn raw_lex(source: &str) -> Result<Vec<Lexeme>, CompileError> {
    // `do_lex` is `repeat(..)`, so it is total — it never returns `Err`; an
    // unconsumed remainder means an unterminated tag/comment/raw block.
    match do_lex().parse_partial(source) {
        Ok((lexemes, rest)) if rest.is_empty() => Ok(lexemes),
        Ok((_, rest)) => Err(CompileError {
            message: "unterminated tag while lexing template".to_string(),
            file: String::new(),
            start: source.len() - rest.len(),
            end: source.len(),
        }),
        Err(failure) => Err(CompileError {
            message: failure.reason,
            file: String::new(),
            start: failure.cursor.byte_offset,
            end: source.len(),
        }),
    }
}

// ── Assembler (mirrors `Stem.Parser.assemble_tokens`) ────────────────────────

fn trailing_backslashes(text: &str) -> usize {
    text.bytes().rev().take_while(|&b| b == b'\\').count()
}

// Fold the raw lexemes into the `Token` stream the structural parser consumes:
// merge adjacent text (across dropped comments), apply trim markers and
// backslash escapes, and classify each tag. The byte spans recorded here feed
// the source map (`compile_to_wire_with_spans`).
pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, CompileError> {
    let lexemes = raw_lex(source)?;
    let mut tokens: Vec<Token> = Vec::new();
    let mut text = String::new();
    let mut text_start = 0usize;
    let mut trim_next = false;
    let mut cursor = 0usize; // running start: the end of the previous lexeme

    for lexeme in lexemes {
        let start = cursor;
        cursor = lexeme.end;
        match lexeme.kind {
            LexKind::Text(run) => {
                if text.is_empty() {
                    text_start = start;
                }
                text.push_str(&run);
            }
            // Comments are dropped; surrounding text merges and a pending trim
            // carries across, exactly like the BEAM tokenizer.
            LexKind::BlockComment | LexKind::InlineComment => {}
            LexKind::RawBlock(content) => {
                if text.is_empty() {
                    text_start = start;
                }
                text.push_str(&content);
            }
            LexKind::StandardTag(inner) => {
                // Backslash escaping (standard tags only, like `escaped_mustache`):
                // N trailing backslashes before `{{`. N=1 escapes the tag (the
                // whole `{{…}}` becomes literal text); N>=2 emits N-1 backslashes
                // and evaluates the tag.
                let n = trailing_backslashes(&text);
                if n >= 1 {
                    text.truncate(text.len() - 1);
                    if n == 1 {
                        text.push_str("{{");
                        text.push_str(&inner);
                        text.push_str("}}");
                        continue;
                    }
                }
                emit_tag(
                    &mut tokens,
                    &mut text,
                    text_start,
                    &mut trim_next,
                    &inner,
                    false,
                    (start, lexeme.end),
                )?;
            }
            LexKind::RawTag(inner) => {
                emit_tag(
                    &mut tokens,
                    &mut text,
                    text_start,
                    &mut trim_next,
                    &inner,
                    true,
                    (start, lexeme.end),
                )?;
            }
        }
    }

    flush_text(
        &mut tokens,
        &mut text,
        &mut trim_next,
        (text_start, source.len()),
    );
    Ok(tokens)
}

#[allow(clippy::too_many_arguments)]
fn emit_tag(
    tokens: &mut Vec<Token>,
    text: &mut String,
    text_start: usize,
    trim_next: &mut bool,
    inner: &str,
    triple: bool,
    span: (usize, usize),
) -> Result<(), CompileError> {
    // Flush the pending text (applying any pending right-trim), then handle this
    // tag's own trim markers and classify it.
    flush_text(tokens, text, trim_next, (text_start, span.0));
    let (inner2, trim_left, trim_right) = extract_trim(inner);
    if trim_left {
        trim_trailing_text(tokens);
    }
    if let Some(token) = classify(&inner2, triple, span)? {
        tokens.push(token);
    }
    *trim_next = trim_right;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(&'static str, usize)> {
        raw_lex(source)
            .unwrap()
            .iter()
            .map(|l| {
                let tag = match l.kind {
                    LexKind::Text(_) => "text",
                    LexKind::BlockComment => "block_comment",
                    LexKind::InlineComment => "inline_comment",
                    LexKind::RawBlock(_) => "raw_block",
                    LexKind::RawTag(_) => "raw_tag",
                    LexKind::StandardTag(_) => "standard_tag",
                };
                (tag, l.end)
            })
            .collect()
    }

    #[test]
    fn lexes_text_tags_and_comments_with_end_offsets() {
        // `do_lex` tiles the source; the recorded ends are the unit boundaries.
        assert_eq!(
            kinds("Hi {{name}}!"),
            vec![("text", 3), ("standard_tag", 11), ("text", 12)]
        );
        assert_eq!(kinds("{{{raw}}}"), vec![("raw_tag", 9)]);
        assert_eq!(
            kinds("a{{!-- c --}}b{{! d }}e"),
            vec![
                ("text", 1),
                ("block_comment", 13),
                ("text", 14),
                ("inline_comment", 22),
                ("text", 23),
            ]
        );
        assert_eq!(kinds("{{{{#raw}}}}x{{{{/raw}}}}"), vec![("raw_block", 25)]);
    }

    #[test]
    fn unterminated_tag_reports_its_offset() {
        let err = raw_lex("Hi {{name").unwrap_err();
        assert_eq!(err.start, 3);
    }
}
