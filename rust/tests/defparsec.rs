use nimble_parsec_rs::Integer;
use nimble_parsec_rs::{
    choice, concat, defcombinator, defcombinatorp, defparsec, defparsecp, ignore, integer_exact,
    integer_min, string, times, Parser, TimesOptions, Value,
};

/// Asserts a codegen-built parser and the runtime-built equivalent agree on
/// tokens/rest for successes and on the reason for failures, across `inputs`.
fn assert_parity(compiled: &Parser, runtime: &Parser, inputs: &[&str]) {
    for &input in inputs {
        match (compiled.parse(input), runtime.parse(input)) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.tokens, b.tokens, "tokens differ for {input:?}");
                assert_eq!(a.rest, b.rest, "rest differs for {input:?}");
            }
            (Err(a), Err(b)) => assert_eq!(a.reason, b.reason, "reason differs for {input:?}"),
            _ => panic!("codegen and runtime disagree on success/failure for {input:?}"),
        }
    }
}

/// Asserts a `compile_parser!`-built parser actually took the specialized path
/// rather than silently falling back to the runtime interpreter. A specialized
/// parser is wrapped in `Ast::Native`, so `Parser`'s structural `Debug` renders
/// as `Parser { ast: Native(<fn>) }`; a fallback renders the underlying `Ast`
/// tree (e.g. `Parser { ast: Bytes(3) }`). We anchor on the `ast: Native(`
/// field prefix rather than a bare `Native(` so a string/`Fail` literal that
/// happens to contain `Native(`, or a `Native` nested deeper in a fallback
/// tree, can't produce a false positive.
fn assert_specialized(parser: &Parser) {
    let debug = format!("{parser:?}");
    assert!(
        debug.contains("ast: Native("),
        "expected a specialized (Native) parser, got: {debug}"
    );
}

// ---------------------------------------------------------------------------
// defparsec! — generates a public parse function with codegen
// ---------------------------------------------------------------------------

defparsec!(
    parse_date,
    concat(
        concat(
            integer_exact(4),
            concat(ignore(string("-")), integer_exact(2)),
        ),
        concat(ignore(string("-")), integer_exact(2))
    )
);

#[test]
fn defparsec_codegen_parse_succeeds() {
    let ok = parse_date("2024-03-15 rest").expect("date parse");
    assert_eq!(ok.rest, " rest");
    assert_eq!(ok.cursor.byte_offset, 10);
    assert_eq!(ok.tokens.len(), 3);
    assert_eq!(ok.tokens[0], Value::Int(Integer::from(2024u32)));
    assert_eq!(ok.tokens[1], Value::Int(Integer::from(3u32)));
    assert_eq!(ok.tokens[2], Value::Int(Integer::from(15u32)));
}

#[test]
fn defparsec_codegen_parse_fails() {
    assert!(parse_date("not-a-date").is_err());
}

// ---------------------------------------------------------------------------
// defparsec! — falls back to OnceLock runtime for unsupported combinators
// ---------------------------------------------------------------------------

// `times` has no codegen arm, so the macro must fall back to a OnceLock-cached
// runtime parser rather than emitting specialized inline code.
defparsec!(parse_xs, times(string("x"), TimesOptions::exact(2)));

#[test]
fn defparsec_runtime_fallback_parses() {
    let ok = parse_xs("xxrest").expect("two x's");
    assert_eq!(ok.rest, "rest");
    assert_eq!(
        ok.tokens,
        vec![Value::Str("x".to_string()), Value::Str("x".to_string())]
    );
    // Below the required count must fail.
    assert!(parse_xs("xy").is_err());
}

// ---------------------------------------------------------------------------
// defparsecp! — private function
// ---------------------------------------------------------------------------

defparsecp!(private_parse_ab, concat(string("a"), string("b")));

#[test]
fn defparsecp_generates_private_function() {
    let ok = private_parse_ab("abXY").expect("ab parse");
    assert_eq!(ok.rest, "XY");
    assert_eq!(ok.tokens.len(), 2);
}

// ---------------------------------------------------------------------------
// defcombinator! / defcombinatorp!
// ---------------------------------------------------------------------------

defcombinator!(two_digit_int, integer_exact(2));

