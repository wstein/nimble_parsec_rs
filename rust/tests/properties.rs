//! Property-based tests (the review's "unit + property tests" bar).
//!
//! The per-combinator behavior is pinned by the deterministic suites; these
//! assert invariants that should hold across *all* inputs/seeds:
//!   1. `string` matches exactly its literal prefix.
//!   2. Parsing is total and monotonic — it never panics and never grows `rest`.
//!   3. `generate` round-trips: a generated input re-parses for non-recursive
//!      grammars (the documented contract).
//!   4. Recursive grammars are graceful at any depth — never a stack-overflow.

use nimble_parsec_rs::{
    ascii_string, choice, concat, generate, ignore, integer_min, optional, recursive, string,
    AsciiPredicate, Value,
};
use proptest::prelude::*;

proptest! {
    // 1. `string(lit)` succeeds iff the input starts with `lit`, consuming exactly
    //    `lit` and leaving the rest untouched.
    #[test]
    fn string_consumes_exactly_its_literal_prefix(lit in "[a-z]{1,8}", tail in "[^a-z].*|") {
        let input = format!("{lit}{tail}");
        let parsed = string(lit.clone()).parse(&input).expect("prefix must match");
        prop_assert_eq!(parsed.rest, tail.as_str());
        prop_assert_eq!(parsed.tokens, vec![Value::Str(lit)]);
    }

    // 2. A `repeat(min = 0)` grammar is total (always `Ok`) and never reports a
    //    remainder longer than its input — i.e. parsing only ever consumes.
    #[test]
    fn parsing_is_total_and_monotonic(input in ".*") {
        let grammar = choice(vec![
            integer_min(1),
            ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 1, None),
            string(" "),
        ])
        .repeated(0, None);

        let parsed = grammar.parse(&input).expect("repeat(0, _) cannot fail");
        prop_assert!(parsed.rest.len() <= input.len());
    }

    // 3. For a non-recursive grammar, anything `generate` produces parses back
    //    cleanly with no leftover input (the round-trip contract from the docs).
    #[test]
    fn generate_round_trips_for_non_recursive_grammars(seed in any::<u64>()) {
        // Rebuilt per use because `Parser` is single-ownership (not `Clone`).
        let grammar = || {
            string("key")
                .then(string("="))
                .then(integer_min(1))
                .then(optional(string(";")))
        };
        let input = generate(&grammar(), seed);
        let parsed = grammar().parse(&input);
        prop_assert!(parsed.is_ok(), "generated {input:?} failed to parse");
        prop_assert_eq!(parsed.unwrap().rest, "");
    }

    // 4. A recursive (balanced-paren) grammar parses gracefully at any nesting
    //    depth: within the cap it succeeds, never overflowing the stack. Depths
    //    are kept small so the case is stack-safe even in debug builds; the
    //    beyond-the-cap error path is covered deterministically in
    //    `recursion_depth.rs`.
    #[test]
    fn recursive_grammar_is_graceful_within_the_cap(depth in 0usize..16) {
        let grammar = || {
            recursive(|expr| {
                choice(vec![
                    concat(ignore(string("(")), concat(expr, ignore(string(")")))),
                    string("x"),
                ])
            })
        };
        let input = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
        let parsed = grammar().parse_with_max_depth(&input, 64);
        prop_assert!(parsed.is_ok(), "depth {depth} within cap should parse");
        prop_assert_eq!(parsed.unwrap().rest, "");
    }
}
