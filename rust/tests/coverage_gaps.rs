//! Tests targeting the remaining coverage gaps in `lib.rs` and `typed.rs`:
//!
//! - `ParseFailure` Display (`std::fmt::Display`) implementation
//! - `Input::rest()` and `Input::cursor()` public accessors
//! - `Repeated` / `Fold` non-advancing match guard (zero-width inner parser)
//! - `choice` with zero alternatives → `"choice has no options"` error
//! - `SeparatedBy` no-progress guard (zero-width sep + item)
//! - `RepeatedUntil` no-progress guard (zero-width body parser)
//! - `nimble` module free-function aliases not covered by existing tests:
//!   `eos`, `optional`, `times`, `label`, `line`, `debug`, `post_traverse`,
//!   `pre_traverse`

use nimble_parsec_rs::nimble;
use nimble_parsec_rs::typed::{
    choice, empty, literal, repeated_until, separated_by, Eof, Input, Parser,
};

// ── ParseFailure::fmt (Display) ───────────────────────────────────────────────

#[test]
fn parse_failure_display_format() {
    // `ParseFailure` implements `std::fmt::Display`; exercising it covers
    // lib.rs lines 98–105 (the `fmt` method body).
    let err = literal("foo").parse("bar").unwrap_err();
    let displayed = format!("{err}");
    assert!(
        displayed.contains("expected \"foo\""),
        "display should include the reason; got: {displayed}"
    );
    assert!(
        displayed.contains("line 1"),
        "display should include the line number; got: {displayed}"
    );
    assert!(
        displayed.contains("byte offset 0"),
        "display should include the byte offset; got: {displayed}"
    );
}

// ── Input::rest() and Input::cursor() ────────────────────────────────────────

#[test]
fn input_rest_and_cursor_accessors() {
    // Public accessors on `Input` (typed.rs lines 50–57).
    let input = Input::new("hello");
    assert_eq!(input.rest(), "hello");
    assert_eq!(input.cursor().byte_offset, 0);
    assert_eq!(input.cursor().line, 1);
    assert_eq!(input.cursor().line_start_offset, 0);
}

#[test]
fn input_new_empty_string() {
    let input = Input::new("");
    assert_eq!(input.rest(), "");
    assert_eq!(input.cursor().byte_offset, 0);
}

// ── Repeated non-advancing break ─────────────────────────────────────────────

#[test]
fn repeated_stops_on_zero_width_match() {
    // `empty()` succeeds without consuming any input. `Repeated::parse_next`
    // detects that the cursor did not advance (typed.rs lines 514–518) and
    // breaks out of the loop rather than running forever.
    let p = empty().repeated();
    let (items, rest) = p.parse_partial("abc").unwrap();
    assert_eq!(rest, "abc", "nothing should be consumed");
    assert!(
        items.is_empty(),
        "non-advancing guard fires before any item is collected; got {items:?}"
    );
}

// ── Fold non-advancing break ──────────────────────────────────────────────────

#[test]
fn fold_stops_on_zero_width_match() {
    // Same guard logic as `Repeated` but for `Fold` (typed.rs lines 568–570).
    let p = empty().fold(|| 0u32, |acc, _| acc + 1);
    let (count, rest) = p.parse_partial("abc").unwrap();
    assert_eq!(rest, "abc", "nothing should be consumed");
    assert_eq!(count, 0, "no iterations complete before the guard fires");
}

// ── choice with zero alternatives ────────────────────────────────────────────

#[test]
fn choice_zero_alternatives_produces_has_no_options_error() {
    // An empty array reaches the `reasons.is_empty()` branch of
    // `choice_failure` (typed.rs line 966-967), yielding the dedicated message.
    let arr: [Eof<&str>; 0] = [];
    let err = choice(arr).parse("anything").unwrap_err();
    assert_eq!(err.reason, "choice has no options");
}

// ── SeparatedBy no-progress guard ────────────────────────────────────────────

#[test]
fn separated_by_stops_on_zero_width_sep_and_item() {
    // When both the separator and the body item are zero-width (`empty()`),
    // each loop iteration makes no forward progress.  The guard at
    // typed.rs lines 1163–1166 detects this and stops after collecting the
    // first item, preventing an infinite loop.
    let p = separated_by(empty(), empty());
    let (items, rest) = p.parse_partial("abc").unwrap();
    assert_eq!(rest, "abc", "nothing should be consumed");
    assert_eq!(
        items.len(),
        1,
        "exactly one zero-width item is collected before the guard fires"
    );
}

// ── RepeatedUntil no-progress guard ──────────────────────────────────────────