#[test]
fn defcombinator_returns_reusable_parser() {
    let p = two_digit_int();
    let ok1 = p.parse("12rest").expect("first parse");
    let ok2 = p.parse("99end").expect("second parse");
    assert_eq!(ok1.tokens, vec![Value::Int(Integer::from(12u32))]);
    assert_eq!(ok2.tokens, vec![Value::Int(Integer::from(99u32))]);
}

#[test]
fn defcombinator_parser_is_composable() {
    let p = concat(
        two_digit_int(),
        concat(ignore(string(":")), two_digit_int()),
    );
    let ok = p.parse("12:34end").expect("composed parse");
    assert_eq!(ok.rest, "end");
    assert_eq!(ok.tokens.len(), 2);
}

defcombinatorp!(private_combinator, string("x"));

#[test]
fn defcombinatorp_generates_private_combinator() {
    let ok = private_combinator().parse("xyz").expect("x parse");
    assert_eq!(ok.rest, "yz");
}

// ---------------------------------------------------------------------------
// compile_parser! codegen — ignore suppresses allocation
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_codegen_matches_runtime() {
    use nimble_parsec_rs::compile_parser;

    let compiled = compile_parser!(concat(
        integer_exact(4),
        concat(ignore(string("-")), integer_exact(2))
    ));
    let runtime = concat(
        integer_exact(4),
        concat(ignore(string("-")), integer_exact(2)),
    );

    let c = compiled.parse("2024-03").expect("compiled");
    let r = runtime.parse("2024-03").expect("runtime");
    assert_eq!(c.tokens, r.tokens);
    assert_eq!(c.rest, r.rest);
    assert_eq!(c.cursor.byte_offset, r.cursor.byte_offset);
}

// ---------------------------------------------------------------------------
// choice codegen
// ---------------------------------------------------------------------------

defparsec!(
    parse_keyword,
    choice(vec![string("foo"), string("bar"), string("baz")])
);

#[test]
fn defparsec_choice_codegen() {
    assert_eq!(parse_keyword("bar!").expect("parses").rest, "!");
    assert_eq!(
        parse_keyword("foo").expect("parses").tokens,
        vec![Value::Str("foo".to_string())]
    );
    let err = parse_keyword("qux").expect_err("no branch matches");
    assert!(
        err.reason.contains("foo") && err.reason.contains("baz") && err.reason.contains(" or ")
    );
}

#[test]
fn compile_parser_ascii_char_matches_runtime() {
    use nimble_parsec_rs::{ascii_char, compile_parser, AsciiPredicate};

    let compiled = compile_parser!(concat(
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        ascii_char(vec![
            AsciiPredicate::Range(b'0'..=b'9'),
            AsciiPredicate::NotChar(b'3')
        ])
    ));
    let runtime = concat(
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        ascii_char(vec![
            AsciiPredicate::Range(b'0'..=b'9'),
            AsciiPredicate::NotChar(b'3'),
        ]),
    );

    for input in ["a5", "a3", "x", "z9!", ""] {
        match (compiled.parse(input), runtime.parse(input)) {
            (Ok(c), Ok(r)) => {
                assert_eq!(c.tokens, r.tokens, "tokens differ for {input:?}");
                assert_eq!(c.rest, r.rest, "rest differs for {input:?}");
            }
            (Err(c), Err(r)) => assert_eq!(c.reason, r.reason, "reason differs for {input:?}"),
            _ => panic!("codegen and runtime disagree for {input:?}"),
        }
    }
}

#[test]
fn compile_parser_utf8_char_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, utf8_char, Utf8Predicate};

    let compiled = compile_parser!(utf8_char(vec![Utf8Predicate::Range('a'..='z')]));
    let runtime = utf8_char(vec![Utf8Predicate::Range('a'..='z')]);

    for input in ["a", "é", "0", "z!", ""] {
        match (compiled.parse(input), runtime.parse(input)) {
            (Ok(c), Ok(r)) => {
                assert_eq!(c.tokens, r.tokens, "tokens differ for {input:?}");
                assert_eq!(c.rest, r.rest, "rest differs for {input:?}");
            }
            (Err(c), Err(r)) => assert_eq!(c.reason, r.reason, "reason differs for {input:?}"),
            _ => panic!("codegen and runtime disagree for {input:?}"),
        }
    }
}

