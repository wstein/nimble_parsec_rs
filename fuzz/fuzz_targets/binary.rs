//! Fuzz target: binary-stream parsers on arbitrary `&[u8]` input.
//!
//! Exercises `byte`, `byte_range`, `be_u*`, `le_u*`, `take`, `rest`,
//! `take_while`, `utf8_char`, `length_take`, `eventually` on every
//! byte sequence the fuzzer can generate. The parsers must never panic,
//! leak memory, or produce undefined behaviour — regardless of input.
//!
//! # Running
//!
//! ```sh
//! cargo fuzz run binary          # continuous fuzzing
//! cargo fuzz run binary -- -max_len=4096
//! cargo fuzz run binary corpus/binary  # seed from corpus directory
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use nimble_parsec_rs::typed::{
    be_u16, be_u32, be_u64, be_u8, byte, byte_range, choice, eventually, le_u16, le_u32, length_take, rest,
    satisfy, take, take_while, take_while1, utf8_char, Parser,
};

fuzz_target!(|data: &[u8]| {
    // ── Leaf parsers ──────────────────────────────────────────────────────────

    let _ = byte::<&[u8]>(0x00).parse_partial(data);
    let _ = byte::<&[u8]>(0xFF).parse_partial(data);
    let _ = byte_range::<&[u8]>(0x20, 0x7E).parse_partial(data); // printable ASCII
    let _ = byte_range::<&[u8]>(0x41, 0x5A).parse_partial(data); // A-Z
    let _ = satisfy::<&[u8], _>("high", |b: u8| b >= 0x80).parse_partial(data);

    // ── Numeric parsers ───────────────────────────────────────────────────────

    let _ = be_u8::<&[u8]>().parse_partial(data);
    let _ = be_u16::<&[u8]>().parse_partial(data);
    let _ = be_u32::<&[u8]>().parse_partial(data);
    let _ = be_u64::<&[u8]>().parse_partial(data);
    let _ = le_u16::<&[u8]>().parse_partial(data);
    let _ = le_u32::<&[u8]>().parse_partial(data);

    // ── Slice parsers ─────────────────────────────────────────────────────────

    // `take` with various counts.
    let _ = take::<&[u8]>(0).parse_partial(data);
    let _ = take::<&[u8]>(1).parse_partial(data);
    if !data.is_empty() {
        let n = (data[0] as usize).min(data.len().saturating_sub(1));
        let _ = take::<&[u8]>(n).parse_partial(data);
    }
    let _ = rest::<&[u8]>().parse(data);

    // ── Repetition parsers ────────────────────────────────────────────────────

    let _ = take_while::<&[u8], _>(|b: u8| b.is_ascii_alphabetic()).parse_partial(data);
    let _ = take_while::<&[u8], _>(|b: u8| b < 0x80).parse_partial(data);
    let _ = take_while1::<&[u8], _>(|b: u8| b.is_ascii_digit()).parse_partial(data);

    // ── UTF-8 decoder from bytes ──────────────────────────────────────────────

    let _ = utf8_char::<&[u8]>().parse_partial(data);
    let _ = utf8_char::<&[u8]>().repeated().parse(data);

    // ── Structured parsers ────────────────────────────────────────────────────

    // TLV: tag byte + length byte + `length` bytes payload.
    let _ = be_u8::<&[u8]>()
        .then(length_take(be_u8::<&[u8]>().map(|n: u8| n as usize)))
        .parse_partial(data);

    // Length-prefixed: 2-byte big-endian length, then payload.
    let _ = length_take(be_u16::<&[u8]>().map(|n| n as usize)).parse_partial(data);

    // ── Alternation ───────────────────────────────────────────────────────────

    let _ = choice([
        byte::<&[u8]>(0x01),
        byte(0x02),
        byte(0x03),
    ])
    .parse_partial(data);

    // ── Eventually ────────────────────────────────────────────────────────────

    let _ = eventually(byte::<&[u8]>(0xFF)).parse_partial(data);
    let _ = eventually(be_u16::<&[u8]>().map(|n| n == 0xDEAD)).parse_partial(data);

    // ── Sequencing ────────────────────────────────────────────────────────────

    let _ = be_u8::<&[u8]>()
        .then(be_u8())
        .then(be_u16())
        .parse_partial(data);
});
