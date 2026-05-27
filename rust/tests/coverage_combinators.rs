//! Coverage for the combinators that had the fewest tests, derived from the
//! Elixir `choice/2`, `eventually/2`, `label/3`, `debug/2`, `line/2`,
//! `byte_offset/2`, and other describe blocks.

use nimble_parsec_rs::typed::{
    delimited, digits, empty, eventually, integer, literal, recursive, satisfy, separated_by1,
    Parser,
};

// ── choice with empty() as fallback ──────────────────────────────────────────

#[test]
fn choice_with_empty_as_fallback_always_succeeds() {
    // Mirrors Elixir's `choice([ascii_char([?a..?z]), empty()])`.
    // When the satisfy predicate fails, `empty()` matches unconsumed.
    let p = satisfy("digit", |c: char| c.is_ascii_digit())
        .ignored()
        .or(empty());

    // Digit present → satisfy wins, output is ().
    assert_eq!(p.parse("5").unwrap(), ());

    // Non-digit → satisfy fails, empty() wins.
    assert_eq!(p.parse_partial("x").unwrap(), ((), "x"));

    // Empty input → satisfy fails, empty() wins.
    assert_eq!(p.parse("").unwrap(), ());
}

// ── eventually ───────────────────────────────────────────────────────────────

#[test]
fn eventually_complex_inner_skips_to_pattern() {
    // Inner parser: `literal("a").then(integer())` — a literal followed by digits.
    // eventually skips character by character until the compound pattern matches.
    let p = eventually(literal("a").then(integer()));
    let (out, rest) = p.parse_partial("xyz a42 rest").unwrap();
    assert_eq!(out, ("a", 42));
    assert_eq!(rest, " rest");
}

#[test]
fn eventually_with_repeated_finds_all_occurrences() {
    // Mirrors Elixir: `repeat(eventually(hour))`.
    // Each `eventually` advances past non-matching chars to find the next "X";
    // `repeated` collects them all.
    let p = eventually(literal("X")).repeated();
    let results = p.parse("aXbXcX").unwrap();
    assert_eq!(results, vec!["X", "X", "X"]);
}

#[test]
fn eventually_fails_with_message_when_never_found() {
    let err = eventually(literal("X")).parse("abc").unwrap_err();
    assert_eq!(err.reason, "expected the parser to eventually match");
}

// ── labelled ─────────────────────────────────────────────────────────────────

#[test]
fn labelled_on_or_chain_replaces_error() {
    // Mirrors Elixir: `choice([…]) |> label("something")`.
    // The label completely replaces the compound `or`-joined reason.
    let err = literal("a")
        .or(literal("b"))
        .labelled("a or b")
        .parse("c")
        .unwrap_err();
    assert_eq!(err.reason, "a or b");
    assert_eq!(err.expected, vec!["a or b"]);
}

#[test]
fn labelled_on_sequence_wraps_mid_failure() {
    // The label wraps the entire sequence combinator; mid-sequence failures
    // (second literal fails) are reported under the outer label.
    let err = literal("a")
        .then(literal("b"))
        .labelled("pair ab")
        .parse("ax")
        .unwrap_err();
    assert_eq!(err.reason, "pair ab");
    assert_eq!(err.expected, vec!["pair ab"]);
}

#[test]
fn labelled_success_does_not_alter_output() {
    // On success, `labelled` is transparent.
    let result = literal("ok").labelled("the word ok").parse("ok").unwrap();
    assert_eq!(result, "ok");
}

// ── delimited error cases ─────────────────────────────────────────────────────

#[test]
fn delimited_missing_close_produces_close_error() {
    let err = delimited(literal("("), digits(), literal(")"))
        .parse("(42")
        .unwrap_err();
    assert_eq!(err.reason, "expected \")\"");
}

#[test]
fn delimited_missing_open_produces_open_error() {
    let err = delimited(literal("("), digits(), literal(")"))
        .parse("42)")
        .unwrap_err();
    assert_eq!(err.reason, "expected \"(\"");
}

