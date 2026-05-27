//! Tests for the `Generate` trait implementations.
//!
//! Every combinator that implements `Generate` is exercised here so that
//! `generate_into` bodies (typed.rs lines ~1578–1730) appear in the coverage
//! report. The tests also cover two edge cases inside `Gen::char_matching`:
//!
//! - **Extended scan** (lines 1539–1545): triggered when no character in the
//!   fixed printable pool satisfies the predicate — the scanner falls through
//!   to the full 0x20..0x7f range.
//! - **Fallback 'a'** (line 1546): triggered when no printable ASCII at all
//!   satisfies the predicate — `'\0'` is such a predicate.
//!
//! Where the generated input is guaranteed to round-trip (literal, digits, etc.)
//! the tests parse it back.  For edge-case predicates that are known not to
//! round-trip (the `'\0'` fallback) only the structural invariant is checked.

use nimble_parsec_rs::typed::{
    any, choice, digits, empty, eof, eventually, generate, literal, lookahead, not, repeated_until,
    satisfy, Parser,
};

// ── AnyChar ───────────────────────────────────────────────────────────────────

#[test]
fn generate_any_char_produces_one_character() {
    // `AnyChar::generate_into` (typed.rs lines 1578–1582) delegates to
    // `Gen::char_matching(|_| true)`, always returning one character.
    // Bind `p` so that S is unified from the .parse() call below.
    let p = any();
    let input = generate(&p, 42);
    assert_eq!(
        input.chars().count(),
        1,
        "any() generate should yield exactly one char, got {input:?}"
    );
    // The generated char is accepted by `any()`.
    p.parse(input.as_str()).unwrap();
}

// ── Satisfy: pool hit ─────────────────────────────────────────────────────────

#[test]
fn generate_satisfy_predicate_in_pool() {
    // A digit predicate matches many characters in the pool, so the first scan
    // succeeds (typed.rs lines 1530–1538).
    let p = satisfy("digit", |c: char| c.is_ascii_digit());
    let input = generate(&p, 7);
    assert!(!input.is_empty());
    p.parse(input.as_str()).unwrap();
}

// ── Satisfy: extended scan ────────────────────────────────────────────────────

#[test]
fn generate_satisfy_extended_scan_when_not_in_pool() {
    // '!' (0x21) is not in the fixed POOL but IS in the 0x20–0x7e range.
    // `Gen::char_matching` exhausts the pool without a hit and falls through
    // to the extended scan (typed.rs lines 1539–1545), returning '!'.
    let p = satisfy("exclamation", |c: char| c == '!');
    let input = generate(&p, 0);
    assert_eq!(input, "!", "extended scan should find '!'; got {input:?}");
    p.parse(input.as_str()).unwrap();
}

// ── Satisfy: fallback 'a' ─────────────────────────────────────────────────────

#[test]
fn generate_satisfy_fallback_when_no_printable_ascii_matches() {
    // `'\0'` (NUL) matches no character in the pool OR the 0x20–0x7e range.
    // `Gen::char_matching` falls back to returning 'a' (typed.rs line 1546).
    // This does NOT round-trip, but it must not panic and must yield one char.
    // Use turbofish to pin S = &str (no .parse() call in this test).
    let input = generate(&satisfy::<&str, _>("nul", |c: char| c == '\0'), 0);
    assert_eq!(
        input.chars().count(),
        1,
        "fallback must yield exactly one char, got {input:?}"
    );
}

// ── Eof ───────────────────────────────────────────────────────────────────────

#[test]
fn generate_eof_yields_empty_string() {
    // `Eof::generate_into` (typed.rs lines 1599–1601) emits nothing.
    let p = eof();
    let input = generate(&p, 0);
    assert!(input.is_empty());
    p.parse(input.as_str()).unwrap();
}

// ── Empty ─────────────────────────────────────────────────────────────────────

#[test]
fn generate_empty_yields_empty_string() {
    // `Empty::generate_into` (typed.rs lines 1276–1278) emits nothing.
    let p = empty();
    let input = generate(&p, 0);
    assert!(input.is_empty());
    p.parse(input.as_str()).unwrap();
}

// ── Map ───────────────────────────────────────────────────────────────────────

#[test]
fn generate_map_delegates_to_inner() {
    // `Map::generate_into` (typed.rs lines 1603–1607) calls the inner parser's
    // generate, discarding the mapping function.
    let p = digits().map(|s: &str| s.len());
    let input = generate(&p, 1);
    assert!(
        !input.is_empty(),
        "Map should produce the inner parser's output"
    );
    p.parse(input.as_str()).unwrap();
}

