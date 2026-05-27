// SPDX-License-Identifier: Apache-2.0
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

use nimble_parsec_rs::{
    ascii_string, choice, ignore, lookahead_not, post_traverse, repeat, string, utf8_char,
    AsciiPredicate, Integer, Parser, Value,
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

fn name_predicates() -> Vec<AsciiPredicate> {
    vec![
        AsciiPredicate::Range(b'a'..=b'z'),
        AsciiPredicate::Range(b'A'..=b'Z'),
        AsciiPredicate::Range(b'0'..=b'9'),
        AsciiPredicate::Char(b'_'),
        AsciiPredicate::Char(b'-'),
    ]
}

// Collapse a run of `utf8_char` codepoint integers into a string, the role
// Elixir fills with `reduce({List, :to_string, []})`.
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

// Append the byte offset just past `inner` (its end position) and tag the unit,
// so the assembler can recover each unit's span. Mirrors Elixir's
// `post_traverse(inject_end_pos)` followed by `tag/1`.
fn tagged_with_end(inner: Parser, tag: &'static str) -> Parser {
    post_traverse(inner, |mut tokens, context, cursor| {
        tokens.push(Value::Int(Integer::from(cursor.byte_offset)));
        Ok((tokens, context))
    })
    .tagged(tag)
}

// `repeat(utf8_char)` stopping before `stop`, reduced to a string.
fn chars_until(stop: &'static str) -> Parser {
    repeat(lookahead_not(string(stop)).then(utf8_char(vec![])), 0, None)
        .reduce(codepoints_to_string)
}

fn text_chunk() -> Parser {
    tagged_with_end(
        repeat(lookahead_not(string("{{")).then(utf8_char(vec![])), 1, None)
            .reduce(codepoints_to_string),
        "text",
    )
}

fn block_comment() -> Parser {
    tagged_with_end(
        ignore(string("{{!--"))
            .then(ignore(chars_until("--}}")))
            .then(ignore(string("--}}"))),
        "block_comment",
    )
}

fn inline_comment() -> Parser {
    tagged_with_end(
        ignore(string("{{!"))
            .then(ignore(chars_until("}}")))
            .then(ignore(string("}}"))),
        "inline_comment",
    )
}

fn raw_block() -> Parser {
    let parser = ignore(string("{{{{#"))
        .then(ascii_string(name_predicates(), 1, None))
        .then(ignore(string("}}}}")))
        .then(chars_until("{{{{/"))
        .then(ignore(string("{{{{/")))
        .then(ascii_string(name_predicates(), 1, None))
        .then(ignore(string("}}}}")));

    // Validate the open/close names match and keep only the content, mirroring
    // Elixir's `validate_and_reduce_raw_block`.
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

fn raw_tag() -> Parser {
    tagged_with_end(
        ignore(string("{{{"))
            .then(chars_until("}}}"))
            .then(ignore(string("}}}"))),
        "raw_tag",
    )
}

fn standard_tag() -> Parser {
    tagged_with_end(
        ignore(string("{{"))
            .then(chars_until("}}"))
            .then(ignore(string("}}"))),
        "standard_tag",
    )
}

fn do_lex() -> Parser {
    repeat(
        choice(vec![
            block_comment(),
            inline_comment(),
            raw_block(),
            raw_tag(),
            standard_tag(),
            text_chunk(),
        ]),
        0,
        None,
    )
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
