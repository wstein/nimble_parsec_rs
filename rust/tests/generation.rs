use nimble_parsec_rs::{
    ascii_char, choice, concat, generate, generate_with, ignore, integer_exact, integer_min,
    optional, recursive, repeat, string, utf8_string, AsciiPredicate, GenerateConfig, Parser,
    Utf8Predicate,
};

fn datetime() -> Parser {
    let date = concat(
        integer_exact(4),
        concat(
            ignore(string("-")),
            concat(
                integer_exact(2),
                concat(ignore(string("-")), integer_exact(2)),
            ),
        ),
    );
    let time = concat(
        integer_exact(2),
        concat(
            ignore(string(":")),
            concat(
                integer_exact(2),
                concat(ignore(string(":")), integer_exact(2)),
            ),
        ),
    );
    concat(date, concat(ignore(string("T")), time))
}

#[test]
fn generate_round_trips_non_recursive_grammars() {
    let parsers = vec![
        string("hello"),
        integer_min(1),
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        utf8_string(vec![Utf8Predicate::Range('a'..='z')], 1, Some(5)),
        choice(vec![string("foo"), string("bar")]),
        concat(
            optional(ascii_char(vec![AsciiPredicate::Char(b'-')])),
            integer_min(1),
        ),
        datetime(),
    ];

    for parser in &parsers {
        for seed in 0..25u64 {
            let input = generate(parser, seed);
            let ok = parser
                .parse(&input)
                .unwrap_or_else(|e| panic!("generated {input:?} did not parse: {}", e.reason));
            assert_eq!(ok.rest, "", "generated {input:?} was not fully consumed");
        }
    }
}

#[test]
fn generate_is_deterministic_for_a_seed() {
    let parser = datetime();
    assert_eq!(generate(&parser, 7), generate(&parser, 7));
}

#[test]
fn generate_with_repeat_window_zero_emits_exactly_min() {
    // An unbounded repeat with repeat_window 0 produces exactly `min` items.
    let parser = repeat(
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        2,
        None,
    );
    let config = GenerateConfig {
        repeat_window: 0,
        ..Default::default()
    };
    let input = generate_with(&parser, 1, config);
    assert_eq!(input.chars().count(), 2);
    assert!(parser.parse(&input).is_ok());
}

#[test]
fn generate_with_shallow_depth_still_terminates() {
    let parens = recursive(|expr| {
        choice(vec![
            concat(ignore(string("(")), concat(expr, ignore(string(")")))),
            string("x"),
        ])
    });
    let config = GenerateConfig {
        max_recursion_depth: 2,
        ..Default::default()
    };
    for seed in 0..10u64 {
        let input = generate_with(&parens, seed, config);
        assert!(parens.parse(&input).is_ok(), "did not parse: {input:?}");
    }
}

#[test]
fn generate_terminates_and_round_trips_recursive_grammar() {
    let parens = recursive(|expr| {
        choice(vec![
            concat(ignore(string("(")), concat(expr, ignore(string(")")))),
            string("x"),
        ])
    });

    for seed in 0..10u64 {
        let input = generate(&parens, seed);
        let ok = parens
            .parse(&input)
            .unwrap_or_else(|e| panic!("generated {input:?} did not parse: {}", e.reason));
        assert_eq!(ok.rest, "");
    }
}
