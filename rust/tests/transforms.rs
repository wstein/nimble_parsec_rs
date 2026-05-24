use nimble_parsec_rs::{
    ascii_char, concat, ignore, integer_min, map, reduce, string, AsciiPredicate, Value,
};
use num_bigint::BigInt;

#[test]
fn map_transforms_each_token_individually() {
    let two = concat(
        ascii_char(vec![AsciiPredicate::Any]),
        ascii_char(vec![AsciiPredicate::Any]),
    );
    let parser = map(two, |v| match v {
        Value::Int(n) => Value::Int(n + BigInt::from(1)),
        other => other,
    });

    let ok = parser.parse("ab").expect("map should parse");
    // 'a' (97) and 'b' (98) each incremented.
    assert_eq!(
        ok.tokens,
        vec![Value::Int(BigInt::from(98)), Value::Int(BigInt::from(99))]
    );
}

#[test]
fn reduce_collapses_tokens_into_one() {
    let pair = concat(integer_min(1), concat(ignore(string(",")), integer_min(1)));
    let parser = reduce(pair, |tokens| {
        let sum: BigInt = tokens
            .into_iter()
            .filter_map(|t| match t {
                Value::Int(n) => Some(n),
                _ => None,
            })
            .sum();
        Value::Int(sum)
    });

    let ok = parser.parse("3,4").expect("reduce should parse");
    assert_eq!(ok.tokens, vec![Value::Int(BigInt::from(7))]);
}