#[test]
fn compile_parser_choice_matches_runtime() {
    use nimble_parsec_rs::compile_parser;

    let compiled = compile_parser!(concat(
        choice(vec![string("foo"), string("bar")]),
        integer_min(1)
    ));
    let runtime = concat(choice(vec![string("foo"), string("bar")]), integer_min(1));

    for input in ["foo12!", "bar7", "xyz", "foo"] {
        match (compiled.parse(input), runtime.parse(input)) {
            (Ok(c), Ok(r)) => {
                assert_eq!(c.tokens, r.tokens, "tokens differ for {input:?}");
                assert_eq!(c.rest, r.rest, "rest differs for {input:?}");
            }
            (Err(c), Err(r)) => {
                assert_eq!(c.reason, r.reason, "reason differs for {input:?}");
                assert_eq!(c.rest, r.rest, "error rest differs for {input:?}");
            }
            _ => panic!("codegen and runtime disagree on success/failure for {input:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// transform/tagging codegen parity
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_tag_wrap_replace_match_runtime() {
    use nimble_parsec_rs::{compile_parser, replace, tag, wrap, Value};

    let tagged = compile_parser!(tag("n", integer_min(1)));
    assert_parity(&tagged, &tag("n", integer_min(1)), &["42", "x"]);

    let wrapped = compile_parser!(wrap(concat(
        integer_exact(2),
        concat(ignore(string("-")), integer_exact(2))
    )));
    let wrapped_rt = wrap(concat(
        integer_exact(2),
        concat(ignore(string("-")), integer_exact(2)),
    ));
    assert_parity(&wrapped, &wrapped_rt, &["12-34", "12x", "ab"]);

    let replaced = compile_parser!(replace(integer_min(1), Value::Str("NUM".to_string())));
    assert_parity(
        &replaced,
        &replace(integer_min(1), Value::Str("NUM".to_string())),
        &["7", "x"],
    );
}

#[test]
fn compile_parser_map_reduce_match_runtime() {
    use nimble_parsec_rs::{compile_parser, map, reduce, Value};

    let mapped = compile_parser!(map(integer_min(1), |v| match v {
        Value::Int(n) => Value::Int(n + Integer::from(1)),
        other => other,
    }));
    let mapped_rt = map(integer_min(1), |v| match v {
        Value::Int(n) => Value::Int(n + Integer::from(1)),
        other => other,
    });
    assert_parity(&mapped, &mapped_rt, &["5", "x"]);

    let reduced = compile_parser!(reduce(
        concat(integer_min(1), concat(ignore(string(",")), integer_min(1))),
        |tokens| {
            let total: Integer = tokens
                .into_iter()
                .filter_map(|t| match t {
                    Value::Int(n) => Some(n),
                    _ => None,
                })
                .sum();
            Value::Int(total)
        }
    ));
    let reduced_rt = reduce(
        concat(integer_min(1), concat(ignore(string(",")), integer_min(1))),
        |tokens| {
            let total: Integer = tokens
                .into_iter()
                .filter_map(|t| match t {
                    Value::Int(n) => Some(n),
                    _ => None,
                })
                .sum();
            Value::Int(total)
        },
    );
    assert_parity(&reduced, &reduced_rt, &["3,4", "3x"]);
}

#[test]
fn compile_parser_unwrap_and_tag_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, unwrap_and_tag};

    // Single token: tagged. Two tokens: must error identically.
    let single = compile_parser!(unwrap_and_tag("n", integer_min(1)));
    assert_parity(&single, &unwrap_and_tag("n", integer_min(1)), &["42", "x"]);

    let pair = compile_parser!(unwrap_and_tag(
        "pair",
        concat(integer_min(1), concat(ignore(string(",")), integer_min(1)))
    ));
    let pair_rt = unwrap_and_tag(
        "pair",
        concat(integer_min(1), concat(ignore(string(",")), integer_min(1))),
    );
    assert_parity(&pair, &pair_rt, &["3,4", "9"]);
}

#[test]
fn compile_parser_byte_offset_and_line_match_runtime() {
    use nimble_parsec_rs::{byte_offset, compile_parser, line};

    let bo = compile_parser!(byte_offset(integer_min(1)));
    assert_parity(&bo, &byte_offset(integer_min(1)), &["123!", "x"]);

    let ln = compile_parser!(line(integer_min(1)));
    assert_parity(&ln, &line(integer_min(1)), &["42", "x"]);
}

// ---------------------------------------------------------------------------
// optional / repeat codegen parity
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_optional_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, optional};

    let p = compile_parser!(concat(optional(string("-")), integer_min(1)));
    let rt = concat(optional(string("-")), integer_min(1));
    assert_parity(&p, &rt, &["-5", "5", "x", ""]);
}

