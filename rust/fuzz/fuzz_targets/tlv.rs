//! Fuzz target: TLV (Type-Length-Value) protocol parser.
//!
//! A more structured fuzz target: parses a sequence of TLV records from
//! arbitrary binary data. Tests the interaction of `any`, `be_u16`,
//! `length_take`, and `.repeated()` under adversarial input — specifically:
//!
//! - lengths that exceed remaining input
//! - zero-length records
//! - truncated records
//! - all-zero or all-0xFF inputs
//!
//! # Running
//!
//! ```sh
//! cargo +nightly fuzz run tlv
//! cargo +nightly fuzz run tlv corpus/tlv  # seed with hand-crafted corpus
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use nimble_parsec_rs::typed::{any, be_u16, length_take, rest, Parser};

fuzz_target!(|data: &[u8]| {
    // ── TLV grammar ───────────────────────────────────────────────────────────
    //
    // record = tag:u8  length:u16_be  value:[u8; length]
    // message = record*
    //
    // The parser must terminate (no infinite loops) and never panic.

    let record = any::<&[u8]>()
        .then(length_take(be_u16::<&[u8]>().map(|n| n as usize)))
        .map(|(tag, value): (u8, &[u8])| (tag, value.len()));

    // Parse as many records as possible, then capture the remainder.
    let message = record.repeated().then(rest::<&[u8]>());

    // Either succeeds with some records + leftover, or fails cleanly.
    let _ = message.parse_partial(data);

    // ── Netstring variant ─────────────────────────────────────────────────────
    //
    // netstring = length:u8  payload:[u8; length]
    //
    // Repeated netstrings with u8 length prefix (any() reads the single byte).

    let netstring = length_take(any::<&[u8]>().map(|n: u8| n as usize));
    let _ = netstring.repeated().parse(data);
});
