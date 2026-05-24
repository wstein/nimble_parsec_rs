use nimble_parsec_rs::{choice, concat, ignore, recursive, string, ParserRef, Value};

#[test]
fn parser_ref_resolves_after_define() {
    let reference = ParserRef::new();
    reference.define(string("hi"));

    let ok = reference
        .parser()
        .parse("hi!")
        .expect("reference should parse");
    assert_eq!(ok.tokens, vec![Value::Str("hi".to_string())]);
    assert_eq!(ok.rest, "!");
}

#[test]
fn recursive_parses_nested_structure() {
    // A balanced-parenthesis grammar: "(" expr ")" | "x".
    let parser = recursive(|expr| {
        choice(vec![
            concat(ignore(string("(")), concat(expr, ignore(string(")")))),
            string("x"),
        ])
    });

    assert!(parser.parse("x").is_ok());

    let ok = parser.parse("(((x)))").expect("nested parens should parse");
    // Parens are ignored, so only the base case emits a token.
    assert_eq!(ok.tokens, vec![Value::Str("x".to_string())]);
    assert_eq!(ok.rest, "");

    // Trailing input after a complete match is left unconsumed.
    let partial = parser.parse("xyz").expect("should parse the leading x");
    assert_eq!(partial.tokens, vec![Value::Str("x".to_string())]);
    assert_eq!(partial.rest, "yz");

    // Unbalanced input fails (the grammar requires matched parens).
    assert!(parser.parse("((x)").is_err());
}

#[test]
#[should_panic(expected = "used before it was defined")]
fn parser_ref_panics_if_used_before_define() {
    let reference = ParserRef::new();
    let _ = reference.parser().parse("x");
}
