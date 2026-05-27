//! Tests for the position, debug, and generation combinators:
//! `with_byte_offset`, `with_line`, `debug`, and `generate`.

use nimble_parsec_rs::typed::{digits, generate, literal, take_while, Generate};
use nimble_parsec_rs::Parser;

#[test]
fn with_byte_offset_pairs_the_output_with_the_trailing_offset() {
    let p = literal("ab").with_byte_offset();
    assert_eq!(p.parse("ab").unwrap(), ("ab", 2));

    // The offset is where the match *ended*.
    let q = literal("xy").ignore_then(literal("z")).with_byte_offset();
    assert_eq!(q.parse("xyz").unwrap(), ("z", 3));
}

#[test]
fn with_line_reports_line_and_line_start_offset() {
    // Consume everything, including a newline, then read the position.
    let p = take_while(|_c: char| true).with_line();
    let (consumed, (line, line_start)) = p.parse("a\nbc").unwrap();
    assert_eq!(consumed, "a\nbc");
    assert_eq!(line, 2); // one newline crossed
    assert_eq!(line_start, 2); // byte offset just past the '\n'
}

#[test]
fn debug_passes_the_output_through() {
    // `.debug` only traces to stderr; the parse result is unchanged.
    let p = digits().debug("digits");
    assert_eq!(p.parse("123").unwrap(), "123");
    assert!(literal("x").debug("lit").parse("y").is_err());
}

// A non-recursive grammar built from literals / alternation / repetition, which
// `generate` can round-trip.
fn letters<'i>() -> impl Parser<&'i str, Output = Vec<&'i str>> + Generate {
    literal("a").or(literal("b")).repeated()
}

fn tagged_number<'i>() -> impl Parser<&'i str, Output = (&'i str, &'i str)> + Generate {
    literal("(").ignore_then(digits()).then(literal(")"))
}

#[test]
fn generate_is_deterministic_for_a_seed() {
    assert_eq!(generate(&letters(), 12345), generate(&letters(), 12345));
}

#[test]
fn generated_input_round_trips_through_the_parser() {
    for seed in 0..64u64 {
        let sample = generate(&letters(), seed);
        assert!(
            letters().parse(sample.as_str()).is_ok(),
            "letters seed {seed} produced {sample:?}"
        );

        let sample = generate(&tagged_number(), seed);
        let parsed = tagged_number().parse(sample.as_str());
        assert!(
            parsed.is_ok(),
            "tagged_number seed {seed} produced {sample:?}"
        );
    }
}

#[test]
fn generate_produces_variety_across_seeds() {
    use std::collections::HashSet;
    let samples: HashSet<String> = (0..64u64).map(|s| generate(&letters(), s)).collect();
    // The grammar is open-ended, so distinct seeds should not all collapse to
    // one string.
    assert!(samples.len() > 1);
}
