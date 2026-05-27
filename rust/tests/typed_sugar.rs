//! Convenience combinators: `delimited`, `separated_by[1]`, `repeated_until`,
//! the tuple-arity `choice`, and `fold`.

use nimble_parsec_rs::typed::{
    any, choice, delimited, digits, generate, literal, repeated_until, satisfy, separated_by,
    separated_by1, Generate, Parser,
};

#[test]
fn fold_accumulates_without_an_intermediate_vec() {
    // Parse a base-10 number by folding digit characters into an accumulator.
    let number = satisfy("a digit", |c: char| c.is_ascii_digit())
        .fold(|| 0u64, |acc, c| acc * 10 + (c as u64 - '0' as u64));
    assert_eq!(number.parse("01234").unwrap(), 1234);
    // No digits → the seed value.
    assert_eq!(number.parse("").unwrap(), 0);
}

#[test]
fn flat_map_chooses_the_next_parser_from_a_parsed_value() {
    // Context-sensitive: read a count, then exactly that many 'a's.
    let counted = digits()
        .map(|d: &str| d.parse::<usize>().unwrap())
        .flat_map(|n| literal("a").repeated_in(n, n));
    assert_eq!(counted.parse("3aaa").unwrap(), vec!["a", "a", "a"]);
    assert!(counted.parse("3aa").is_err()); // too few for the declared count
}

#[test]
fn delimited_keeps_only_the_content() {
    let p = delimited(literal("("), digits(), literal(")"));
    assert_eq!(p.parse("(42)").unwrap(), "42");
    assert!(delimited(literal("("), digits(), literal(")"))
        .parse("(42")
        .is_err());
}

#[test]
fn separated_by_collects_items_without_a_trailing_separator() {
    let list = separated_by(digits(), literal(","));
    assert_eq!(list.parse("1,2,3").unwrap(), vec!["1", "2", "3"]);

    // Empty input → empty list (min 0).
    assert_eq!(
        separated_by(digits(), literal(",")).parse("").unwrap(),
        Vec::<&str>::new()
    );

    // A trailing separator is not consumed.
    assert_eq!(
        separated_by(digits(), literal(","))
            .parse_partial("1,2,")
            .unwrap(),
        (vec!["1", "2"], ",")
    );
}

#[test]
fn separated_by1_requires_at_least_one_item() {
    assert!(separated_by1(digits(), literal(",")).parse("").is_err());
    assert_eq!(
        separated_by1(digits(), literal(",")).parse("7").unwrap(),
        vec!["7"]
    );
}

#[test]
fn repeated_until_stops_before_the_terminator() {
    let p = repeated_until(any(), literal("END"));
    assert_eq!(
        p.parse_partial("abcEND").unwrap(),
        (vec!['a', 'b', 'c'], "END")
    );
    // No terminator in the input → consumes everything.
    assert_eq!(
        repeated_until(any(), literal("END"))
            .parse("abc")
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn tuple_choice_allows_differently_typed_alternatives() {
    // Different parser types (Literal vs the digits run), one shared Output (&str).
    let token = choice((literal("if"), literal("else"), digits()));
    assert_eq!(token.parse("if").unwrap(), "if");
    assert_eq!(token.parse("else").unwrap(), "else");
    assert_eq!(token.parse("123").unwrap(), "123");

    let err = token.parse("+").unwrap_err();
    assert_eq!(
        err.expected,
        vec![
            "expected \"if\"".to_string(),
            "expected \"else\"".to_string(),
            "expected at least one matching token".to_string(),
        ]
    );
}

#[test]
fn array_choice_still_works() {
    let kw = choice([literal("a"), literal("b"), literal("c")]);
    assert_eq!(kw.parse("b").unwrap(), "b");
}

// A `[d, d, …]` grammar exercising delimited + separated_by, used for round-trip.
fn int_list<'i>() -> impl Parser<&'i str, Output = Vec<&'i str>> + Generate {
    delimited(
        literal("["),
        separated_by(digits(), literal(",")),
        literal("]"),
    )
}

#[test]
fn generate_round_trips_through_the_sugar() {
    for seed in 0..64u64 {
        let sample = generate(&int_list(), seed);
        assert!(
            int_list().parse(sample.as_str()).is_ok(),
            "seed {seed} produced {sample:?}"
        );
    }
}
