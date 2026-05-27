//! Tests for the typed `Parser<Output>` core (RFC 0001, phase 2).

use nimble_parsec_rs::typed::{
    any, choice, digits, eof, literal, lookahead, none_of, not, one_of, recursive, satisfy,
    take_while, take_while1, Parser,
};

#[test]
fn literal_matches_its_prefix_or_fails() {
    assert_eq!(literal("foo").parse("foo").unwrap(), "foo");
    assert_eq!(
        literal("foo").parse_partial("foobar").unwrap(),
        ("foo", "bar")
    );
    let err = literal("foo").parse("bar").unwrap_err();
    assert_eq!(err.reason, "expected \"foo\"");
    assert_eq!(err.expected, vec!["expected \"foo\"".to_string()]);
}

#[test]
fn parse_requires_full_consumption_but_partial_does_not() {
    // `parse` insists on reaching end of input; `parse_partial` returns the rest.
    assert!(literal("foo").parse("foobar").is_err());
    assert_eq!(literal("foo").parse_partial("foobar").unwrap().1, "bar");
    assert!(literal("foo").then(eof()).parse("foo").is_ok());
}

#[test]
fn map_then_and_ignore_variants_thread_typed_output() {
    // `then` yields a tuple; the outputs keep their concrete types.
    let pair = literal("a").then(literal("b"));
    assert_eq!(pair.parse("ab").unwrap(), ("a", "b"));

    // ignore_then / then_ignore keep one side.
    assert_eq!(
        literal("<").ignore_then(literal("x")).parse("<x").unwrap(),
        "x"
    );
    assert_eq!(
        literal("x").then_ignore(literal(">")).parse("x>").unwrap(),
        "x"
    );

    // map transforms the output type (&str -> u32).
    let number = digits().map(|d: &str| d.parse::<u32>().unwrap());
    assert_eq!(number.parse("042").unwrap(), 42);
}

#[test]
fn or_tries_branches_and_unions_expectations() {
    let p = literal("yes").or(literal("no"));
    assert_eq!(p.parse("yes").unwrap(), "yes");
    assert_eq!(p.parse("no").unwrap(), "no");

    let err = p.parse("maybe").unwrap_err();
    assert_eq!(err.reason, "expected \"yes\" or expected \"no\"");
    assert_eq!(
        err.expected,
        vec![
            "expected \"yes\"".to_string(),
            "expected \"no\"".to_string()
        ]
    );
}

#[test]
fn optional_and_repeated_collect_typed_values() {
    assert_eq!(literal("x").optional().parse("x").unwrap(), Some("x"));
    assert_eq!(literal("x").optional().parse("").unwrap(), None);

    let many = literal("ab").repeated();
    assert_eq!(many.parse("ababab").unwrap(), vec!["ab", "ab", "ab"]);
    assert_eq!(
        literal("ab").repeated().parse("").unwrap(),
        Vec::<&str>::new()
    );

    // repeated_at_least enforces a minimum.
    assert!(literal("ab").repeated_at_least(2).parse("ab").is_err());
    assert!(literal("ab").repeated_at_least(2).parse("abab").is_ok());
}

#[test]
fn character_leaves_match_classes() {
    assert_eq!(any().parse("Z").unwrap(), 'Z');
    assert_eq!(
        satisfy("a vowel", |c| "aeiou".contains(c))
            .parse("e")
            .unwrap(),
        'e'
    );
    assert!(satisfy("a vowel", |c| "aeiou".contains(c))
        .parse("z")
        .is_err());

    assert_eq!(
        take_while(|c: char| c.is_alphabetic())
            .parse("abc")
            .unwrap(),
        "abc"
    );
    // take_while accepts the empty match; take_while1 does not.
    assert_eq!(
        take_while(|c: char| c.is_numeric())
            .parse_partial("abc")
            .unwrap()
            .0,
        ""
    );
    assert!(take_while1(|c: char| c.is_numeric()).parse("abc").is_err());
}

#[test]
fn labelled_overrides_the_failure_message() {
    let err = literal("{{")
        .labelled("a tag opener")
        .parse("xy")
        .unwrap_err();
    assert_eq!(err.reason, "a tag opener");
    assert_eq!(err.expected, vec!["a tag opener".to_string()]);
}

