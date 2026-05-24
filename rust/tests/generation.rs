use nimble_parsec_rs::{
    ascii_char, choice, concat, generate, ignore, integer_exact, integer_min, optional, recursive,
    string, utf8_string, AsciiPredicate, Parser, Utf8Predicate,
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
