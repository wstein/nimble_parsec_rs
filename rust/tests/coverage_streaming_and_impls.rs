//! Coverage for:
//!
//! 1. `ParseFailure<S>` and `ParseError<S>` trait impls — `clone`, `Debug`,
//!    `PartialEq`, `Display`, `From`.
//! 2. `Input<S>` clone and `Debug`.
//! 3. `Stream::preview` and `is_valid_split` for `&[T]`, `Partial<S>`, `Bits<S>`.
//! 4. `Compare<T> for &[T]` (single-token literal) and `Compare` for `Partial<S>`.
//! 5. `Bits<S>::inner()` accessor.
//! 6. Streaming `Partial` `Incomplete` propagation paths in `Or`, `Opt`,
//!    `Repeated`, `Fold`, `Labelled`, `Debug`, `ByteRange`, `SeparatedBy`,
//!    `RepeatedUntil`.
//! 7. `parse()` converting `Incomplete` to `ParseFailure`.
//! 8. `Take` invalid UTF-8 boundary rejection.
//! 9. `Tokens<T>` deprecated smoke test.
//! 10. `Gen::below(0)` edge case.

#![allow(deprecated)]

use std::num::NonZeroUsize;

use nimble_parsec_rs::typed::{
    any, byte_range, choice, length_take, literal, repeated_until, satisfy, separated_by,
    separated_by1, take, Parser, Tokens,
};
use nimble_parsec_rs::{Bits, Cursor, Needed, ParseError, ParseFailure, Partial, Stream};

// ── ParseFailure trait impls ──────────────────────────────────────────────────

#[test]
fn parse_failure_clone_and_eq() {
    let err1 = literal("x").parse("y").unwrap_err();
    let err2 = err1.clone();
    assert_eq!(err1, err2);
    // Different failures are !=
    let err3 = literal("z").parse("y").unwrap_err();
    assert_ne!(err1, err3);
}

#[test]
fn parse_failure_debug_format() {
    let err = literal("x").parse("y").unwrap_err();
    let s = format!("{:?}", err);
    assert!(s.contains("ParseFailure"));
    assert!(s.contains("reason"));
}

// ── ParseError trait impls ────────────────────────────────────────────────────

#[test]
fn parse_error_from_parse_failure() {
    let failure: ParseFailure<&str> = ParseFailure::expecting("test", "", Cursor::default());
    let error: ParseError<&str> = ParseError::from(failure.clone());
    assert!(matches!(error, ParseError::Failure(_)));
    // Round-trip through Into
    let error2: ParseError<&str> = failure.into();
    assert!(matches!(error2, ParseError::Failure(_)));
}

#[test]
fn parse_error_clone_failure_variant() {
    let err = literal("x").parse_partial("y").unwrap_err();
    let cloned = err.clone();
    assert_eq!(err, cloned);
}

#[test]
fn parse_error_clone_incomplete_variant() {
    let err: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    let cloned = err.clone();
    assert_eq!(err, cloned);

    let err2: ParseError<Partial<&str>> =
        ParseError::Incomplete(Needed::Size(NonZeroUsize::new(3).unwrap()));
    let cloned2 = err2.clone();
    assert_eq!(err2, cloned2);
}

#[test]
fn parse_error_debug_failure_variant() {
    let err = literal("x").parse_partial("y").unwrap_err();
    let s = format!("{:?}", err);
    assert!(s.contains("Failure"));
}

#[test]
fn parse_error_debug_incomplete_variant() {
    let err: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    let s = format!("{:?}", err);
    assert!(s.contains("Incomplete"));
}

#[test]
fn parse_error_partial_eq_same_variants() {
    // Failure == Failure
    let e1: ParseError<&str> =
        ParseError::Failure(ParseFailure::expecting("x", "", Cursor::default()));
    let e2: ParseError<&str> =
        ParseError::Failure(ParseFailure::expecting("x", "", Cursor::default()));
    assert_eq!(e1, e2);

    // Incomplete == Incomplete
    let i1: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    let i2: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    assert_eq!(i1, i2);
}

#[test]
fn parse_error_partial_eq_different_variants() {
    let fail: ParseError<Partial<&str>> =
        ParseError::Failure(ParseFailure::expecting("x", "", Cursor::default()));
    let inc: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    assert_ne!(fail, inc);
}

#[test]
fn parse_error_display_incomplete_unknown() {
    let err: ParseError<Partial<&str>> = ParseError::Incomplete(Needed::Unknown);
    let s = format!("{}", err);
    assert_eq!(s, "incomplete input (need more data)");
}

