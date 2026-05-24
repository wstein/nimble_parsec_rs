use nimble_parsec_rs::{label, string, Value};

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
