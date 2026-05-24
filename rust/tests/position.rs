use nimble_parsec_rs::{byte_offset, concat, ignore, integer_min, line, string, Value};
use num_bigint::BigInt;

fn int(n: i64) -> Value {
    Value::Int(BigInt::from(n))
}

#[test]
fn byte_offset_wraps_results_with_trailing_offset() {
    let ok = byte_offset(integer_min(1))
        .parse("123!")
        .expect("byte_offset should parse");
    // {[123], 3}
    assert_eq!(
        ok.tokens,
        vec![Value::List(vec![Value::List(vec![int(123)]), int(3)])]
    );
    assert_eq!(ok.rest, "!");
}

#[test]
fn line_wraps_results_with_line_position() {
    // After "a\n" the cursor is on line 2 with line offset 2.
    let parser = concat(ignore(string("a\n")), line(integer_min(1)));
    let ok = parser.parse("a\n42").expect("line should parse");
    // {[42], {2, 2}}
    assert_eq!(
        ok.tokens,
        vec![Value::List(vec![
            Value::List(vec![int(42)]),
            Value::List(vec![int(2), int(2)]),
        ])]
    );
    assert_eq!(ok.rest, "");
}
