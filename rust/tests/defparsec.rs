use nimble_parsec_rs::{
    concat, defcombinator, defcombinatorp, defparsec, defparsecp, ignore, integer_exact, string,
    Value,
};
use num_bigint::BigInt;

// ---------------------------------------------------------------------------
// defparsec! — generates a public parse function with codegen
// ---------------------------------------------------------------------------

defparsec!(
    parse_date,
    concat(
        concat(
            integer_exact(4),
            concat(ignore(string("-")), integer_exact(2)),
        ),
        concat(ignore(string("-")), integer_exact(2))
    )
);

#[test]
fn defparsec_codegen_parse_succeeds() {
    let ok = parse_date("2024-03-15 rest").expect("date parse");
    assert_eq!(ok.rest, " rest");
    assert_eq!(ok.cursor.byte_offset, 10);
    assert_eq!(ok.tokens.len(), 3);
    assert_eq!(ok.tokens[0], Value::Int(BigInt::from(2024u32)));
    assert_eq!(ok.tokens[1], Value::Int(BigInt::from(3u32)));
    assert_eq!(ok.tokens[2], Value::Int(BigInt::from(15u32)));
}

#[test]
fn defparsec_codegen_parse_fails() {
    assert!(parse_date("not-a-date").is_err());
}

// ---------------------------------------------------------------------------
// defparsec! — falls back to OnceLock runtime for unsupported combinators
// ---------------------------------------------------------------------------

defparsec!(parse_number, integer_min(1));

#[test]
fn defparsec_runtime_fallback_parses() {
    let ok = parse_number("42abc").expect("number parse");
    assert_eq!(ok.rest, "abc");
    assert_eq!(ok.tokens, vec![Value::Int(BigInt::from(42u32))]);
}

// ---------------------------------------------------------------------------
// defparsecp! — private function
// ---------------------------------------------------------------------------

defparsecp!(private_parse_ab, concat(string("a"), string("b")));

#[test]
fn defparsecp_generates_private_function() {
    let ok = private_parse_ab("abXY").expect("ab parse");
    assert_eq!(ok.rest, "XY");
    assert_eq!(ok.tokens.len(), 2);
}

// ---------------------------------------------------------------------------
// defcombinator! / defcombinatorp!
// ---------------------------------------------------------------------------

defcombinator!(two_digit_int, integer_exact(2));

#[test]
fn defcombinator_returns_reusable_parser() {
    let p = two_digit_int();
    let ok1 = p.parse("12rest").expect("first parse");
    let ok2 = p.parse("99end").expect("second parse");
    assert_eq!(ok1.tokens, vec![Value::Int(BigInt::from(12u32))]);
    assert_eq!(ok2.tokens, vec![Value::Int(BigInt::from(99u32))]);
}

#[test]
fn defcombinator_parser_is_composable() {
    let p = concat(
        two_digit_int(),
        concat(ignore(string(":")), two_digit_int()),
    );
    let ok = p.parse("12:34end").expect("composed parse");
    assert_eq!(ok.rest, "end");
    assert_eq!(ok.tokens.len(), 2);
}

defcombinatorp!(private_combinator, string("x"));

#[test]
fn defcombinatorp_generates_private_combinator() {
    let ok = private_combinator().parse("xyz").expect("x parse");
    assert_eq!(ok.rest, "yz");
}

// ---------------------------------------------------------------------------
// compile_parser! codegen — ignore suppresses allocation
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_codegen_matches_runtime() {
    use nimble_parsec_rs::compile_parser;

    let compiled = compile_parser!(concat(
        integer_exact(4),
        concat(ignore(string("-")), integer_exact(2))
    ));
    let runtime = concat(
        integer_exact(4),
        concat(ignore(string("-")), integer_exact(2)),
    );

    let c = compiled.parse("2024-03").expect("compiled");
    let r = runtime.parse("2024-03").expect("runtime");
    assert_eq!(c.tokens, r.tokens);
    assert_eq!(c.rest, r.rest);
    assert_eq!(c.cursor.byte_offset, r.cursor.byte_offset);
}
