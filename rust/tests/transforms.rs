use nimble_parsec_rs::{
    ascii_char, concat, ignore, integer_min, map, reduce, replace, string, unwrap_and_tag, wrap,
    AsciiPredicate, Value,
};
use num_bigint::BigInt;

fn int(n: i64) -> Value {
    Value::Int(BigInt::from(n))
}

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
    assert_eq!(ok.tokens, vec![int(7)]);
}

#[test]
fn replace_swaps_results_for_a_constant() {
    let parser = replace(integer_min(1), Value::Str("NUM".to_string()));
    let ok = parser.parse("123!").expect("replace should parse");
    assert_eq!(ok.tokens, vec![Value::Str("NUM".to_string())]);
    assert_eq!(ok.rest, "!");
}

#[test]
fn wrap_collects_results_into_a_list() {
    let pair = concat(integer_min(1), concat(ignore(string(",")), integer_min(1)));
    let ok = wrap(pair).parse("3,4").expect("wrap should parse");
    assert_eq!(ok.tokens, vec![Value::List(vec![int(3), int(4)])]);
}

#[test]
fn unwrap_and_tag_tags_a_single_value() {
    let ok = unwrap_and_tag("n", integer_min(1))
        .parse("42")
        .expect("unwrap_and_tag should parse");
    assert_eq!(
        ok.tokens,
        vec![Value::KeyValue("n".to_string(), Box::new(int(42)))]
    );
}

#[test]
fn unwrap_and_tag_rejects_multiple_tokens() {
    let pair = concat(integer_min(1), concat(ignore(string(",")), integer_min(1)));
    assert!(unwrap_and_tag("pair", pair).parse("3,4").is_err());
}
