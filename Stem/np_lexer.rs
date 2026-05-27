// SPDX-License-Identifier: Apache-2.0
//
// NOTE (historical): this reference example uses the pre-rewrite, `Value`-based
// `nimble_parsec_rs` API, which has since been replaced by the typed
// `Parser<Output>` surface (see `rust/docs/rfcs/0001-typed-parser.md`). For the
// typed equivalent of this lexer — text/tags/comments folded into a typed token
// enum with no `Value` tagging — see `rust/tests/typed_grammar.rs`.
//
// Combinator lexer — a `nimble_parsec_rs` port of `Stem.Parser`'s lexer, sharing
// one conceptual model with the BEAM reference. `do_lex` (the combinator grammar)
// recognizes the raw lexical units (text runs, comments, raw blocks, `{{{ }}}`
// and `{{ }}` tags), mirroring Elixir's NimbleParsec `do_lex`; `tokenize` then
// folds them into the `Token` stream, applying trim markers, backslash escapes,
// and tag classification, mirroring Elixir's hand-written `assemble_tokens`.
//
// This replaces the previous hand-written byte-scanning tokenizer; the structural
// parser (`assemble`/`collect`) and the existing wire/conformance gates
// (`compile_diff`/`verify`/`fuzz`) are the arbiter of byte-for-byte parity.
//
// ─────────────────────────────────────────────────────────────────────────────
// This file doubles as a *worked example* of `nimble_parsec_rs`: a complete,
// real-world lexer that exercises a broad slice of the crate's surface. It uses
// both calling styles deliberately — the free combinator functions (`string`,
// `choice`, `lookahead_not`, `post_traverse`, …), which mirror NimbleParsec's
// names for readers porting from Elixir, and the fluent `Parser` methods
// (`.then`, `.ignored`, `.repeated`, `.reduce`, `.tagged`), which read better
// when chaining. Every fluent method delegates to the free function of the
// matching name, so the two are interchangeable; pick whichever reads best.
//
// Combinator coverage (this file + its sibling `np_expr.rs`):
//
//   Demonstrated here:
//     string, utf8_char, ascii_string (+ AsciiPredicate ranges)   — leaves
//     concat (`.then`), ignore (`.ignored`)                       — sequencing
//     repeat (`.repeated`), choice                                — control flow
//     lookahead_not                                               — negative assertion
//     reduce (`.reduce`), tag (`.tagged`)                         — result shaping
//     post_traverse                                               — context-aware
//                                                                    validation + the
//                                                                    end-offset injection,
//                                                                    and *failing* the parse
//   Demonstrated in `np_expr.rs`:
//     recursive / ParserRef, optional, `.or`, Utf8Predicate
//   Purpose-built alternatives worth knowing (not needed by this grammar):
//     byte_offset / line — emit position metadata directly, instead of the manual
//       `post_traverse` offset injection used here (kept manual so the decoder gets
//       a flat `[content…, end]` shape); map / wrap / replace / unwrap_and_tag —
//       other result transforms; times / duplicate / repeat_while — bounded or
//       predicate-driven repetition; eventually / lookahead — scan-ahead; eos —
//       end-of-input assertion; label — override a failure message (no effect here,
//       since `choice` makes the top-level grammar total). See the crate README.

use nimble_parsec_rs::{
    ascii_string, choice, lookahead_not, post_traverse, string, utf8_char, AsciiPredicate,
    Integer, Parser, Value,
};

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

// ── do_lex grammar (mirrors `Stem.Parser.do_lex`) ────────────────────────────

// The character class for a tag/raw-block name: `[A-Za-z0-9_-]`. `AsciiPredicate`
// ranges and single chars compose into the set `ascii_string` accepts.
fn name_predicates() -> Vec<AsciiPredicate> {
    vec![
        AsciiPredicate::Range(b'a'..=b'z'),
        AsciiPredicate::Range(b'A'..=b'Z'),
        AsciiPredicate::Range(b'0'..=b'9'),
        AsciiPredicate::Char(b'_'),
        AsciiPredicate::Char(b'-'),
    ]
}

// Collapse a run of `utf8_char` codepoint integers into a string — the `reduce`
// callback fills the role Elixir gives `reduce({List, :to_string, []})`.
fn codepoints_to_string(tokens: Vec<Value>) -> Value {
    let text: String = tokens
        .iter()
        .filter_map(|value| match value {
            Value::Int(int) => int.to_string().parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        })
        .collect();
    Value::Str(text)
}

// Wrap `inner` so the assembler can recover each unit's span: `post_traverse`
// runs a closure *after* `inner` with the post-parse cursor in hand, letting us
// append the byte offset just past the match; `.tagged` then names the unit.
// Mirrors Elixir's `post_traverse(inject_end_pos)` followed by `tag/1`.
//
// (`nimble_parsec_rs` also ships a `byte_offset` combinator that emits the
// position directly; we hand-roll it via `post_traverse` only to keep a flat
// `[content…, end]` token shape that `lexeme_from_value` can pop in order.)
fn tagged_with_end(inner: Parser, tag: &'static str) -> Parser {
    post_traverse(inner, |mut tokens, context, cursor| {
        tokens.push(Value::Int(Integer::from(cursor.byte_offset)));
        Ok((tokens, context))
    })
    .tagged(tag)
}