// ── TryMap ────────────────────────────────────────────────────────────────────

#[test]
fn generate_try_map_delegates_to_inner() {
    // `TryMap::generate_into` (typed.rs lines 1609–1613) — same delegate pattern.
    let p = digits().try_map(|s: &str| Ok::<usize, String>(s.len()));
    let input = generate(&p, 2);
    assert!(!input.is_empty());
    p.parse(input.as_str()).unwrap();
}

// ── To ────────────────────────────────────────────────────────────────────────

#[test]
fn generate_to_delegates_to_inner() {
    // `To::generate_into` (typed.rs lines 1615–1619) — delegate.
    // Use turbofish to pin S = &str (no .parse() call for the u8 output type).
    let input = generate(&literal::<&str, _>("hello").to(99u8), 0);
    assert_eq!(input, "hello");
}

// ── Fold ──────────────────────────────────────────────────────────────────────

#[test]
fn generate_fold_runs_inner_zero_to_two_times() {
    // `Fold::generate_into` (typed.rs lines 1621–1627) calls `gen.below(3)`
    // and appends that many copies of the inner parser's output.
    // Use turbofish to pin S = &str.
    let p = literal::<&str, _>("a").fold(|| 0u64, |acc, _| acc + 1);
    for seed in 0..8u64 {
        let input = generate(&p, seed);
        // Must be 0, 1, or 2 copies of "a".
        assert!(
            input.chars().all(|c| c == 'a') && input.len() <= 2,
            "seed {seed}: unexpected fold output {input:?}"
        );
    }
}

// ── Ignored ───────────────────────────────────────────────────────────────────

#[test]
fn generate_ignored_delegates_to_inner() {
    // `Ignored::generate_into` (typed.rs lines 1629–1633).
    let input = generate(&literal::<&str, _>("x").ignored(), 0);
    assert_eq!(input, "x");
}

// ── Labelled ─────────────────────────────────────────────────────────────────

#[test]
fn generate_labelled_delegates_to_inner() {
    // `Labelled::generate_into` (typed.rs lines 1635–1639).
    let input = generate(&literal::<&str, _>("ok").labelled("the word ok"), 0);
    assert_eq!(input, "ok");
}

// ── WithByteOffset ────────────────────────────────────────────────────────────

#[test]
fn generate_with_byte_offset_delegates_to_inner() {
    // `WithByteOffset::generate_into` (typed.rs lines 1641–1645).
    let input = generate(&literal::<&str, _>("abc").with_byte_offset(), 0);
    assert_eq!(input, "abc");
}

// ── WithLine ─────────────────────────────────────────────────────────────────

#[test]
fn generate_with_line_delegates_to_inner() {
    // `WithLine::generate_into` (typed.rs lines 1647–1651).
    let input = generate(&literal::<&str, _>("x").with_line(), 0);
    assert_eq!(input, "x");
}

// ── Debug ─────────────────────────────────────────────────────────────────────

#[test]
fn generate_debug_delegates_to_inner() {
    // `Debug::generate_into` (typed.rs lines 1653–1657).
    let input = generate(&literal::<&str, _>("hi").debug("label"), 0);
    assert_eq!(input, "hi");
}

// ── PostTraverse ──────────────────────────────────────────────────────────────

#[test]
fn generate_post_traverse_delegates_to_inner() {
    // `PostTraverse::generate_into` (typed.rs lines 1659–1663).
    let p = literal::<&str, _>("xyz").post_traverse(|out: &str, _cursor| Ok::<&str, String>(out));
    let input = generate(&p, 0);
    assert_eq!(input, "xyz");
}

// ── PreTraverse ───────────────────────────────────────────────────────────────

#[test]
fn generate_pre_traverse_delegates_to_inner() {
    // `PreTraverse::generate_into` (typed.rs lines 1665–1669).
    let p = literal::<&str, _>("abc").pre_traverse(|out: &str, _cursor| Ok::<&str, String>(out));
    let input = generate(&p, 0);
    assert_eq!(input, "abc");
}

// ── ThenIgnore ────────────────────────────────────────────────────────────────

#[test]
fn generate_then_ignore_emits_both_parts() {
    // `ThenIgnore::generate_into` (typed.rs lines 1685–1690) generates both
    // first and second, even though only first's output is kept at parse time.
    let p = literal("a").then_ignore(literal("b"));
    let input = generate(&p, 0);
    assert_eq!(input, "ab");
    p.parse(input.as_str()).unwrap();
}