#[test]
fn compile_parser_repeat_matches_runtime() {
    use nimble_parsec_rs::{ascii_char, compile_parser, repeat, AsciiPredicate};

    let bounded = compile_parser!(repeat(
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        1,
        Some(3)
    ));
    let bounded_rt = repeat(
        ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]),
        1,
        Some(3),
    );
    assert_parity(&bounded, &bounded_rt, &["abcd", "a", "", "1"]);

    let unbounded = compile_parser!(repeat(string("ab"), 0, None));
    assert_parity(
        &unbounded,
        &repeat(string("ab"), 0, None),
        &["ababx", "x", ""],
    );

    // Below `min`: the inner failure must propagate identically.
    let min_two = compile_parser!(repeat(string("ab"), 2, None));
    assert_parity(
        &min_two,
        &repeat(string("ab"), 2, None),
        &["abab", "ab", "x"],
    );
}

// ---------------------------------------------------------------------------
// duplicate / eventually / repeat_while / traversal codegen parity
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_duplicate_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, duplicate};

    let p = compile_parser!(duplicate(string("ab"), 3));
    assert_parity(
        &p,
        &duplicate(string("ab"), 3),
        &["ababab!", "abab", "x", ""],
    );
}

#[test]
fn compile_parser_eventually_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, eventually};

    let p = compile_parser!(eventually(integer_min(1)));
    assert_parity(
        &p,
        &eventually(integer_min(1)),
        &["abc12!", "12", "abc", ""],
    );
}

#[test]
fn compile_parser_repeat_while_matches_runtime() {
    use nimble_parsec_rs::{
        ascii_char, compile_parser, repeat_while, AsciiPredicate, RepeatWhileControl,
    };

    fn pair() -> nimble_parsec_rs::Parser {
        concat(
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
        )
    }
    let p = compile_parser!(repeat_while(
        concat(
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')])
        ),
        |rest, _, _| if rest.starts_with('3') {
            RepeatWhileControl::Halt
        } else {
            RepeatWhileControl::Cont
        },
        0,
        None
    ));
    let rt = repeat_while(
        pair(),
        |rest, _, _| {
            if rest.starts_with('3') {
                RepeatWhileControl::Halt
            } else {
                RepeatWhileControl::Cont
            }
        },
        0,
        None,
    );
    assert_parity(&p, &rt, &["12345", "1234", "31", ""]);
}

#[test]
fn compile_parser_traversals_match_runtime() {
    use nimble_parsec_rs::{compile_parser, post_traverse, pre_traverse, Value};

    // post_traverse: result depends on the trailing position.
    let post = compile_parser!(post_traverse(integer_min(1), |_tokens, ctx, cursor| {
        Ok((
            vec![Value::Int(Integer::from(cursor.byte_offset as i64))],
            ctx,
        ))
    }));
    let post_rt = post_traverse(integer_min(1), |_tokens, ctx, cursor| {
        Ok((
            vec![Value::Int(Integer::from(cursor.byte_offset as i64))],
            ctx,
        ))
    });
    assert_parity(&post, &post_rt, &["123", "x"]);

    // pre_traverse: result depends on the leading position.
    let pre = compile_parser!(pre_traverse(integer_min(1), |mut tokens, ctx, cursor| {
        tokens.push(Value::Int(Integer::from(cursor.byte_offset as i64)));
        Ok((tokens, ctx))
    }));
    let pre_rt = pre_traverse(integer_min(1), |mut tokens, ctx, cursor| {
        tokens.push(Value::Int(Integer::from(cursor.byte_offset as i64)));
        Ok((tokens, ctx))
    });
    assert_parity(&pre, &pre_rt, &["123", "x"]);

    // A callback that fails must error identically.
    let failing = compile_parser!(post_traverse(integer_min(1), |_t, _c, _cur| Err(
        "nope".to_string()
    )));
    let failing_rt = post_traverse(integer_min(1), |_t, _c, _cur| Err("nope".to_string()));
    assert_parity(&failing, &failing_rt, &["1", "x"]);
}

