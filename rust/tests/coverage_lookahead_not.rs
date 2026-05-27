//! Thorough coverage of `lookahead` and `not` (Elixir's `lookahead_not`),
//! derived from the Elixir `lookahead/2` and `lookahead_not/2` describe blocks.
//!
//! Key properties under test:
//! - Both combinators are **zero-width** — they succeed or fail without
//!   advancing the cursor.
//! - `lookahead` succeeds when the inner parser would succeed, carrying its
//!   output but restoring the input.
//! - `not` succeeds when the inner parser would fail, always yielding `()`.

use nimble_parsec_rs::nimble;
use nimble_parsec_rs::typed::{any, choice, literal, lookahead, not, Parser};

// ── lookahead ─────────────────────────────────────────────────────────────────

#[test]
fn lookahead_with_or_inner_is_zero_width() {
    // The lookahead peeks at the `or`-branch output without consuming it.
    // `.then(any())` then actually consumes the first character.
    let p = lookahead(literal("a").or(literal("b"))).then(any());
    let ((peeked, consumed), rest) = p.parse_partial("ab").unwrap();
    assert_eq!(peeked, "a"); // lookahead saw "a"
    assert_eq!(consumed, 'a'); // any() consumed "a"
    assert_eq!(rest, "b"); // "b" untouched
}

#[test]
fn nested_lookahead_is_zero_width() {
    // Nesting lookaheads: neither level consumes.
    // `.then(literal("x"))` is the first actual consumer.
    let p = lookahead(lookahead(literal("x"))).then(literal("x"));
    assert_eq!(p.parse("x").unwrap(), ("x", "x"));
}

#[test]
fn lookahead_compound_parser() {
    // `lookahead` of a sequence: inner parser succeeds over "ab", but cursor is
    // fully restored so the remainder still starts at "abc".
    let p = lookahead(literal("a").then(literal("b")));
    let (out, rest) = p.parse_partial("abc").unwrap();
    assert_eq!(out, ("a", "b")); // the compound output
    assert_eq!(rest, "abc"); // cursor completely restored
}

#[test]
fn lookahead_used_as_repeated_guard() {
    // Mirrors Elixir: `times(ascii_char([]) |> lookahead(ascii_char([?0..?9])), min: 1)`.
    // Here: repeat consuming `any()` only while the next char is 'a'.
    let p = lookahead(literal("a")).ignore_then(any()).repeated();
    let (items, rest) = p.parse_partial("aaab").unwrap();
    assert_eq!(items, vec!['a', 'a', 'a']);
    assert_eq!(rest, "b");
}

#[test]
fn lookahead_fails_when_inner_fails() {
    // If the inner parser fails, lookahead propagates the failure.
    assert!(lookahead(literal("x")).parse_partial("y").is_err());
}

// ── not ──────────────────────────────────────────────────────────────────────

#[test]
fn not_with_choice_inner() {
    // Succeeds (consuming nothing) when the choice would fail.
    let guard = not(choice([literal("a"), literal("b")]));
    let ((), rest) = guard.parse_partial("cde").unwrap();
    assert_eq!(rest, "cde"); // nothing consumed

    // Fails when any choice branch would succeed.
    assert!(guard.parse_partial("a").is_err());
    assert!(guard.parse_partial("b").is_err());
}

#[test]
fn not_with_compound_inner() {
    // `not(a.then(b))` — fails only when the full sequence matches.
    let guard = not(literal("a").then(literal("b")));

    // "ac" — first literal matches but second doesn't → compound fails → not succeeds.
    let ((), rest) = guard.parse_partial("ac").unwrap();
    assert_eq!(rest, "ac");

    // "ab" — full sequence matches → not fails.
    assert!(guard.parse("ab").is_err());
}

#[test]
fn not_error_message_is_exact() {
    let err = not(literal("x")).parse("x").unwrap_err();
    assert_eq!(err.reason, "did not expect the lookahead parser to match");
    // `not` uses `ParseFailure::rejected` — `expected` is empty.
    assert!(
        err.expected.is_empty(),
        "`not` expected should be empty; got {:?}",
        err.expected
    );
}

#[test]
fn not_as_guard_collect_until_semicolon() {
    // Canonical "repeat until delimiter" pattern: guard on ';', then consume
    // one character.  Mirrors Elixir's `repeat(lookahead_not(…) |> …)`.
    let p = not(literal(";")).ignore_then(any()).repeated();
    let (chars, rest) = p.parse_partial("abc;rest").unwrap();
    assert_eq!(chars, vec!['a', 'b', 'c']);
    assert_eq!(rest, ";rest");
}

#[test]
fn not_preserves_no_consumption_on_success() {
    // When `not` succeeds it must leave the cursor exactly where it found it.
    let p = not(literal("END")).then(literal("s"));
    let (((), s), rest) = p.parse_partial("start").unwrap();
    assert_eq!(s, "s");
    assert_eq!(rest, "tart");
}

#[test]
fn not_succeeds_on_empty_input_if_inner_fails() {
    // `not(literal("x"))` on "" — the inner parser fails (nothing to match),
    // so `not` succeeds consuming nothing.
    let ((), rest) = not(literal("x")).parse_partial("").unwrap();
    assert_eq!(rest, "");
}

// ── nimble::lookahead_not alias ───────────────────────────────────────────────

#[test]
fn nimble_lookahead_not_alias() {
    // `nimble::lookahead_not` is the NimbleParsec-named alias for `not`.
    let ((), rest) = nimble::lookahead_not(literal("x"))
        .parse_partial("y")
        .unwrap();
    assert_eq!(rest, "y");

    assert!(nimble::lookahead_not(literal("x")).parse("x").is_err());
}
