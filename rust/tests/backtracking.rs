//! The interpreter threads one shared token accumulator through the whole
//! parse, so any combinator that recovers from an inner failure must roll the
//! accumulator back to where it started — otherwise tokens emitted on a
//! discarded attempt would leak into the result. These tests pin that down for
//! every recovery point: `choice`, `optional`, the repetitions, and the
//! lookaheads.

use nimble_parsec_rs::{
    ascii_char, choice, concat, lookahead, lookahead_not, optional, repeat, string, AsciiPredicate,
    Integer, Value,
};

fn digit() -> nimble_parsec_rs::Parser {
    ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')])
}

fn int(c: char) -> Value {
    Value::Int(Integer::from(c as u32))
}

#[test]
fn choice_discards_tokens_from_a_partially_matched_branch() {
    // The first branch matches "a" (emitting it) before failing on "X"; the
    // winning branch must not inherit that stray "a".
    let parser = choice(vec![concat(string("a"), string("X")), string("ac")]);
    let ok = parser.parse("ac").expect("second branch should win");
    assert_eq!(ok.tokens, vec![Value::Str("ac".to_string())]);
    assert_eq!(ok.rest, "");
}

#[test]
fn optional_discards_tokens_from_a_partially_matched_inner() {
    // optional's inner emits a digit, then fails on the trailing ",". On
    // recovery the digit must be rolled back and nothing consumed.
    let parser = concat(string("z"), optional(concat(digit(), string(","))));
    let ok = parser.parse("z5x").expect("optional recovers");
    assert_eq!(ok.tokens, vec![Value::Str("z".to_string())]);
    assert_eq!(ok.rest, "5x");
}

#[test]
fn repeat_discards_tokens_from_the_failing_final_iteration() {
    // Each iteration emits "<digit>" then ",". The third begins matching ("3")
    // then fails on "x"; that partial iteration's "3" must not survive.
    let comma = || Value::Str(",".to_string());
    let parser = repeat(concat(digit(), string(",")), 0, None);
    let ok = parser
        .parse("1,2,3x")
        .expect("repeat stops on the failed iteration");
    assert_eq!(ok.tokens, vec![int('1'), comma(), int('2'), comma()]);
    assert_eq!(ok.rest, "3x");
}

#[test]
fn lookahead_emits_nothing_even_when_inner_would() {
    // A positive lookahead matches a digit but must not emit it, and must not
    // consume input.
    let parser = concat(lookahead(digit()), digit());
    let ok = parser.parse("7").expect("lookahead then digit");
    assert_eq!(ok.tokens, vec![int('7')]);
    assert_eq!(ok.rest, "");
}

#[test]
fn lookahead_not_emits_nothing_from_its_probe() {
    // The negative lookahead's inner runs (and fails) without leaving tokens.
    let parser = concat(lookahead_not(string("x")), digit());
    let ok = parser.parse("9").expect("negative lookahead then digit");
    assert_eq!(ok.tokens, vec![int('9')]);
    assert_eq!(ok.rest, "");
}
