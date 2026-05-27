//! Parity combinators carrying their NimbleParsec names: `empty`, `integer`,
//! `eventually`.

use nimble_parsec_rs::typed::{empty, eventually, generate, integer, literal, Parser};

#[test]
fn nimbleparsec_terminology_aliases_delegate_to_the_core() {
    // The dedicated `nimble` module carries NimbleParsec free-function names;
    // `use nimble_parsec_rs::nimble::*;` (root re-export) gives the vocabulary.
    use nimble_parsec_rs::nimble::{concat, duplicate, replace, string};

    // string == literal
    assert_eq!(string("ab").parse("ab").unwrap(), "ab");

    // concat == .then
    assert_eq!(
        concat(string("a"), string("b")).parse("ab").unwrap(),
        ("a", "b")
    );

    // replace == .to
    assert_eq!(replace(string("x"), 9u8).parse("x").unwrap(), 9);

    // duplicate(p, n) == .repeated_in(n, n)
    assert_eq!(
        duplicate(string("ab"), 3).parse("ababab").unwrap(),
        vec!["ab", "ab", "ab"]
    );
    assert!(duplicate(string("ab"), 3).parse("abab").is_err());
}

#[test]
fn nimble_module_provides_a_one_import_vocabulary() {
    use nimble_parsec_rs::nimble::*;

    // Free-function pipeline style mirroring NimbleParsec, from one import.
    let grammar = concat(string("("), concat(integer(), string(")")));
    assert_eq!(grammar.parse("(42)").unwrap(), ("(", (42, ")")));

    // method-combinators are available as free functions too
    let tagged = map(integer(), |n| n * 2);
    assert_eq!(tagged.parse("21").unwrap(), 42);
}

#[test]
fn empty_always_succeeds_without_consuming() {
    assert_eq!(empty().parse("").unwrap(), ());
    assert_eq!(empty().then(literal("x")).parse("x").unwrap(), ((), "x"));
    // Useful as an always-matching final alternative.
    let optional_a = literal("a").ignored().or(empty());
    assert_eq!(optional_a.parse("").unwrap(), ());
}

#[test]
fn integer_parses_a_digit_run_into_i64() {
    assert_eq!(integer().parse("042").unwrap(), 42);
    assert!(integer().parse("").is_err());
    assert!(integer().parse("x").is_err());

    // Overflow fails gracefully rather than panicking.
    let err = integer().parse("99999999999999999999").unwrap_err();
    assert_eq!(err.reason, "integer out of range");
}

#[test]
fn integer_generates_round_trippable_values() {
    for seed in 0..32u64 {
        let sample = generate(&integer(), seed);
        assert!(
            integer().parse(&sample).is_ok(),
            "seed {seed} produced {sample:?}"
        );
    }
}

#[test]
fn eventually_skips_until_the_inner_parser_matches() {
    let find = eventually(literal("X"));
    assert_eq!(find.parse_partial("abcXdef").unwrap(), ("X", "def"));
    // Matches immediately when already at the target.
    assert_eq!(
        eventually(literal("X")).parse_partial("Xyz").unwrap(),
        ("X", "yz")
    );
    // Fails if the target never appears.
    assert!(eventually(literal("X")).parse("abc").is_err());
}
