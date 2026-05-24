use nimble_parsec_rs::{concat, ignore, integer_min, post_traverse, pre_traverse, string, Value};
use num_bigint::BigInt;

fn int(n: i64) -> Value {
    Value::Int(BigInt::from(n))
}

#[test]
fn post_traverse_sees_results_and_trailing_position() {
    // Replace the integer with the byte offset after it, and record the result
    // count in the context.
    let parser = post_traverse(integer_min(1), |tokens, mut ctx, cursor| {
        ctx.insert("count".to_string(), int(tokens.len() as i64));
        Ok((vec![int(cursor.byte_offset as i64)], ctx))
    });

    let ok = parser.parse("123").expect("post_traverse should parse");
    assert_eq!(ok.tokens, vec![int(3)]); // offset after "123"
    assert_eq!(ok.context.get("count"), Some(&int(1)));
}

#[test]
fn post_traverse_can_fail_the_parse() {
    let parser = post_traverse(integer_min(1), |_tokens, _ctx, _cursor| {
        Err("nope".to_string())
    });
    let err = parser
        .parse("123")
        .expect_err("traversal error should fail the parse");
    assert_eq!(err.reason, "nope");
}

#[test]
fn pre_traverse_sees_position_before_combinator() {
    // pre_traverse receives the cursor before the integer (byte offset 0).
    let parser = pre_traverse(integer_min(1), |mut tokens, ctx, cursor| {
        tokens.push(int(cursor.byte_offset as i64));
        Ok((tokens, ctx))
    });

    let ok = parser.parse("123").expect("pre_traverse should parse");
    assert_eq!(ok.tokens, vec![int(123), int(0)]);
}

#[test]
fn context_threads_across_concat() {
    let first = post_traverse(integer_min(1), |tokens, mut ctx, _| {
        ctx.insert("first".to_string(), tokens[0].clone());
        Ok((tokens, ctx))
    });
    let second = post_traverse(
        concat(ignore(string("-")), integer_min(1)),
        |mut tokens, ctx, _| {
            // The value stored by `first` is visible here.
            let prev = ctx.get("first").cloned().expect("first ran earlier");
            tokens.push(prev);
            Ok((tokens, ctx))
        },
    );

    let ok = concat(first, second).parse("12-34").expect("should parse");
    assert_eq!(ok.tokens, vec![int(12), int(34), int(12)]);
    assert_eq!(ok.rest, "");
}
