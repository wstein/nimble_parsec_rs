//! Tests for the `ignored()` combinator and the `debug` pass-through, ported
//! from the Elixir `ignore/2 combinator` describe blocks.
//!
//! Elixir's `ignore(p)` discards the accumulator entry; Rust's `.ignored()`
//! discards the parser output, returning `()` on success and propagating the
//! original error unchanged on failure.

use nimble_parsec_rs::typed::{digits, literal, Parser};
use nimble_parsec_rs::nimble;

// ── ignored() success / failure ───────────────────────────────────────────────

#[test]
fn ignored_returns_unit_on_success() {
    // Mirroring Elixir: `compile_ignore("TO") == {:ok, [], "", ...}`
    // In Rust the output is `()` rather than an empty list.
    assert_eq!(literal("x").ignored().parse("x").unwrap(), ());
}

#[test]
fn ignored_propagates_failure() {
    // On failure the original error reason is preserved unchanged.
    let err = literal("x").ignored().parse("y").unwrap_err();
    assert_eq!(err.reason, "expected \"x\"");
}

// ── Chaining with ignored() ───────────────────────────────────────────────────

#[test]
fn ignored_then_pair() {
    // `literal("(").ignored()` has Output=(); `.then(digits())` pairs it.
    let p = literal("(").ignored().then(digits());
    assert_eq!(p.parse("(42").unwrap(), ((), "42"));
}

#[test]
fn then_with_ignored_second() {
    // Ignore the second element of a `.then` pair.
    let p = literal("a").then(literal("b").ignored());
    assert_eq!(p.parse("ab").unwrap(), ("a", ()));
}

#[test]
fn ignored_then_ignore_then_sequence() {
    // Three-combinator chain where the middle element is discarded.
    // `literal("(").then_ignore(literal(",")).then(literal(")"))` keeps first + last.
    let p = literal("a").then_ignore(literal(",")).then(literal("b"));
    assert_eq!(p.parse("a,b").unwrap(), ("a", "b"));
}

// ── nimble::ignore alias ──────────────────────────────────────────────────────

#[test]
fn nimble_ignore_alias_works() {
    // `nimble::ignore` is the NimbleParsec-named alias for `.ignored()`.
    assert_eq!(nimble::ignore(literal("x")).parse("x").unwrap(), ());

    let err = nimble::ignore(literal("x")).parse("y").unwrap_err();
    assert_eq!(err.reason, "expected \"x\"");
}

#[test]
fn nimble_ignore_on_digit_run() {
    // Mirrors Elixir: `ignore(ascii_char([?a..?z]) |> times(min: 1))`.
    let p = nimble::ignore(digits());
    assert_eq!(p.parse("123").unwrap(), ());
    assert!(p.parse("abc").is_err());
}

// ── debug pass-through ────────────────────────────────────────────────────────

#[test]
fn debug_on_success_passes_output_through() {
    // `.debug` emits tracing to stderr but does not alter the parse result.
    let result = digits().debug("digits").parse("123").unwrap();
    assert_eq!(result, "123");
}

#[test]
fn debug_on_failure_passes_error_through() {
    // The error reason is unchanged when debug wraps a failing parser.
    let err = literal("x").debug("lit").parse("y").unwrap_err();
    assert_eq!(err.reason, "expected \"x\"");
}
