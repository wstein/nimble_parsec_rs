use nimble_parsec_rs::{ascii_char, choice, concat, repeat, AsciiPredicate, Value};
use num_bigint::BigInt;

fn ch(c: char) -> Value {
    Value::Int(BigInt::from(c as u32))
}

#[test]
fn choice_and_repeat_basics() {
    let letter = ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]);
    let parser = concat(
        choice(vec![string_foo(), string_bar()]),
        repeat(letter, 1, Some(3)),
    );

    let ok = parser.parse("fooxyz!").expect("choice+repeat should parse");
    assert_eq!(
        ok.tokens,
        vec![Value::Str("foo".to_string()), ch('x'), ch('y'), ch('z')]
    );
    assert_eq!(ok.rest, "!");
}

#[test]
fn ascii_predicates_support_negative_constraints() {
    let parser = ascii_char(vec![
        AsciiPredicate::Range(b'0'..=b'9'),
        AsciiPredicate::NotChar(b'3'),
    ]);

    assert!(parser.parse("7x").is_ok());
    assert!(parser.parse("3x").is_err());
}

fn string_foo() -> nimble_parsec_rs::Parser {
    nimble_parsec_rs::string("foo")
}

fn string_bar() -> nimble_parsec_rs::Parser {
    nimble_parsec_rs::string("bar")
}
