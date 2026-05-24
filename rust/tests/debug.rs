use nimble_parsec_rs::{debug, string, Value};

#[test]
fn debug_passes_success_through() {
    let ok = debug(string("ab"))
        .parse("abc")
        .expect("debug should parse");
    assert_eq!(ok.tokens, vec![Value::Str("ab".to_string())]);
    assert_eq!(ok.rest, "c");
}

#[test]
fn debug_passes_failure_through() {
    assert!(debug(string("ab")).parse("xy").is_err());
}
