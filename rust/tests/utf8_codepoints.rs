use nimble_parsec_rs::Integer;
use nimble_parsec_rs::{utf8_char, utf8_string, Utf8Predicate, Value};

fn cp(c: char) -> Value {
    Value::Int(Integer::from(c as u32))
}

#[test]
fn utf8_char_emits_codepoint_integer_for_any() {
    let ok = utf8_char(vec![])
        .parse("é!")
        .expect("any codepoint should match");
    assert_eq!(ok.tokens, vec![cp('é')]);
    assert_eq!(ok.rest, "!");
    // 'é' (U+00E9) is two UTF-8 bytes.
    assert_eq!(ok.cursor.byte_offset, 2);
}

#[test]
fn utf8_char_enforces_inclusive_range() {
    let digit = utf8_char(vec![Utf8Predicate::Range('0'..='9')]);
    let ok = digit.parse("5x").expect("digit should match");
    assert_eq!(ok.tokens, vec![cp('5')]);
    assert!(digit.parse("ax").is_err());
}

#[test]
fn utf8_char_honors_negative_constraints() {
    let not_a = utf8_char(vec![Utf8Predicate::NotChar('a')]);
    assert!(not_a.parse("b").is_ok());
    assert!(not_a.parse("a").is_err());
}

#[test]
fn utf8_string_stops_at_first_out_of_range_codepoint() {
    let lower = utf8_string(vec![Utf8Predicate::Range('a'..='z')], 1, None);
    let ok = lower
        .parse("abc123")
        .expect("leading lowercase should parse");
    assert_eq!(ok.tokens, vec![Value::Str("abc".to_string())]);
    assert_eq!(ok.rest, "123");
}

#[test]
fn utf8_string_fails_when_below_minimum() {
    let lower = utf8_string(vec![Utf8Predicate::Range('a'..='z')], 2, None);
    assert!(lower.parse("a1").is_err());
}

#[test]
fn utf8_string_respects_maximum() {
    let any = utf8_string(vec![], 0, Some(2));
    let ok = any.parse("abcd").expect("bounded parse should succeed");
    assert_eq!(ok.tokens, vec![Value::Str("ab".to_string())]);
    assert_eq!(ok.rest, "cd");
}