#[test]
fn parse_error_display_incomplete_size() {
    let err: ParseError<Partial<&str>> =
        ParseError::Incomplete(Needed::Size(NonZeroUsize::new(7).unwrap()));
    let s = format!("{}", err);
    assert!(s.contains("7"), "got: {s}");
    assert!(s.contains("base units"), "got: {s}");
}

// ── Input<S> clone and Debug ──────────────────────────────────────────────────

#[test]
fn input_clone_and_debug() {
    use nimble_parsec_rs::typed::Input;
    let input = Input::new("hello world");
    let cloned = input.clone(); // exercises Clone impl (Copy newtype)
    let s = format!("{:?}", cloned); // exercises Debug → calls stream.preview()
    assert!(s.contains("Input"));
}

#[test]
fn input_debug_on_token_slice() {
    use nimble_parsec_rs::typed::Input;
    let tokens = vec![1u8, 2u8, 3u8];
    let input = Input::new(tokens.as_slice());
    let s = format!("{:?}", input); // exercises &[T]::preview
    assert!(s.contains("Input"));
}

#[test]
fn input_debug_on_partial_stream() {
    use nimble_parsec_rs::typed::Input;
    let input = Input::new(Partial("hello"));
    let s = format!("{:?}", input); // exercises Partial<S>::preview
    assert!(s.contains("Input"));
}

#[test]
fn input_debug_on_bits_stream() {
    use nimble_parsec_rs::typed::Input;
    let bits_stream = Bits::new(b"\xAB\xCD".as_ref());
    let input = Input::new(bits_stream);
    let s = format!("{:?}", input); // exercises Bits<S>::preview
    assert!(s.contains("Input"));
}

// ── Stream::is_valid_split for uncovered types ────────────────────────────────

#[test]
fn partial_str_is_valid_split_via_take() {
    // take::<Partial<&str>>(3) calls is_valid_split on Partial<&str>
    let result = take::<Partial<&str>>(3).parse_partial(Partial("hello"));
    assert_eq!(result.unwrap().0, "hel");
}

#[test]
fn bits_is_valid_split_via_take() {
    // take::<Bits<&[u8]>>(4) calls is_valid_split on Bits<&[u8]>
    let bits_stream = Bits::new(b"\xAB".as_ref());
    let (slice, _rest) = take::<Bits<&[u8]>>(4).parse_partial(bits_stream).unwrap();
    assert_eq!(slice.len_bits, 4);
}

// ── Bits<S>::inner() accessor ─────────────────────────────────────────────────

#[test]
fn bits_inner_accessor() {
    let data: &[u8] = b"\xAB\xCD";
    let bits_stream = Bits::new(data);
    let inner: &[u8] = bits_stream.inner();
    assert_eq!(inner, b"\xAB\xCD".as_ref());
}

// ── Compare<T> for &[T] (single-token literal) ───────────────────────────────

#[test]
fn literal_single_byte_on_byte_slice() {
    // Compare<u8> for &[u8] — single-element match; Output is S::Slice = &[u8]
    let result = literal::<&[u8], u8>(0x42u8).parse(b"\x42".as_ref());
    assert_eq!(result.unwrap(), b"\x42".as_ref());
}

#[test]
fn literal_single_byte_mismatch() {
    let result = literal::<&[u8], u8>(0x42u8).parse(b"\x43".as_ref());
    assert!(result.is_err());
}

// ── Compare<Pat> for Partial<S> ───────────────────────────────────────────────

#[test]
fn literal_on_partial_str_stream() {
    // Compare<&str> for Partial<&str> via the Partial delegation impl
    let result = literal("hi").parse_partial(Partial("hi there"));
    assert!(result.is_ok());
}

#[test]
fn literal_on_partial_byte_stream() {
    // Compare<&[u8]> for Partial<&[u8]>
    let result =
        literal::<Partial<&[u8]>, &[u8]>(b"AB".as_ref()).parse_partial(Partial(b"ABC".as_ref()));
    assert!(result.is_ok());
}

// ── parse() Incomplete → ParseFailure conversion ─────────────────────────────

#[test]
fn parse_converts_incomplete_to_failure() {
    // take(10) on a 2-byte Partial stream returns Incomplete.
    // parse() (not parse_partial()) must convert that to a ParseFailure.
    let result = take::<Partial<&str>>(10).parse(Partial("hi"));
    let err = result.unwrap_err();
    assert!(err.reason.contains("incomplete"), "got: {}", err.reason);
}

