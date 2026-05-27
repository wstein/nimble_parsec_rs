//! Fuzz target: text-stream parsers on arbitrary valid UTF-8 input.
//!
//! The fuzzer generates arbitrary `&str` inputs; the parsers must never panic,
//! regardless of structure or content.
//!
//! # Running
//!
//! ```sh
//! cargo fuzz run str
//! cargo fuzz run str -- -max_len=4096
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use nimble_parsec_rs::typed::{
    any, bytes, choice, digits, empty, eventually, integer, literal, not, satisfy, take,
    take_while, take_while1, Parser,
};

fuzz_target!(|data: &str| {
    // ── Leaf parsers ──────────────────────────────────────────────────────────

    let _ = any::<&str>().parse_partial(data);
    let _ = literal::<&str, _>("hello").parse_partial(data);
    let _ = literal::<&str, _>("🌍").parse_partial(data);
    let _ = satisfy::<&str, _>("alpha", |c: char| c.is_alphabetic()).parse_partial(data);
    let _ = satisfy::<&str, _>("digit", |c: char| c.is_ascii_digit()).parse_partial(data);

    // ── Slice parsers ─────────────────────────────────────────────────────────

    let _ = take_while::<&str, _>(|c: char| c.is_alphabetic()).parse_partial(data);
    let _ = take_while1::<&str, _>(|c: char| c.is_ascii_digit()).parse_partial(data);
    let _ = digits::<&str>().parse_partial(data);

    // ── `take` and `bytes` on text (must land on char boundary) ───────────────

    let _ = take::<&str>(0).parse_partial(data);
    let _ = take::<&str>(1).parse_partial(data);
    let _ = bytes::<&str>(4).parse_partial(data);

    // ── Integer parsing ───────────────────────────────────────────────────────

    let _ = integer::<&str>().parse_partial(data);

    // ── Control flow ──────────────────────────────────────────────────────────

    let _ = empty::<&str>().parse_partial(data);
    let _ = any::<&str>().optional().parse_partial(data);
    let _ = any::<&str>().repeated().parse(data);
    let _ = not(literal::<&str, _>("x")).parse_partial(data);

    // ── Alternation ───────────────────────────────────────────────────────────

    let _ = choice([literal("a"), literal("b"), literal("c")]).parse_partial(data);
    let _ = literal::<&str, _>("yes")
        .or(literal("no"))
        .parse_partial(data);

    // ── Sequencing ────────────────────────────────────────────────────────────

    let _ = any::<&str>().then(any()).parse_partial(data);
    let _ = any::<&str>().then_ignore(any()).parse_partial(data);

    // ── Eventually ────────────────────────────────────────────────────────────

    let _ = eventually(literal::<&str, _>("END")).parse_partial(data);
});