#[test]
fn delimited_success_returns_content_only() {
    // Sanity: the open/close delimiters are discarded, only content returned.
    assert_eq!(
        delimited(literal("["), digits(), literal("]"))
            .parse("[99]")
            .unwrap(),
        "99"
    );
}

// ── separated_by1 ─────────────────────────────────────────────────────────────

#[test]
fn separated_by1_error_on_empty_input() {
    // When the input is empty the first item fails, bubbling its error.
    // `digits()` uses `take_while1`, so the error message is the take_while1 message.
    let err = separated_by1(digits(), literal(","))
        .parse("")
        .unwrap_err();
    assert!(!err.reason.is_empty(), "expected a non-empty error reason");
    assert!(err.rest.is_empty()); // failed at position 0
}

#[test]
fn separated_by1_single_item() {
    assert_eq!(
        separated_by1(digits(), literal(",")).parse("42").unwrap(),
        vec!["42"]
    );
}

// ── recursive with explicit shallow depth cap ─────────────────────────────────

fn parens<'i>() -> impl Parser<'i, Output = u32> {
    recursive(|expr| {
        literal("(")
            .ignore_then(expr)
            .then_ignore(literal(")"))
            .map(|depth: u32| depth + 1)
            .or(literal("x").map(|_| 0u32))
    })
}

#[test]
fn recursive_shallow_depth_cap_fails_gracefully() {
    // With max_depth=2, inputs requiring more than 2 Recursive crossings fail.
    // "(((x)))" needs 4 crossings → exceeds cap of 2.
    // A depth of 2 is safe on any default stack; no extra thread needed.
    let err = parens()
        .parse_with_max_depth("(((x)))", 2)
        .unwrap_err();
    assert!(
        err.reason.contains("maximum recursion depth exceeded"),
        "reason was: {}",
        err.reason
    );
}

#[test]
fn recursive_within_depth_cap_succeeds() {
    // Each nesting level costs one extra Recursive crossing.
    // "x"   (0 nesting levels) = 1 Recursive crossing needed (the initial parens() call).
    assert_eq!(parens().parse_with_max_depth("x", 1).unwrap(), 0);
    // "(x)" (1 nesting level)  = 2 Recursive crossings needed.
    assert_eq!(parens().parse_with_max_depth("(x)", 2).unwrap(), 1);
    // "((x))" (2 nesting levels) = 3 crossings; budget=3 is just enough.
    assert_eq!(parens().parse_with_max_depth("((x))", 3).unwrap(), 2);
}

// ── with_line newline tracking ────────────────────────────────────────────────

#[test]
fn with_line_tracks_newlines_correctly() {
    // No newline: line 1, line_start_offset 0.
    let (out, (line, start)) = literal("x").with_line().parse("x").unwrap();
    assert_eq!(out, "x");
    assert_eq!(line, 1);
    assert_eq!(start, 0);

    // One newline before the final char:
    //   "a\nb" → consume "a\n" (2 bytes), then "b"; after "b" we are on line 2,
    //   line_start_offset = 2 (byte just after '\n').
    let (out2, (line2, start2)) = literal("a")
        .then_ignore(literal("\n"))
        .then_ignore(literal("b"))
        .with_line()
        .parse("a\nb")
        .unwrap();
    assert_eq!(out2, "a");
    assert_eq!(line2, 2);
    assert_eq!(start2, 2);
}

// ── integer greedy parsing ────────────────────────────────────────────────────

#[test]
fn integer_greedy_partial_parse() {
    // integer() is greedy — it reads all leading digits, leaving the rest.
    let (n, rest) = integer().parse_partial("123abc").unwrap();
    assert_eq!(n, 123);
    assert_eq!(rest, "abc");
}

#[test]
fn integer_leading_zeros_are_consumed() {
    // NimbleParsec also parses "042" as 42 (the Elixir test confirms this).
    assert_eq!(integer().parse("042").unwrap(), 42);
}
