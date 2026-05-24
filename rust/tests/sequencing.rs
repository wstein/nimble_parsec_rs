use nimble_parsec_rs::{duplicate, eventually, integer_min, string, Value};
use num_bigint::BigInt;

#[test]
fn duplicate_parses_combinator_n_times() {
    let ok = duplicate(string("ab"), 3)
        .parse("ababab!")
        .expect("three repeats should parse");
    assert_eq!(
        ok.tokens,
        vec![
            Value::Str("ab".to_string()),
            Value::Str("ab".to_string()),
            Value::Str("ab".to_string()),
        ]
    );
    assert_eq!(ok.rest, "!");
}

#[test]
fn duplicate_requires_every_repeat() {
    assert!(duplicate(string("ab"), 2).parse("ab").is_err());
}

#[test]
fn duplicate_zero_matches_nothing() {
    let ok = duplicate(string("ab"), 0)
        .parse("xyz")
        .expect("zero repeats should match nothing");
    assert!(ok.tokens.is_empty());
    assert_eq!(ok.rest, "xyz");
}

#[test]
fn eventually_skips_until_inner_matches() {
    let ok = eventually(integer_min(1))
        .parse("abc12!")
        .expect("should eventually find an integer");
    assert_eq!(ok.tokens, vec![Value::Int(BigInt::from(12))]);
    assert_eq!(ok.rest, "!");
    assert_eq!(ok.cursor.byte_offset, 5);
}

#[test]
fn eventually_fails_when_never_matching() {
    assert!(eventually(string("X")).parse("aaa").is_err());
}