// ---------------------------------------------------------------------------
// leaf-producer codegen parity: bytes, integer_range
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_bytes_matches_runtime() {
    use nimble_parsec_rs::{bytes, compile_parser};

    let compiled = compile_parser!(bytes(3));
    assert_specialized(&compiled);
    // Successes, a too-short input, and multi-byte inputs landing on a boundary.
    assert_parity(&compiled, &bytes(3), &["abcd", "ab", "", "héllo", "aé"]);

    // bytes(2) on "aé" lands mid-codepoint (a=1 byte, é=2 bytes), so `str::get`
    // returns None and both must fail identically.
    let mid = compile_parser!(bytes(2));
    assert_specialized(&mid);
    assert_parity(&mid, &bytes(2), &["aé", "ab", "a", ""]);
}

#[test]
fn compile_parser_integer_range_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, integer_range};

    let bounded = compile_parser!(integer_range(2, Some(4)));
    assert_specialized(&bounded);
    assert_parity(
        &bounded,
        &integer_range(2, Some(4)),
        &["1", "12", "1234", "123456", "x", ""],
    );

    // The `None` (unbounded-max) form must specialize too.
    let unbounded = compile_parser!(integer_range(1, None));
    assert_specialized(&unbounded);
    assert_parity(
        &unbounded,
        &integer_range(1, None),
        &["7", "12345", "x", ""],
    );

    // Degenerate min > max: the max cap fires immediately, so the min check
    // always fails. Both sides must agree on that.
    let impossible = compile_parser!(integer_range(4, Some(2)));
    assert_specialized(&impossible);
    assert_parity(&impossible, &integer_range(4, Some(2)), &["1234", "12", ""]);
}

// ---------------------------------------------------------------------------
// string-run codegen parity: ascii_string, utf8_string
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_ascii_string_matches_runtime() {
    use nimble_parsec_rs::{ascii_string, compile_parser, AsciiPredicate};

    let lower = compile_parser!(ascii_string(
        vec![AsciiPredicate::Range(b'a'..=b'z')],
        1,
        Some(3)
    ));
    assert_specialized(&lower);
    assert_parity(
        &lower,
        &ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 1, Some(3)),
        // run of 0 (min not met), exactly min, over max (capped), non-ASCII stop.
        &["", "abc", "abcdef", "ABC", "abé"],
    );

    // Empty predicate set accepts any ASCII byte and must still specialize.
    let any = compile_parser!(ascii_string(vec![], 0, None));
    assert_specialized(&any);
    assert_parity(&any, &ascii_string(vec![], 0, None), &["abc123", "", "é"]);

    // Degenerate max=Some(0) with min=0: matches nothing, emitting an empty
    // Value::Str and consuming nothing.
    let zero = compile_parser!(ascii_string(
        vec![AsciiPredicate::Range(b'a'..=b'z')],
        0,
        Some(0)
    ));
    assert_specialized(&zero);
    assert_parity(
        &zero,
        &ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 0, Some(0)),
        &["abc", "", "1"],
    );
}

#[test]
fn compile_parser_utf8_string_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, utf8_string, Utf8Predicate};

    let lower = compile_parser!(utf8_string(
        vec![Utf8Predicate::Range('a'..='z')],
        2,
        Some(4)
    ));
    assert_specialized(&lower);
    assert_parity(
        &lower,
        &utf8_string(vec![Utf8Predicate::Range('a'..='z')], 2, Some(4)),
        // below min, exactly min, capped at max, multi-byte codepoint stop.
        &["a", "ab", "abcdef", "abé", ""],
    );

    // Empty predicate set accepts any codepoint and must still specialize.
    let any = compile_parser!(utf8_string(vec![], 0, None));
    assert_specialized(&any);
    assert_parity(&any, &utf8_string(vec![], 0, None), &["héllo", "", "abc"]);

    // Degenerate max=Some(0) with min=0: matches nothing, emitting an empty
    // Value::Str and consuming nothing.
    let zero = compile_parser!(utf8_string(
        vec![Utf8Predicate::Range('a'..='z')],
        0,
        Some(0)
    ));
    assert_specialized(&zero);
    assert_parity(
        &zero,
        &utf8_string(vec![Utf8Predicate::Range('a'..='z')], 0, Some(0)),
        &["abc", "", "1"],
    );
}

// ---------------------------------------------------------------------------
// error / zero-width / passthrough codegen parity:
// label, lookahead, lookahead_not, debug
// ---------------------------------------------------------------------------

