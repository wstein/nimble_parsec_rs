use nimble_parsec_rs::{
    ascii_char, choice, concat, ignore, integer_exact, integer_min, map, optional, repeat, string,
    tag, utf8_string, AsciiPredicate, Cursor, ParseSuccess, Value,
};

fn ok_tokens(result: ParseSuccess<'_>) -> Vec<Value> {
    result.tokens
}

#[test]
fn iso_datetime_no_timezone_like_integration_test() {
    let date = concat(
        concat(
            integer_exact(4),
            concat(ignore(string("-")), concat(integer_exact(2), ignore(string("-")))),
        ),
        integer_exact(2),
    );

    let time = concat(
        integer_exact(2),
        concat(
            ignore(string(":")),
            concat(integer_exact(2), concat(ignore(string(":")), integer_exact(2))),
        ),
    );

    let parser = concat(date, concat(ignore(string("T")), time));
    let ok = parser.parse("2010-04-17T14:12:34").expect("parser should succeed");

    assert_eq!(
        ok_tokens(ok.clone()),
        vec![
            Value::Int(2010),
            Value::Int(4),
            Value::Int(17),
            Value::Int(14),
            Value::Int(12),
            Value::Int(34)
        ]
    );
    assert_eq!(ok.rest, "");
    assert_eq!(ok.cursor, Cursor { line: 1, line_start_offset: 0, byte_offset: 19 });
}

#[test]
fn markdown_h1_like_integration_test() {
    let parser = concat(ignore(string("#")), utf8_string(1, None));
    let ok = parser.parse("# Heading").expect("parser should succeed");

    assert_eq!(ok_tokens(ok.clone()), vec![Value::Str(" Heading".to_string())]);
    assert_eq!(ok.rest, "");
    assert_eq!(ok.cursor.byte_offset, 9);
}

#[test]
fn signed_int_with_optional_sign_and_tag() {
    let sign = optional(ascii_char(vec![AsciiPredicate::Char(b'-')]));
    let core = concat(sign, integer_min(1));

    let mapped = map(core, |tokens| {
        if tokens.len() == 2 {
            if matches!(tokens[0], Value::Char('-')) {
                if let Value::Int(v) = tokens[1] {
                    return vec![Value::Int(-v)];
                }
            }
        }
        tokens
    });

    let parser = tag("signed_int", mapped);

    let ok_neg = parser.parse("-1").expect("negative should parse");
    assert_eq!(
        ok_neg.tokens,
        vec![Value::Tagged("signed_int".to_string(), vec![Value::Int(-1)])]
    );

    let ok_pos = parser.parse("42").expect("positive should parse");
    assert_eq!(
        ok_pos.tokens,
        vec![Value::Tagged("signed_int".to_string(), vec![Value::Int(42)])]
    );
}

#[test]
fn choice_and_repeat_basics() {
    let letter = ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]);
    let parser = concat(choice(vec![string("foo"), string("bar")]), repeat(letter, 1, Some(3)));

    let ok = parser.parse("fooxyz!").expect("choice+repeat should parse");
    assert_eq!(
        ok.tokens,
        vec![
            Value::Str("foo".to_string()),
            Value::Char('x'),
            Value::Char('y'),
            Value::Char('z')
        ]
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