#[test]
fn repeated_until_stops_on_zero_width_body_parser() {
    // `empty()` matches without consuming; `RepeatedUntil::parse_next`
    // detects no progress (typed.rs lines 1230–1233) and stops immediately.
    let p = repeated_until(empty(), literal("X"));
    let (items, rest) = p.parse_partial("abc").unwrap();
    assert_eq!(rest, "abc", "nothing should be consumed");
    assert!(
        items.is_empty(),
        "the guard fires before any zero-width item is collected; got {items:?}"
    );
}

// ── nimble module: eos ────────────────────────────────────────────────────────

#[test]
fn nimble_eos_alias() {
    // `nimble::eos()` delegates to `eof()` (typed.rs lines 1762–1765).
    nimble::eos().parse("").unwrap();
    assert!(nimble::eos().parse("x").is_err());
}

// ── nimble module: optional ───────────────────────────────────────────────────

#[test]
fn nimble_optional_alias() {
    // `nimble::optional` wraps `Parser::optional` (typed.rs lines 1772–1775).
    let p = nimble::optional(literal("x"));
    assert_eq!(p.parse("x").unwrap(), Some("x"));
    assert_eq!(p.parse("").unwrap(), None);
}

// ── nimble module: times ──────────────────────────────────────────────────────

#[test]
fn nimble_times_alias() {
    // `nimble::times(p, n)` repeats exactly `n` times (typed.rs lines 1783–1786).
    let result = nimble::times(literal("ab"), 3).parse("ababab").unwrap();
    assert_eq!(result, vec!["ab", "ab", "ab"]);
    assert!(nimble::times(literal("ab"), 3).parse("abab").is_err());
}

// ── nimble module: label ──────────────────────────────────────────────────────

#[test]
fn nimble_label_alias() {
    // `nimble::label` wraps `Parser::labelled` (typed.rs lines 1812–1815).
    let err = nimble::label(literal("x"), "the letter x")
        .parse("y")
        .unwrap_err();
    assert_eq!(err.reason, "the letter x");
    assert_eq!(err.expected, vec!["the letter x"]);
}

// ── nimble module: line ───────────────────────────────────────────────────────

#[test]
fn nimble_line_alias() {
    // `nimble::line` wraps `Parser::with_line` (typed.rs lines 1827–1830).
    let (out, (line, start)) = nimble::line(literal("x")).parse("x").unwrap();
    assert_eq!(out, "x");
    assert_eq!(line, 1);
    assert_eq!(start, 0);
}

// ── nimble module: debug (free function) ─────────────────────────────────────

#[test]
fn nimble_debug_free_function_alias() {
    // `nimble::debug` (the free function) wraps `Parser::debug`
    // (typed.rs lines 1832–1835). Existing tests only call the method form.
    assert_eq!(nimble::debug(literal("x"), "lbl").parse("x").unwrap(), "x");
    assert!(nimble::debug(literal("x"), "lbl").parse("y").is_err());
}

// ── nimble module: post_traverse ──────────────────────────────────────────────

#[test]
fn nimble_post_traverse_alias() {
    // `nimble::post_traverse` wraps `Parser::post_traverse`
    // (typed.rs lines 1837–1844).
    let p = nimble::post_traverse(literal("hello"), |out: &str, _cursor| {
        Ok::<usize, String>(out.len())
    });
    assert_eq!(p.parse("hello").unwrap(), 5);
}

#[test]
fn nimble_post_traverse_rejection_propagates() {
    // Rejection from the closure is converted to a ParseFailure.
    let p = nimble::post_traverse(literal("x"), |_: &str, _cursor| {
        Err::<&str, String>("bad token".to_string())
    });
    let err = p.parse("x").unwrap_err();
    assert_eq!(err.reason, "bad token");
    assert!(
        err.expected.is_empty(),
        "rejected failure has empty expected"
    );
}

// ── nimble module: pre_traverse ───────────────────────────────────────────────

#[test]
fn nimble_pre_traverse_alias() {
    // `nimble::pre_traverse` wraps `Parser::pre_traverse`
    // (typed.rs lines 1846–1853).
    let p = nimble::pre_traverse(literal("hi"), |out: &str, _cursor| {
        Ok::<usize, String>(out.len())
    });
    assert_eq!(p.parse("hi").unwrap(), 2);
}

#[test]
fn nimble_pre_traverse_rejection_propagates() {
    let p = nimble::pre_traverse(literal("x"), |_: &str, _cursor| {
        Err::<&str, String>("rejected".to_string())
    });
    let err = p.parse("x").unwrap_err();
    assert_eq!(err.reason, "rejected");
    assert!(err.expected.is_empty());
}

// ── Bonus: digits() generates and round-trips (ensure no regress) ─────────────

#[test]
fn parse_failure_is_std_error() {
    // `ParseFailure` implements `std::error::Error`, enabling `?`-based chaining.
    // Verifying this through the trait object is sufficient to mark the impl.
    let err = literal("x").parse("y").unwrap_err();
    let _: &dyn std::error::Error = &err;
}