// ── Opt ───────────────────────────────────────────────────────────────────────

#[test]
fn generate_opt_produces_inner_or_empty() {
    // `Opt::generate_into` (typed.rs lines 1702–1708) picks 50/50 via `gen.coin()`.
    for seed in 0..8u64 {
        let p = literal("a").optional();
        let input = generate(&p, seed);
        assert!(
            input.is_empty() || input == "a",
            "seed {seed}: unexpected optional output {input:?}"
        );
        p.parse(input.as_str()).unwrap();
    }
}

// ── Repeated with max ─────────────────────────────────────────────────────────

#[test]
fn generate_repeated_with_max_clamps_count() {
    // When `max` is set, `Repeated::generate_into` (typed.rs line 1713–1714)
    // clamps the generated count to `max`.
    for seed in 0..8u64 {
        let p = literal("a").repeated_in(0, 2);
        let input = generate(&p, seed);
        assert!(
            input.len() <= 2,
            "max=2 but got {} bytes: {input:?}",
            input.len()
        );
        p.parse(input.as_str()).unwrap();
    }
}

// ── Lookahead (no-op generate) ────────────────────────────────────────────────

#[test]
fn generate_lookahead_contributes_nothing() {
    // `Lookahead::generate_into` (typed.rs line 1724–1726) emits nothing.
    let input = generate(&lookahead(literal::<&str, _>("x")), 0);
    assert!(input.is_empty(), "lookahead generate should be a no-op");
}

// ── Not (no-op generate) ──────────────────────────────────────────────────────

#[test]
fn generate_not_contributes_nothing() {
    // `Not::generate_into` (typed.rs line 1728–1730) emits nothing.
    let input = generate(&not(literal::<&str, _>("x")), 0);
    assert!(input.is_empty(), "not generate should be a no-op");
}

// ── ChoiceOf — array variant ──────────────────────────────────────────────────

#[test]
fn generate_choice_array_picks_one_alternative() {
    // `ChoiceOf<[P; N]>::generate_into` (typed.rs lines 1083–1087) calls
    // `[P; N]::generate_alt` (lines 1054–1059), which picks randomly from the
    // non-empty array.
    for seed in 0..6u64 {
        let p = choice([literal("x"), literal("y"), literal("z")]);
        let input = generate(&p, seed);
        assert!(
            input == "x" || input == "y" || input == "z",
            "seed {seed}: unexpected choice output {input:?}"
        );
        p.parse(input.as_str()).unwrap();
    }
}

// ── ChoiceOf — tuple variant ──────────────────────────────────────────────────

#[test]
fn generate_choice_tuple_picks_one_alternative() {
    // The macro-generated `GenerateAlternatives for (P0, P1)` impl
    // (typed.rs lines 1062–1073) is exercised by a two-element tuple choice.
    for seed in 0..4u64 {
        let p = choice((literal("hello"), literal("world")));
        let input = generate(&p, seed);
        assert!(
            input == "hello" || input == "world",
            "seed {seed}: unexpected tuple choice output {input:?}"
        );
        p.parse(input.as_str()).unwrap();
    }
}

// ── RepeatedUntil ─────────────────────────────────────────────────────────────

#[test]
fn generate_repeated_until_emits_zero_to_two_body_copies() {
    // `RepeatedUntil::generate_into` (typed.rs lines 1246–1253) generates
    // `gen.below(3)` (= 0, 1, or 2) copies of the body.
    // Use turbofish to pin S = &str since there is no .parse() call.
    let p = repeated_until(literal::<&str, _>("a"), literal::<&str, _>("END"));
    for seed in 0..4u64 {
        let input = generate(&p, seed);
        // All chars must be 'a'; length is 0–2.
        assert!(
            input.chars().all(|c| c == 'a') && input.len() <= 2,
            "seed {seed}: unexpected repeated_until output {input:?}"
        );
    }
}

// ── Eventually ────────────────────────────────────────────────────────────────

#[test]
fn generate_eventually_emits_inner_directly() {
    // `Eventually::generate_into` (typed.rs lines 1416–1421) generates the
    // inner parser directly (zero-skip prefix), so the output round-trips.
    let p = eventually(literal("TARGET"));
    let input = generate(&p, 0);
    assert_eq!(input, "TARGET");
    p.parse(input.as_str()).unwrap();
}
