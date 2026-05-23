use nimble_parsec_rs::{
    ascii_char, concat, integer_min, lookahead, lookahead_not, optional, repeat_while, string,
    times, AsciiPredicate, RepeatWhileControl, TimesOptions, Value,
};
use num_bigint::BigInt;

fn ch(c: char) -> Value {
    Value::Int(BigInt::from(c as u32))
}

#[test]
fn lookahead_matches_without_consuming() {
    let parser = concat(
        ascii_char(vec![AsciiPredicate::Any]),
        lookahead(integer_min(1)),
    );

    let ok = parser.parse("a0").expect("lookahead should pass");
    assert_eq!(ok.tokens, vec![ch('a')]);
    assert_eq!(ok.rest, "0");
    assert_eq!(ok.cursor.byte_offset, 1);
}

#[test]
fn lookahead_not_matches_when_inner_fails() {
    let parser = concat(
        ascii_char(vec![AsciiPredicate::Any]),
        lookahead_not(ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')])),
    );

    let ok = parser.parse("aa").expect("lookahead_not should pass");
    assert_eq!(ok.tokens, vec![ch('a')]);
    assert_eq!(ok.rest, "a");
    assert_eq!(ok.cursor.byte_offset, 1);

    assert!(parser.parse("a0").is_err());
}

#[test]
fn repeat_while_stops_on_predicate() {
    let parser = repeat_while(
        concat(
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
        ),
        |rest, _| {
            if rest.starts_with('3') {
                RepeatWhileControl::Halt
            } else {
                RepeatWhileControl::Cont
            }
        },
        0,
        None,
    );

    let ok = parser.parse("12345").expect("repeat_while should parse");
    assert_eq!(ok.tokens, vec![ch('1'), ch('2')]);
    assert_eq!(ok.rest, "345");
}

#[test]
fn times_respects_min_and_max() {
    let parser = times(
        ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
        TimesOptions::min_max(1, 4),
    );

    let ok = parser.parse("12345").expect("times should parse");
    assert_eq!(ok.tokens, vec![ch('1'), ch('2'), ch('3'), ch('4')]);
    assert_eq!(ok.rest, "5");
}

#[test]
fn repeat_while_stops_on_non_consuming_match() {
    // optional consumes nothing when it cannot match; the loop must terminate
    // even though the while predicate keeps saying Cont.
    let parser = repeat_while(
        optional(string("x")),
        |_, _| RepeatWhileControl::Cont,
        0,
        None,
    );
    let ok = parser
        .parse("yyy")
        .expect("non-consuming match should stop");
    assert!(ok.tokens.is_empty());
    assert_eq!(ok.rest, "yyy");
}

#[test]
fn times_exact_count() {
    let parser = times(string("ab"), TimesOptions::exact(2));
    let ok = parser.parse("abab!").expect("times exact should parse");
    assert_eq!(
        ok.tokens,
        vec![Value::Str("ab".to_string()), Value::Str("ab".to_string())]
    );
    assert_eq!(ok.rest, "!");
}
