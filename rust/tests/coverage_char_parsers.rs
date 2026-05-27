//! Character-level parser tests, mirroring the Elixir suite's `ascii_char`,
//! `utf8_char`, `ascii_string`, and `utf8_string` describe blocks.
//!
//! Covers: `satisfy` with explicit ranges/exclusions, `one_of`, `none_of`,
//! `take_while`/`take_while1` edge cases, `any` on ASCII and multi-byte Unicode.

use nimble_parsec_rs::typed::{any, none_of, one_of, satisfy, take_while, take_while1, Parser};

// ── satisfy: explicit character ranges ───────────────────────────────────────

#[test]
fn satisfy_with_digit_range() {
    let digit = satisfy("digit 0-9", |c: char| c.is_ascii_digit());

    assert_eq!(digit.parse("5").unwrap(), '5');
    assert_eq!(digit.parse("0").unwrap(), '0');
    assert_eq!(digit.parse("9").unwrap(), '9');

    let err = digit.parse("a").unwrap_err();
    // The label IS the error reason (not "expected digit 0-9").
    assert_eq!(err.reason, "digit 0-9");
    assert_eq!(err.expected, vec!["digit 0-9"]);
}

#[test]
fn satisfy_with_uppercase_range() {
    let upper = satisfy("uppercase A-Z", |c: char| c.is_ascii_uppercase());

    assert_eq!(upper.parse("M").unwrap(), 'M');
    assert_eq!(upper.parse("A").unwrap(), 'A');
    assert_eq!(upper.parse("Z").unwrap(), 'Z');

    assert!(upper.parse("m").is_err());
    assert!(upper.parse("1").is_err());
}

#[test]
fn satisfy_exclusion_pattern() {
    // Model of Elixir's `ascii_char([not: ?\n])`.
    let not_newline = satisfy("not a newline", |c: char| c != '\n');

    assert_eq!(not_newline.parse("x").unwrap(), 'x');
    assert_eq!(not_newline.parse(" ").unwrap(), ' ');

    let err = not_newline.parse("\n").unwrap_err();
    assert_eq!(err.reason, "not a newline");
}

// ── one_of / none_of ─────────────────────────────────────────────────────────

#[test]
fn one_of_matches_any_in_set() {
    let op = one_of("+-*/");

    assert_eq!(op.parse("+").unwrap(), '+');
    assert_eq!(op.parse("-").unwrap(), '-');
    assert_eq!(op.parse("*").unwrap(), '*');
    assert_eq!(op.parse("/").unwrap(), '/');

    let err = op.parse("x").unwrap_err();
    // The hardcoded label in the implementation.
    assert_eq!(err.reason, "one of an expected set");
    assert_eq!(err.expected, vec!["one of an expected set"]);
}

#[test]
fn none_of_rejects_set_members() {
    let not_space = none_of(" \t");

    assert_eq!(not_space.parse("a").unwrap(), 'a');
    assert_eq!(not_space.parse("x").unwrap(), 'x');

    let err = not_space.parse(" ").unwrap_err();
    assert_eq!(err.reason, "a character outside an excluded set");

    assert!(not_space.parse("\t").is_err());

    // Empty exclusion set — every character passes.
    assert_eq!(none_of("").parse("z").unwrap(), 'z');
    assert_eq!(none_of("").parse("\n").unwrap(), '\n');
}

// ── take_while edge cases ────────────────────────────────────────────────────

#[test]
fn take_while_on_empty_returns_empty_str() {
    // `take_while` succeeds even when nothing matches, returning "".
    let p = take_while(|c: char| c.is_alphabetic());
    assert_eq!(p.parse("").unwrap(), "");
}

#[test]
fn take_while_stops_at_first_nonmatch() {
    let p = take_while(|c: char| c.is_alphabetic());
    let (matched, rest) = p.parse_partial("abc123").unwrap();
    assert_eq!(matched, "abc");
    assert_eq!(rest, "123");
}

#[test]
fn take_while_on_fully_matching_input() {
    let p = take_while(|c: char| c.is_alphabetic());
    assert_eq!(p.parse("abcxyz").unwrap(), "abcxyz");
}

#[test]
fn take_while1_on_empty_fails_with_message() {
    let err = take_while1(|c: char| c.is_alphabetic())
        .parse("")
        .unwrap_err();
    assert_eq!(err.reason, "expected at least one matching character");
}

// ── any on ASCII ──────────────────────────────────────────────────────────────

#[test]
fn any_on_single_ascii_char() {
    assert_eq!(any().parse("Z").unwrap(), 'Z');
    assert_eq!(any().parse(" ").unwrap(), ' ');
    assert_eq!(any().parse("0").unwrap(), '0');
}

// ── any on Unicode multi-byte codepoints ─────────────────────────────────────

#[test]
fn any_on_unicode_multibyte_char() {
    // 'é'  = U+00E9  — 2 UTF-8 bytes
    assert_eq!(any().parse("é").unwrap(), 'é');

    // '€'  = U+20AC  — 3 UTF-8 bytes
    assert_eq!(any().parse("€").unwrap(), '€');

    // '🦀' = U+1F980 — 4 UTF-8 bytes (the Rust crab emoji)
    assert_eq!(any().parse("🦀").unwrap(), '🦀');

    // `parse` requires full consumption: each string above is exactly one
    // codepoint, so the full-consumption check passes.
    assert!(any().parse("éx").is_err()); // two codepoints → leftover "x"
}

// ── take_while on a Unicode string ───────────────────────────────────────────

#[test]
fn take_while_on_unicode_string() {
    // `is_alphabetic` is Unicode-aware: 'é','h','l','o' are all alphabetic.
    let p = take_while(|c: char| c.is_alphabetic());
    let (matched, rest) = p.parse_partial("héllo wörld").unwrap();
    assert_eq!(matched, "héllo");
    assert_eq!(rest, " wörld");

    // Full Unicode word.
    assert_eq!(p.parse("héllo").unwrap(), "héllo");
}