#[test]
fn to_replaces_the_output_with_a_constant() {
    #[derive(Debug, PartialEq, Clone)]
    enum Op {
        And,
        Or,
    }
    let op = literal("&&").to(Op::And).or(literal("||").to(Op::Or));
    assert_eq!(op.parse("&&").unwrap(), Op::And);
    assert_eq!(op.parse("||").unwrap(), Op::Or);
}

#[test]
fn try_map_can_fail_the_parse() {
    // Parse digits, then reject values that don't fit the domain.
    let byte = digits().try_map(|d: &str| {
        d.parse::<u16>()
            .ok()
            .filter(|&n| n <= 255)
            .ok_or_else(|| format!("{d} is out of range"))
    });
    assert_eq!(byte.parse("200").unwrap(), 200);
    let err = byte.parse("999").unwrap_err();
    assert_eq!(err.reason, "999 is out of range");
    assert!(err.expected.is_empty());
}

#[test]
fn lookahead_and_not_are_zero_width() {
    // `lookahead` peeks without consuming.
    let peek = lookahead(literal("ab")).then(literal("a"));
    assert_eq!(peek.parse_partial("abc").unwrap(), (("ab", "a"), "bc"));

    // `not` succeeds only when the inner parser would fail, consuming nothing.
    let not_close = not(literal("}}")).ignore_then(any());
    assert_eq!(not_close.parse("x").unwrap(), 'x');
    assert!(not_close.parse("}}").is_err());
}

#[test]
fn one_of_none_of_and_choice() {
    assert_eq!(one_of("+-*/").parse("*").unwrap(), '*');
    assert!(one_of("+-*/").parse("x").is_err());
    assert_eq!(none_of(" \t").parse("a").unwrap(), 'a');
    assert!(none_of(" \t").parse(" ").is_err());

    // n-way choice over homogeneous alternatives.
    let kw = choice([literal("if"), literal("else"), literal("while")]);
    assert_eq!(kw.parse("else").unwrap(), "else");
    assert_eq!(kw.parse("while").unwrap(), "while");
    let err = kw.parse("for").unwrap_err();
    assert_eq!(
        err.reason,
        "expected \"if\" or expected \"else\" or expected \"while\""
    );
}

#[test]
fn bounded_repetition() {
    let p = literal("a").repeated_in(2, 3);
    assert!(p.parse("a").is_err()); // below min
    assert_eq!(p.parse("aa").unwrap().len(), 2);
    // stops at max, leaving the rest
    assert_eq!(p.parse_partial("aaaa").unwrap(), (vec!["a", "a", "a"], "a"));
}

// A balanced-parenthesis grammar that returns its nesting depth, exercising the
// typed `recursive`.
fn parens<'i>() -> impl Parser<&'i str, Output = u32> {
    recursive(|expr| {
        literal("(")
            .ignore_then(expr)
            .then_ignore(literal(")"))
            .map(|depth: u32| depth + 1)
            .or(literal("x").map(|_| 0u32))
    })
}

#[test]
fn recursive_grammar_yields_typed_output() {
    assert_eq!(parens().parse("x").unwrap(), 0);
    assert_eq!(parens().parse("(((x)))").unwrap(), 3);
    assert!(parens().parse("((x)").is_err());
}

#[test]
fn recursive_grammar_is_depth_capped() {
    // Mirrors the Value-API recursion guard: pathologically deep input fails
    // gracefully rather than overflowing the stack. Run on a large stack so the
    // (debug-bloated) frames can reach the cap before exhausting it.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let input = format!("{}x{}", "(".repeat(50_000), ")".repeat(50_000));
            let err = parens().parse(input.as_str()).unwrap_err();
            // The cap message bubbles up through the enclosing `or`s, which join
            // their branches' reasons, so it is a substring.
            assert!(
                err.reason.contains("maximum recursion depth exceeded"),
                "reason was: {}",
                err.reason
            );
        })
        .expect("spawn")
        .join()
        .expect("thread panicked");
}
