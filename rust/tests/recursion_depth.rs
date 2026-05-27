//! Recursion-depth cap: deeply nested input must fail gracefully with a
//! `ParseFailure`, never overflow the native call stack (an uncatchable abort).
//!
//! These tests are the regression net for the one confirmed pre-release
//! security finding (web-facing DoS via `recursive`). Stem's differential
//! harness can never generate this class of input, so it lives here.
//!
//! All cases run on a thread with a large explicit stack. The cap is sized for
//! *release* builds on a 2 MiB stack; *debug* builds have ~30 KiB frames, so a
//! big stack here neutralizes that build-specific bloat and lets these tests
//! verify the cap *logic* (it trips at N levels) rather than the host's stack
//! size, which would otherwise make them flaky across debug/release.

use nimble_parsec_rs::{choice, concat, ignore, recursive, string, Parser};

// The balanced-parenthesis grammar from `references.rs`: "(" expr ")" | "x".
// Each level of nesting crosses the recursive reference exactly once.
fn paren_grammar() -> Parser {
    recursive(|expr| {
        choice(vec![
            concat(ignore(string("(")), concat(expr, ignore(string(")")))),
            string("x"),
        ])
    })
}

// `depth` pairs of parens around the base case `x`, e.g. depth 3 => "(((x)))".
fn nested(depth: usize) -> String {
    format!("{}x{}", "(".repeat(depth), ")".repeat(depth))
}

// Run `f` on a 64 MiB-stack thread so debug frame bloat can't overflow before
// the cap trips; panics (failed asserts) propagate as test failures.
fn on_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn test thread")
        .join()
        .expect("test thread panicked");
}

#[test]
fn pathologically_deep_input_fails_gracefully_under_default_cap() {
    on_big_stack(|| {
        // Far deeper than the cap. Without the bound this recurses until the
        // stack is exhausted and the process aborts; with it we get a clean
        // error. That this returns at all (rather than crashing) is the proof.
        let input = nested(50_000);
        let err = paren_grammar()
            .parse(&input)
            .expect_err("input nested far beyond the cap must fail, not crash");
        // The cap message bubbles up through the enclosing `choice`es, which
        // append their own alternative failures, so it's a substring.
        assert!(
            err.reason.contains("maximum recursion depth exceeded"),
            "reason was: {}",
            err.reason
        );
    });
}

#[test]
fn custom_cap_is_honored() {
    on_big_stack(|| {
        let input = nested(500);
        let err = paren_grammar()
            .parse_with_max_depth(&input, 64)
            .expect_err("nesting beyond the custom cap must fail");
        assert!(
            err.reason.contains("maximum recursion depth exceeded"),
            "reason was: {}",
            err.reason
        );
    });
}

#[test]
fn nesting_within_the_cap_still_parses() {
    on_big_stack(|| {
        // The cap must not break legitimate recursion. Depth below the default
        // still succeeds and emits the single base-case token.
        let input = nested(100);
        let ok = paren_grammar()
            .parse(&input)
            .expect("nesting within the cap should parse");
        assert_eq!(ok.rest, "");

        // And right up against a custom cap (each "(" spends one unit; the base
        // case `x` is reached at the final reference crossing).
        let shallow = nested(8);
        assert!(paren_grammar().parse_with_max_depth(&shallow, 16).is_ok());
    });
}

#[test]
fn budget_resets_between_runs_and_after_failure() {
    on_big_stack(|| {
        let parser = paren_grammar();

        // A run that trips the cap must not leave the thread-local budget
        // drained for the next run on the same thread.
        assert!(parser.parse(&nested(50_000)).is_err());
        assert!(
            parser.parse(&nested(50)).is_ok(),
            "budget must be restored after a depth-exceeded failure"
        );

        // Two successful runs in a row also both succeed.
        assert!(parser.parse(&nested(50)).is_ok());
        assert!(parser.parse(&nested(50)).is_ok());
    });
}

#[test]
fn cap_can_be_raised_above_a_lower_bound() {
    on_big_stack(|| {
        // The same input that fails under a low cap succeeds under a higher one.
        let input = nested(100);
        assert!(paren_grammar().parse_with_max_depth(&input, 50).is_err());
        assert!(paren_grammar().parse_with_max_depth(&input, 150).is_ok());
    });
}
