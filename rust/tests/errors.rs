use nimble_parsec_rs::{
    ascii_char, choice, label, string, utf8_char, AsciiPredicate, Utf8Predicate, Value,
};

#[test]
fn label_overrides_failure_message() {
    let parser = label(string("foo"), "a greeting");
    let err = parser.parse("bar").expect_err("should fail");
    assert_eq!(err.reason, "expected a greeting");
    assert_eq!(err.rest, "bar");
}

#[test]
fn label_passes_success_through() {
    let parser = label(string("foo"), "a greeting");
    let ok = parser.parse("foo!").expect("should parse");
    assert_eq!(ok.tokens, vec![Value::Str("foo".to_string())]);
    assert_eq!(ok.rest, "!");
}

#[test]
fn choice_aggregates_branch_failures() {
    let parser = choice(vec![
        label(string("foo"), "a foo"),
        label(string("bar"), "a bar"),
    ]);
    let err = parser.parse("xyz").expect_err("all branches should fail");
    assert_eq!(err.reason, "expected a foo or expected a bar");
    assert_eq!(err.rest, "xyz");
}

#[test]
fn ascii_char_reports_allowed_range() {
    let parser = ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]);
    let err = parser.parse("x").expect_err("non-digit should fail");
    assert_eq!(
        err.reason,
        "expected ASCII character in the range \"0\" to \"9\""
    );
}

#[test]
fn ascii_char_reports_negated_constraints() {
    let parser = ascii_char(vec![
        AsciiPredicate::Range(b'0'..=b'9'),
        AsciiPredicate::NotChar(b'3'),
    ]);
    let err = parser.parse("3").expect_err("excluded digit should fail");
    assert_eq!(
        err.reason,
        "expected ASCII character in the range \"0\" to \"9\", and not equal to \"3\""
    );
}

#[test]
fn parse_failure_displays_and_is_a_std_error() {
    let err = string("foo").parse("bar").expect_err("should fail");
    let text = err.to_string();
    assert!(text.contains("expected string"));
    assert!(text.contains("byte offset 0"));

    // Usable through the std error trait object.
    let boxed: Box<dyn std::error::Error> = Box::new(err);
    assert!(boxed.to_string().contains("expected string"));
}

#[test]
fn utf8_char_reports_allowed_range() {
    let parser = utf8_char(vec![Utf8Predicate::Range('a'..='z')]);
    let err = parser.parse("0").expect_err("non-letter should fail");
    assert_eq!(
        err.reason,
        "expected utf8 codepoint in the range \"a\" to \"z\""
    );
}