#[test]
fn compile_parser_label_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, label};

    let compiled = compile_parser!(label(string("foo"), "a greeting"));
    assert_specialized(&compiled);
    // "foo" passes through; "bar"/"" fail with the rewritten "expected a greeting".
    assert_parity(
        &compiled,
        &label(string("foo"), "a greeting"),
        &["foo", "bar", ""],
    );

    // A multi-token inner: on success every inner token must reach the outer
    // accumulator; on failure the reason is still rewritten.
    let multi = compile_parser!(label(concat(string("a"), string("b")), "a then b"));
    assert_specialized(&multi);
    assert_parity(
        &multi,
        &label(concat(string("a"), string("b")), "a then b"),
        &["ab", "ax", "b", ""],
    );

    // ignore(label(..)): tokens suppressed, input still consumed on success.
    let ignored = compile_parser!(concat(ignore(label(string("x"), "ex")), string("y")));
    assert_specialized(&ignored);
    assert_parity(
        &ignored,
        &concat(ignore(label(string("x"), "ex")), string("y")),
        &["xy", "y", "xz"],
    );
}

#[test]
fn compile_parser_lookahead_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, lookahead};

    // Positive zero-width assertion: peek "ab", then actually consume "a".
    let compiled = compile_parser!(concat(lookahead(string("ab")), string("a")));
    assert_specialized(&compiled);
    let runtime = concat(lookahead(string("ab")), string("a"));
    // match (peek ok, consume "a"); peek fails; nothing to peek.
    assert_parity(&compiled, &runtime, &["abc", "axc", "a", ""]);
}

#[test]
fn compile_parser_lookahead_not_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, lookahead_not};

    // Negative zero-width assertion: succeed only when "x" is NOT ahead.
    let compiled = compile_parser!(concat(lookahead_not(string("x")), string("a")));
    assert_specialized(&compiled);
    let runtime = concat(lookahead_not(string("x")), string("a"));
    // not-"x" then consume "a"; "x" ahead -> fail; below.
    assert_parity(&compiled, &runtime, &["abc", "xab", "a", ""]);
}

#[test]
fn compile_parser_debug_matches_runtime() {
    use nimble_parsec_rs::{compile_parser, debug};

    // debug passes results through unchanged; stderr output is not asserted.
    let compiled = compile_parser!(debug(integer_min(1)));
    assert_specialized(&compiled);
    assert_parity(&compiled, &debug(integer_min(1)), &["42x", "x", ""]);
}

// ---------------------------------------------------------------------------
// ignore suppression for the newly-specialized leaf combinators
// ---------------------------------------------------------------------------

// The `ignored = true` codegen branch for bytes/integer_range/ascii_string/
// utf8_string is a distinct code path (it omits the token push). Exercise each
// one through `ignore(..)` and rely on assert_parity's token comparison to
// catch any over- or under-emission.
#[test]
fn compile_parser_ignore_suppresses_new_combinators() {
    use nimble_parsec_rs::{
        ascii_string, bytes, compile_parser, integer_range, utf8_string, AsciiPredicate,
        Utf8Predicate,
    };

    let p = compile_parser!(concat(ignore(bytes(2)), string("c")));
    assert_specialized(&p);
    assert_parity(
        &p,
        &concat(ignore(bytes(2)), string("c")),
        &["abc", "ac", "ab"],
    );

    let p = compile_parser!(concat(ignore(integer_range(1, Some(3))), string("x")));
    assert_specialized(&p);
    assert_parity(
        &p,
        &concat(ignore(integer_range(1, Some(3))), string("x")),
        &["12x", "x", "1234x"],
    );

    let p = compile_parser!(concat(
        ignore(ascii_string(
            vec![AsciiPredicate::Range(b'a'..=b'z')],
            1,
            None
        )),
        string("!")
    ));
    assert_specialized(&p);
    assert_parity(
        &p,
        &concat(
            ignore(ascii_string(
                vec![AsciiPredicate::Range(b'a'..=b'z')],
                1,
                None,
            )),
            string("!"),
        ),
        &["abc!", "!", "1!"],
    );

    let p = compile_parser!(concat(
        ignore(utf8_string(vec![Utf8Predicate::Range('a'..='z')], 1, None)),
        string("!")
    ));
    assert_specialized(&p);
    assert_parity(
        &p,
        &concat(
            ignore(utf8_string(vec![Utf8Predicate::Range('a'..='z')], 1, None)),
            string("!"),
        ),
        &["abc!", "!", "1!"],
    );
}
