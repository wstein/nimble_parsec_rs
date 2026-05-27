//! Property-based tests for the typed API: invariants that should hold across
//! all inputs/depths.

use nimble_parsec_rs::{any, literal, recursive, Parser};
use proptest::prelude::*;

// A balanced-parenthesis grammar returning its nesting depth.
fn parens<'i>() -> impl Parser<&'i str, Output = u32> {
    recursive(|expr| {
        literal("(")
            .ignore_then(expr)
            .then_ignore(literal(")"))
            .map(|d: u32| d + 1)
            .or(literal("x").map(|_| 0u32))
    })
}

proptest! {
    // `literal` consumes exactly its (fixed) prefix and leaves the rest intact.
    #[test]
    fn literal_consumes_exactly_its_prefix(tail in "[^a-z].*|") {
        let input = format!("abc{tail}");
        let (matched, rest) = literal("abc").parse_partial(input.as_str()).unwrap();
        prop_assert_eq!(matched, "abc");
        prop_assert_eq!(rest, tail.as_str());
    }

    // `any().repeated()` is total: it never panics, consumes the whole input,
    // and yields exactly one item per character.
    #[test]
    fn any_repeated_is_total_and_consumes_everything(input in ".*") {
        let chars = any().repeated().parse(input.as_str()).expect("repeated any is total");
        prop_assert_eq!(chars.len(), input.chars().count());
    }

    // A recursive grammar parses correctly at every depth within the cap and
    // never overflows. Depths are kept small so the case is stack-safe in debug.
    #[test]
    fn recursive_depth_round_trips_within_the_cap(depth in 0u32..16) {
        let input = format!("{}x{}", "(".repeat(depth as usize), ")".repeat(depth as usize));
        prop_assert_eq!(parens().parse(input.as_str()).unwrap(), depth);
    }
}
