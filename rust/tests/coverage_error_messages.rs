//! Error-message fidelity tests — every distinct `reason` string and the
//! `expected` field, derived from the Elixir NimbleParsec test suite.
//!
//! The Elixir suite tests `{:error, @error, rest, ...}` with exact message
//! strings. We do the same here against the Rust `ParseFailure` fields.

use nimble_parsec_rs::typed::{
    any, bytes, choice, eof, eventually, integer, literal, not, satisfy, take_while1, Parser,
};

// ── Leaf-combinator errors ────────────────────────────────────────────────────

#[test]
fn literal_error_message_format() {
    let err = literal("foo").parse("bar").unwrap_err();
    assert_eq!(err.reason, "expected \"foo\"");
    assert_eq!(err.expected, vec!["expected \"foo\""]);
    // The rest is the full unmodified input (no consumption on failure).
    assert_eq!(err.rest, "bar");
}

#[test]
fn any_on_empty_string() {
    let err = any().parse("").unwrap_err();
    assert_eq!(err.reason, "expected any token");
    assert_eq!(err.expected, vec!["expected any token"]);
}

#[test]
fn satisfy_uses_its_label_as_full_reason() {
    // The label passed to `satisfy` IS the full error reason — it does NOT get
    // wrapped in "expected …". Matches the Elixir `@error` annotation pattern.
    let err = satisfy("a digit", |c: char| c.is_ascii_digit())
        .parse("x")
        .unwrap_err();
    assert_eq!(err.reason, "a digit");
    assert_eq!(err.expected, vec!["a digit"]);
}

#[test]
fn take_while1_on_empty_fails() {
    let err = take_while1(|c: char| c.is_alphabetic())
        .parse("")
        .unwrap_err();
    assert_eq!(err.reason, "expected at least one matching token");
    assert_eq!(err.expected, vec!["expected at least one matching token"]);
}

#[test]
fn take_while1_on_nonmatch_fails() {
    // Characters present but none satisfy the predicate → same message.
    let err = take_while1(|c: char| c.is_alphabetic())
        .parse("123")
        .unwrap_err();
    assert_eq!(err.reason, "expected at least one matching token");
}

#[test]
fn eof_when_input_remains() {
    let err = eof().parse("x").unwrap_err();
    assert_eq!(err.reason, "expected end of input");
    assert_eq!(err.expected, vec!["expected end of input"]);
}

#[test]
fn parse_requires_all_input_consumed() {
    // `parse` succeeds only when the full string is consumed; leftover text
    // triggers an "expected end of input" error carrying the remainder.
    let err = literal("foo").parse("foobar").unwrap_err();
    assert_eq!(err.reason, "expected end of input");
    assert_eq!(err.rest, "bar");

    // `parse_partial` succeeds and hands back the rest.
    assert_eq!(
        literal("foo").parse_partial("foobar").unwrap(),
        ("foo", "bar")
    );
}

// ── Alternation / choice errors ───────────────────────────────────────────────

#[test]
fn or_joins_two_branch_reasons() {
    let err = literal("a").or(literal("b")).parse("c").unwrap_err();
    assert_eq!(err.reason, "expected \"a\" or expected \"b\"");
    assert_eq!(err.expected, vec!["expected \"a\"", "expected \"b\""]);
}

#[test]
fn or_joins_three_branch_reasons() {
    // `a.or(b).or(c)` nests as `Or<Or<A,B>, C>`.  The inner Or produces
    // `"expected \"a\" or expected \"b\""` as its reason, which becomes the
    // first argument to the outer join.
    let err = literal("a")
        .or(literal("b"))
        .or(literal("c"))
        .parse("x")
        .unwrap_err();
    assert_eq!(
        err.reason,
        "expected \"a\" or expected \"b\" or expected \"c\""
    );
    assert_eq!(
        err.expected,
        vec!["expected \"a\"", "expected \"b\"", "expected \"c\""]
    );
}

#[test]
fn choice_joins_all_alternative_reasons() {
    let err = choice([literal("if"), literal("else"), literal("while")])
        .parse("for")
        .unwrap_err();
    assert_eq!(
        err.reason,
        "expected \"if\" or expected \"else\" or expected \"while\""
    );
    // The three expectations are all present.
    assert!(err.expected.contains(&"expected \"if\"".to_string()));
    assert!(err.expected.contains(&"expected \"else\"".to_string()));
    assert!(err.expected.contains(&"expected \"while\"".to_string()));
}

// ── Negative-assertion error ──────────────────────────────────────────────────

#[test]
fn not_matched_error_message() {
    // `not` uses `ParseFailure::rejected` — the `expected` field is empty.
    let err = not(literal("x")).parse("x").unwrap_err();
    assert_eq!(err.reason, "did not expect the lookahead parser to match");
    assert!(
        err.expected.is_empty(),
        "not's expected should be empty; got {:?}",
        err.expected
    );
}

// ── Semantic-rejection errors (empty `expected`) ──────────────────────────────

#[test]
fn integer_on_empty_or_nondigit() {
    let err_empty = integer().parse("").unwrap_err();
    assert_eq!(err_empty.reason, "expected an integer");
    assert_eq!(err_empty.expected, vec!["expected an integer"]);

    let err_alpha = integer().parse("x").unwrap_err();
    assert_eq!(err_alpha.reason, "expected an integer");
    assert_eq!(err_alpha.expected, vec!["expected an integer"]);
}

#[test]
fn integer_overflow_message() {
    // Overflow uses `ParseFailure::rejected` → `expected` is empty.
    let err = integer().parse("99999999999999999999").unwrap_err();
    assert_eq!(err.reason, "integer out of range");
    assert!(
        err.expected.is_empty(),
        "integer overflow expected should be empty; got {:?}",
        err.expected
    );
}

// ── Byte-level errors ─────────────────────────────────────────────────────────

#[test]
fn bytes_too_few_message() {
    let err5 = bytes(5).parse("abc").unwrap_err();
    assert_eq!(err5.reason, "expected 5 bytes");
    assert_eq!(err5.expected, vec!["expected 5 bytes"]);

    let err3 = bytes(3).parse("ab").unwrap_err();
    assert_eq!(err3.reason, "expected 3 bytes");
}

// ── `eventually` error ────────────────────────────────────────────────────────

#[test]
fn eventually_not_found_message() {
    let err = eventually(literal("X")).parse("abc").unwrap_err();
    assert_eq!(err.reason, "expected the parser to eventually match");
    assert_eq!(
        err.expected,
        vec!["expected the parser to eventually match"]
    );
}
