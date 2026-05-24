use nimble_parsec_rs::{ascii_string, bytes, concat, eos, string, AsciiPredicate, Value};

#[test]
fn ascii_string_collects_in_range_bytes() {
    let lower = ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 1, None);
    let ok = lower
        .parse("abc123")
        .expect("leading lowercase should parse");
    assert_eq!(ok.tokens, vec![Value::Str("abc".to_string())]);
    assert_eq!(ok.rest, "123");
}

#[test]
fn ascii_string_stops_at_non_ascii_byte() {
    // Empty predicates accept any ASCII byte, but a multibyte codepoint ends it.
    let any = ascii_string(vec![], 1, None);
    let ok = any.parse("aé").expect("leading ascii should parse");
    assert_eq!(ok.tokens, vec![Value::Str("a".to_string())]);
    assert_eq!(ok.rest, "é");
}

#[test]
fn ascii_string_fails_below_minimum() {
    let lower = ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 2, None);
    assert!(lower.parse("a1").is_err());
}

#[test]
fn bytes_consumes_exact_count() {
    let ok = bytes(3).parse("abcd").expect("three bytes should parse");
    assert_eq!(ok.tokens, vec![Value::Str("abc".to_string())]);
    assert_eq!(ok.rest, "d");
    assert_eq!(ok.cursor.byte_offset, 3);
}

#[test]
fn bytes_fails_when_too_few() {
    assert!(bytes(3).parse("ab").is_err());
}

#[test]
fn bytes_fails_on_non_utf8_boundary() {
    // 'é' is two bytes; taking one byte would split the codepoint.
    assert!(bytes(1).parse("é").is_err());
}

#[test]
fn eos_matches_only_at_end() {
    assert!(eos().parse("").is_ok());
    assert!(eos().parse("x").is_err());

    let ok = concat(string("ab"), eos())
        .parse("ab")
        .expect("string then eos should parse");
    assert_eq!(ok.rest, "");
}
