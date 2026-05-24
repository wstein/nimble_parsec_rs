use nimble_parsec_rs::{
    ascii_char, concat, ignore, integer_min, string, tag, AsciiPredicate, Value,
};
use num_bigint::BigInt;

fn letter() -> nimble_parsec_rs::Parser {
    ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')])
}

#[test]
fn fluent_chain_matches_free_functions() {
    let fluent = letter().then(integer_min(1)).tagged("t");
    let free = tag("t", concat(letter(), integer_min(1)));

    let a = fluent.parse("a12").expect("fluent parses");
    let b = free.parse("a12").expect("free parses");
    assert_eq!(a.tokens, b.tokens);
    assert_eq!(a.rest, b.rest);
}

#[test]
fn fluent_or_optional_repeated_ignored() {
    assert!(string("foo").or(string("bar")).parse("bar").is_ok());

    let signed = string("-").ignored().then(integer_min(1));
    assert_eq!(signed.parse("-5").expect("parses").rest, "");

    let letters = letter().repeated(1, Some(3));
    assert_eq!(letters.parse("abcd").expect("parses").rest, "d");

    let opt = letter().optional();
    assert_eq!(
        opt.parse("1").expect("optional parses"),
        opt.parse("1").unwrap()
    );
    assert!(opt.parse("1").expect("optional parses").tokens.is_empty());
}

#[test]
fn fluent_reduce_and_replace() {
    let summed =
        concat(integer_min(1), concat(ignore(string(",")), integer_min(1))).reduce(|tokens| {
            let total: BigInt = tokens
                .into_iter()
                .filter_map(|t| match t {
                    Value::Int(n) => Some(n),
                    _ => None,
                })
                .sum();
            Value::Str(total.to_string())
        });
    assert_eq!(
        summed.parse("3,4").expect("parses").tokens,
        vec![Value::Str("7".to_string())]
    );

    let replaced = integer_min(1).replaced_with(Value::Str("NUM".to_string()));
    assert_eq!(
        replaced.parse("99").expect("parses").tokens,
        vec![Value::Str("NUM".to_string())]
    );
}