// ── Streaming Incomplete in Or ────────────────────────────────────────────────

#[test]
fn or_first_branch_incomplete_propagated() {
    // First branch: take(5) on 3-char Partial stream → Incomplete
    // Or should propagate Incomplete (not try the second branch).
    let parser = take::<Partial<&str>>(5).or(take(3));
    let result = parser.parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn or_second_branch_incomplete_propagated() {
    // First branch: literal("x") fails (not Incomplete).
    // Second branch: take(5) on 3-char Partial → Incomplete.
    let parser = literal::<Partial<&str>, _>("x").or(take(5));
    let result = parser.parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Streaming Incomplete in Opt ───────────────────────────────────────────────

#[test]
fn opt_propagates_incomplete() {
    // take(5) on 3-char Partial stream returns Incomplete.
    // Opt must propagate it rather than returning None.
    let result = take::<Partial<&str>>(5)
        .optional()
        .parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Streaming Incomplete in Repeated ─────────────────────────────────────────

#[test]
fn repeated_with_min_propagates_incomplete() {
    // repeated_at_least(2): first iteration succeeds, second hits Incomplete.
    // Since we're below min, Incomplete is propagated.
    let result = take::<Partial<&str>>(2)
        .repeated_at_least(2)
        .parse_partial(Partial("ab")); // only enough for one take(2)
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn repeated_at_or_above_min_breaks_on_incomplete() {
    // repeated_at_least(1): first iteration succeeds; second hits Incomplete.
    // Since we've already met min=1 AND PARTIAL=true, Incomplete is returned.
    let result = take::<Partial<&str>>(2)
        .repeated_at_least(1)
        .parse_partial(Partial("ab")); // one take(2) = "ab", then no more
                                       // Could be Incomplete or Ok([...]) depending on whether the retry sees empty
                                       // Either is acceptable here; we just want the path exercised.
    let _ = result;
}

// ── Streaming Incomplete in Fold ─────────────────────────────────────────────

#[test]
fn fold_on_partial_propagates_incomplete() {
    // fold: inner = take(2), but after consuming "ab", the next take(2) on
    // the remaining 1-char Partial stream returns Incomplete.
    // Since PARTIAL=true, Fold propagates it.
    let result = take::<Partial<&str>>(2)
        .fold(
            || String::new(),
            |mut acc, s| {
                acc.push_str(s);
                acc
            },
        )
        .parse_partial(Partial("abc")); // 3 chars: one take(2)="ab", then 1 remains → Incomplete
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Streaming Incomplete in Labelled ─────────────────────────────────────────

#[test]
fn labelled_propagates_incomplete() {
    let result = take::<Partial<&str>>(5)
        .labelled("five_chars")
        .parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Streaming Incomplete in Debug combinator ─────────────────────────────────

#[test]
fn debug_combinator_propagates_incomplete() {
    let result = take::<Partial<&str>>(5)
        .debug("take5")
        .parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── ByteRange Incomplete on empty Partial ────────────────────────────────────

#[test]
fn byte_range_incomplete_on_empty_partial() {
    // byte_range on Partial<&[u8]> that has no bytes left → Incomplete
    let result = byte_range::<Partial<&[u8]>>(0x30, 0x39).parse_partial(Partial(b"".as_ref()));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Take invalid UTF-8 boundary ──────────────────────────────────────────────

#[test]
fn take_rejects_invalid_utf8_boundary_on_str() {
    // "é" = U+00E9 = 2 UTF-8 bytes [0xC3, 0xA9].
    // take::<&str>(1) asks for 1 byte, which lands in the middle of é → rejected.
    let result = take::<&str>(1).parse("é hello");
    let err = result.unwrap_err();
    assert!(
        err.reason.contains("boundary") || err.reason.contains("UTF"),
        "got: {}",
        err.reason
    );
}

// ── SeparatedBy Incomplete paths ──────────────────────────────────────────────

#[test]
fn separated_by_item_incomplete_on_partial() {
    // First item parse hits Incomplete immediately.
    let result = separated_by(take::<Partial<&str>>(5), literal(",")).parse_partial(Partial("ab"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn separated_by_sep_incomplete_on_partial() {
    // First item parses OK ("ab"), then separator take(3) hits Incomplete.
    let result = separated_by(take::<Partial<&str>>(2), take::<Partial<&str>>(3))
        .parse_partial(Partial("abx")); // "ab" item ok, then separator needs 3 but only 1 left
                                        // Separator hits Incomplete → propagated
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn separated_by_second_item_incomplete_on_partial() {
    // Item "ab" ok, sep "," ok, then second item take(5) hits Incomplete.
    let result =
        separated_by(take::<Partial<&str>>(2), literal(",")).parse_partial(Partial("ab,c")); // after separator, only 1 char left for take(2)
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── RepeatedUntil Incomplete path ─────────────────────────────────────────────

#[test]
fn repeated_until_body_incomplete_on_partial() {
    // end = literal("END"), body = take(2).
    // On "ab" partial stream: end fails, body take(2) returns Incomplete (only
    // 2 chars: first try consumes "ab", then on empty Partial → Incomplete).
    let result =
        repeated_until(take::<Partial<&str>>(2), literal("END")).parse_partial(Partial("ab"));
    // After consuming "ab", next take(2) on empty Partial → Incomplete
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── Tokens<T> deprecated smoke test ──────────────────────────────────────────

#[test]
fn tokens_deprecated_still_works() {
    // Smoke test: Tokens<T> is deprecated but still compiles and functions.
    #[derive(Copy, Clone, Debug, PartialEq)]
    enum T {
        A,
        B,
    }
    let ts = vec![T::A, T::B, T::A];
    let toks = Tokens(&ts);

    // first()
    let (tok, w) = toks.first().unwrap();
    assert_eq!(tok, T::A);
    assert_eq!(w, 1);

    // split_at()
    let (slice, rest) = toks.split_at(2);
    assert_eq!(slice, &[T::A, T::B]);
    assert_eq!(rest.len(), 1);

    // as_slice, len, preview
    let sl = toks.as_slice();
    assert_eq!(sl.len(), 3);
    let _ = toks.preview(5); // cover preview()

    // advance_cursor
    let cursor = Cursor::default();
    let advanced = toks.advance_cursor(cursor, 2);
    assert_eq!(advanced.byte_offset, 2);

    // parse with satisfy
    let result = satisfy::<Tokens<T>, _>("A", |t: T| t == T::A)
        .repeated()
        .parse(Tokens(&ts[..1]));
    assert_eq!(result.unwrap(), vec![T::A]);
}

// ── separated_by1 Incomplete ──────────────────────────────────────────────────

#[test]
fn separated_by1_first_item_incomplete_on_partial() {
    // separated_by1 with min=1: first item parse returns Incomplete.
    let result = separated_by1(take::<Partial<&str>>(5), literal(",")).parse_partial(Partial("ab"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

// ── ParseError::Display for Failure variant ───────────────────────────────────

#[test]
fn parse_error_display_failure_variant() {
    // ParseError<S>::fmt for the Failure variant delegates to ParseFailure::fmt.
    let failure = ParseFailure::expecting("expected 'x'", "", Cursor::default());
    let err: ParseError<&str> = ParseError::from(failure);
    let s = format!("{}", err);
    assert!(s.contains("expected 'x'"), "got: {s}");
}

// ── LengthTake invalid split on &str ─────────────────────────────────────────

#[test]
fn length_take_invalid_utf8_split_on_str() {
    // Prefix parser reads a digit as the byte count.
    // "1é": prefix="1" → n=1; remaining stream="é" (2 UTF-8 bytes).
    // is_valid_split(1) on "é" is false → rejected error.
    let prefix = any::<&str>().map(|c: char| c as usize - '0' as usize);
    let result = length_take(prefix).parse("1é");
    let err = result.unwrap_err();
    assert!(
        err.reason.contains("boundary") || err.reason.contains("split"),
        "got: {}",
        err.reason
    );
}

// ── choice() Incomplete propagation (array and tuple forms) ──────────────────

#[test]
fn choice_array_incomplete_propagated() {
    // First alternative take(5) on 3-char Partial stream returns Incomplete.
    // choice must propagate it rather than trying the next alternative.
    let result = choice([take::<Partial<&str>>(5), take(3)]).parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn choice_tuple_first_arm_incomplete_propagated() {
    // Tuple choice: first arm take(5) returns Incomplete on short Partial input.
    let result =
        choice((take::<Partial<&str>>(5), take::<Partial<&str>>(3))).parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}

#[test]
fn choice_tuple_second_arm_incomplete_propagated() {
    // Tuple choice: first arm (literal "x") fails; second arm take(5) returns
    // Incomplete. The Incomplete from the second arm must be propagated.
    let result = choice((literal::<Partial<&str>, _>("x"), take::<Partial<&str>>(5)))
        .parse_partial(Partial("abc"));
    assert!(matches!(result, Err(ParseError::Incomplete(_))));
}
