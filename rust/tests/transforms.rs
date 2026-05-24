use nimble_parsec_rs::Integer;
use nimble_parsec_rs::{
    ascii_char, concat, ignore, integer_min, map, post_traverse, reduce, replace, string,
    unwrap_and_tag, wrap, AsciiPredicate, Value,
};

fn int(n: i64) -> Value {
    Value::Int(Integer::from(n))
}

// `ignore` suppresses inner token allocation, but token-dependent observable
// effects must still run. These pin that contract.

#[test]
fn ignore_preserves_post_traverse_context() {
    let first = ignore(post_traverse(integer_min(1), |tokens, mut ctx, _| {
        ctx.insert("seen".to_string(), Value::Str("yes".to_string()));
        Ok((tokens, ctx))
    }));
    let parser = concat(first, concat(ignore(string("-")), integer_min(1)));

    let ok = parser.parse("12-34").expect("parses");
    assert_eq!(ok.context.get("seen"), Some(&Value::Str("yes".to_string())));
    assert_eq!(ok.tokens, vec![int(34)]);
}

#[test]
fn ignore_propagates_post_traverse_error() {
    let parser = ignore(post_traverse(integer_min(1), |_, _, _| {
        Err("boom".to_string())
    }));
    assert_eq!(parser.parse("12").expect_err("should fail").reason, "boom");
}

#[test]
fn ignore_still_validates_unwrap_and_tag() {
    let pair = concat(integer_min(1), concat(ignore(string(",")), integer_min(1)));
    assert!(ignore(unwrap_and_tag("pair", pair)).parse("3,4").is_err());
}

#[test]
fn map_transforms_each_token_individually() {
    let two = concat(
        ascii_char(vec![AsciiPredicate::Any]),
        ascii_char(vec![AsciiPredicate::Any]),
    );
    let parser = map(two, |v| match v {
        Value::Int(n) => Value::Int(n + Integer::from(1)),
        other => other,
    });

    let ok = parser.parse("ab").expect("map should parse");
    // 'a' (97) and 'b' (98) each incremented.
    assert_eq!(
        ok.tokens,
        vec![Value::Int(Integer::from(98)), Value::Int(Integer::from(99))]
    );
}

#[test]
fn reduce_collapses_tokens_into_one() {
    let pair = concat(integer_min(1), concat(ignore(string(",")), integer_min(1)));
    let parser = reduce(pair, |tokens| {
        let sum: Integer = tokens
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