// `repeat`-of-`utf8_char` stopping before `stop`, reduced to a string. Shown in
// fluent style: each `utf8_char` is gated by a `lookahead_not` (so we never
// consume the terminator), the pair is `.repeated`, and the run is `.reduce`d.
fn chars_until(stop: &'static str) -> Parser {
    lookahead_not(string(stop))
        .then(utf8_char(vec![]))
        .repeated(0, None)
        .reduce(codepoints_to_string)
}

// A maximal run of text, stopping before the next `{{`. Same shape as
// `chars_until` but with a minimum of 1 (an empty text run carries no meaning).
fn text_chunk() -> Parser {
    tagged_with_end(
        lookahead_not(string("{{"))
            .then(utf8_char(vec![]))
            .repeated(1, None)
            .reduce(codepoints_to_string),
        "text",
    )
}

// `{{!-- … --}}` — opener, body (ignored), closer. `.ignored()` discards the
// delimiters' and body's tokens; only the unit's tag + end offset survive.
fn block_comment() -> Parser {
    tagged_with_end(
        string("{{!--")
            .ignored()
            .then(chars_until("--}}").ignored())
            .then(string("--}}").ignored()),
        "block_comment",
    )
}

// `{{! … }}` — the single-brace inline comment variant.
fn inline_comment() -> Parser {
    tagged_with_end(
        string("{{!")
            .ignored()
            .then(chars_until("}}").ignored())
            .then(string("}}").ignored()),
        "inline_comment",
    )
}

// `{{{{#name}}}}…{{{{/name}}}}` — the open name, verbatim content, and close name
// are captured; the delimiters are ignored.
fn raw_block() -> Parser {
    let parser = string("{{{{#")
        .ignored()
        .then(ascii_string(name_predicates(), 1, None))
        .then(string("}}}}").ignored())
        .then(chars_until("{{{{/"))
        .then(string("{{{{/").ignored())
        .then(ascii_string(name_predicates(), 1, None))
        .then(string("}}}}").ignored());

    // `post_traverse` can also *reject* the parse: here it validates that the
    // open/close names match and keeps only the content, mirroring Elixir's
    // `validate_and_reduce_raw_block`. Returning `Err` turns into a parse failure.
    let validated = post_traverse(parser, |tokens, context, _cursor| match tokens.as_slice() {
        [Value::Str(open), Value::Str(content), Value::Str(close)] => {
            if open == close {
                Ok((vec![Value::Str(content.clone())], context))
            } else {
                Err(format!(
                    "raw block open `{{{{{{{{#{open}}}}}}}}}` is closed by `{{{{{{{{/{close}}}}}}}}}`"
                ))
            }
        }
        _ => Err("malformed raw block".to_string()),
    });
    tagged_with_end(validated, "raw_block")
}

// `{{{ … }}}` — a triple-brace raw tag; its inner text is kept verbatim.
fn raw_tag() -> Parser {
    tagged_with_end(
        string("{{{")
            .ignored()
            .then(chars_until("}}}"))
            .then(string("}}}").ignored()),
        "raw_tag",
    )
}

// `{{ … }}` — the ordinary double-brace tag; inner text kept for classification.
fn standard_tag() -> Parser {
    tagged_with_end(
        string("{{")
            .ignored()
            .then(chars_until("}}"))
            .then(string("}}").ignored()),
        "standard_tag",
    )
}

// The whole document: zero or more lexical units. `choice` tries the alternatives
// in order (longest/most-specific delimiters first so `{{{{` beats `{{{` beats
// `{{`, and text is the catch-all), and `.repeated(0, None)` tiles the source.
fn do_lex() -> Parser {
    choice(vec![
        block_comment(),
        inline_comment(),
        raw_block(),
        raw_tag(),
        standard_tag(),
        text_chunk(),
    ])
    .repeated(0, None)
}

fn raw_lex(source: &str) -> Result<Vec<Lexeme>, CompileError> {
    match do_lex().parse(source) {
        Ok(success) if success.rest.is_empty() => {
            success.tokens.into_iter().map(lexeme_from_value).collect()
        }
        // An unconsumed remainder means an unterminated tag/comment/raw block:
        // report the offset, matching the hand-written tokenizer's error span.
        Ok(success) => Err(CompileError {
            message: "unterminated tag while lexing template".to_string(),
            file: String::new(),
            start: success.cursor.byte_offset,
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

fn lexeme_from_value(value: Value) -> Result<Lexeme, CompileError> {
    let internal = |message: &str| CompileError {
        message: format!("internal lexer error: {message}"),
        file: String::new(),
        start: 0,
        end: 0,
    };
    let Value::Tagged(name, mut items) = value else {
        return Err(internal("lexer token is not tagged"));
    };
    let end = match items.pop() {
        Some(Value::Int(offset)) => offset
            .to_string()
            .parse::<usize>()
            .map_err(|_| internal("end offset out of range"))?,
        _ => return Err(internal("lexer token is missing its end offset")),
    };
    let content = |items: &mut Vec<Value>| match items.pop() {
        Some(Value::Str(text)) => Ok(text),
        _ => Err(internal("lexer token is missing its string payload")),
    };
    let kind = match name.as_str() {
        "text" => LexKind::Text(content(&mut items)?),
        "block_comment" => LexKind::BlockComment,
        "inline_comment" => LexKind::InlineComment,
        "raw_block" => LexKind::RawBlock(content(&mut items)?),
        "raw_tag" => LexKind::RawTag(content(&mut items)?),
        "standard_tag" => LexKind::StandardTag(content(&mut items)?),
        other => return Err(internal(&format!("unknown lexer tag `{other}`"))),
    };
    Ok(Lexeme { kind, end })
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
