//! Working with non-`&str` / binary-ish data on a UTF-8-only parser: the
//! practical patterns, and where they run out.
//!
//! The crate parses `&str`, so the strategies are (1) transcode foreign
//! encodings to UTF-8 up front, (2) carry binary as text (hex/base64) and decode
//! in a `.map`/`.try_map`, and (3) use `bytes(n)` for fixed-width fields. A
//! *dynamic* length prefix (read N, then read N more) is expressible directly
//! with `.flat_map` — shown last.

use nimble_parsec_rs::typed::{bytes, integer, literal, take_while1, Parser};

/// Workaround 1 — transcode a non-UTF-8 encoding to UTF-8, then parse normally.
/// Latin-1 maps each byte `0x00..=0xFF` directly to the same Unicode scalar.
#[test]
fn transcode_latin1_then_parse() {
    // "café" in Latin-1: the 'é' is the single byte 0xE9 (not valid UTF-8 alone).
    let latin1: &[u8] = &[b'c', b'a', b'f', 0xE9];
    let utf8: String = latin1.iter().map(|&b| b as char).collect();

    let word = take_while1(|c: char| c.is_alphabetic());
    assert_eq!(word.parse(utf8.as_str()).unwrap(), "café");
}

/// Workaround 2 — carry binary as hex text and decode it inside the grammar with
/// `.try_map` (base64 is the same shape with a base64 crate).
#[test]
fn decode_hex_encoded_binary_field() {
    let hex_blob = take_while1(|c: char| c.is_ascii_hexdigit()).try_map(|h: &str| {
        if h.len() % 2 != 0 {
            return Err("odd-length hex blob".to_string());
        }
        let raw = h.as_bytes();
        let mut bytes = Vec::with_capacity(raw.len() / 2);
        let mut i = 0;
        while i < raw.len() {
            let hi = (raw[i] as char).to_digit(16).unwrap() as u8;
            let lo = (raw[i + 1] as char).to_digit(16).unwrap() as u8;
            bytes.push(hi * 16 + lo);
            i += 2;
        }
        Ok(bytes)
    });

    let field = literal("blob:").ignore_then(hex_blob);
    assert_eq!(field.parse("blob:48656c6c6f").unwrap(), b"Hello".to_vec());
    assert!(field.parse("blob:abc").is_err()); // odd length → rejected
}

/// Workaround 3 — fixed-width fields, where the byte counts are known at
/// build time, map directly onto `bytes(n)`.
#[test]
fn fixed_width_record_with_bytes() {
    // A 2-char type code followed by a 3-char id.
    let record = bytes(2).then(bytes(3));
    assert_eq!(record.parse("AB123").unwrap(), ("AB", "123"));

    // Reminder: `bytes(n)` counts *bytes* and must land on a char boundary —
    // fine for ASCII fields, but a multi-byte field needs a boundary-aware count.
    assert!(bytes(1).parse("é").is_err()); // 'é' is 2 bytes; 1 splits it
    assert_eq!(bytes(2).parse("é").unwrap(), "é");
}

/// A *dynamic* length prefix ("take the next N bytes, where N was just parsed")
/// uses the parsed length to choose the next parser — `.flat_map` (monadic bind).
/// This is a netstring: a byte count, ':', then exactly that many bytes.
#[test]
fn dynamic_length_prefix_with_flat_map() {
    let netstring = integer()
        .then_ignore(literal(":"))
        .flat_map(|n: i64| bytes(n as usize));

    assert_eq!(netstring.parse("5:hello").unwrap(), "hello");
    assert!(netstring.parse("5:hi").is_err()); // not enough bytes for the count

    // The count is in *bytes* (correct for netstrings): "2:é" is the 2-byte 'é'.
    assert_eq!(netstring.parse("2:é").unwrap(), "é");
}
