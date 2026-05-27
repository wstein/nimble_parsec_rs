//! End-to-end tests for `&[u8]` binary-stream parsing — proof of the
//! `Stream<Token=u8>` path.
//!
//! Grammar patterns are drawn from:
//! - **NimbleParsec** — `bytes`, `utf8_char`, type-tag dispatch over binaries
//! - **nom** — TLV records, magic bytes, null-terminated strings, fixed structs,
//!   DNS-label names, network-packet fields
//! - **winnow** — streaming partial input, length-prefixed data, byte ranges
//!
//! Every test uses `S = &[u8]` (or `Partial<&[u8]>` for streaming) unless
//! otherwise noted.

use nimble_parsec_rs::typed::{
    any, be_f64, be_i16, be_u16, be_u32, be_u64, byte, byte_range, choice, eventually, le_u16,
    le_u32, le_u64, length_take, literal, not, recursive, rest, satisfy, take, take_while,
    take_while1, utf8_char, Parser,
};
use nimble_parsec_rs::{Needed, ParseError, Partial};

// ── 1. Basic byte parsers ─────────────────────────────────────────────────────

#[test]
fn byte_matches_exact_value() {
    assert_eq!(byte(0x41_u8).parse([0x41_u8].as_ref()).unwrap(), 0x41);
    assert!(byte(0x41_u8).parse([0x42_u8].as_ref()).is_err());
}

#[test]
fn byte_rejects_empty_input() {
    let err = byte(0xFF_u8).parse([].as_ref()).unwrap_err();
    assert_eq!(err.reason, "expected byte 0xff");
}

#[test]
fn byte_range_accepts_bytes_in_range() {
    // ASCII digit: 0x30..=0x39
    let digit = byte_range::<&[u8]>(b'0', b'9');
    assert_eq!(digit.parse([b'5'].as_ref()).unwrap(), b'5');
    assert!(digit.parse([b'a'].as_ref()).is_err());
    assert_eq!(digit.parse([b'0'].as_ref()).unwrap(), b'0');
    assert_eq!(digit.parse([b'9'].as_ref()).unwrap(), b'9');
}

#[test]
fn byte_range_error_message_shows_bounds() {
    let err = byte_range::<&[u8]>(b'A', b'Z')
        .parse([b'a'].as_ref())
        .unwrap_err();
    assert_eq!(err.reason, "expected byte in 0x41..=0x5a");
}

#[test]
fn any_on_byte_stream_yields_each_byte() {
    let p = any::<&[u8]>().repeated();
    let data: &[u8] = &[0x01, 0x02, 0xFF];
    assert_eq!(p.parse(data).unwrap(), vec![0x01, 0x02, 0xFF]);
}

#[test]
fn satisfy_on_bytes_matches_predicate() {
    let hi_bit = satisfy::<&[u8], _>("high-bit byte", |b: u8| b >= 0x80);
    assert_eq!(hi_bit.parse([0xC0_u8].as_ref()).unwrap(), 0xC0);
    assert!(hi_bit.parse([0x7F_u8].as_ref()).is_err());
}

#[test]
fn take_while_on_bytes_collects_a_run() {
    let body = take_while::<&[u8], _>(|b: u8| b != 0);
    let (slice, rest_slice) = body.parse_partial([b'h', b'i', 0, b'!'].as_ref()).unwrap();
    assert_eq!(slice, b"hi");
    assert_eq!(rest_slice, &[0, b'!']);
}

#[test]
fn take_on_byte_slice_yields_subslice() {
    let p = take::<&[u8]>(4);
    let data: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF, 0xFF];
    let (chunk, remaining) = p.parse_partial(data).unwrap();
    assert_eq!(chunk, &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(remaining, &[0xFF]);
}

#[test]
fn rest_on_byte_slice_yields_all_remaining() {
    let p = byte(0x01_u8).ignore_then(rest::<&[u8]>());
    assert_eq!(p.parse([0x01, 0x02, 0x03].as_ref()).unwrap(), &[0x02, 0x03]);
}

#[test]
fn literal_on_byte_slice_matches_pattern() {
    let p = literal::<&[u8], _>(b"PNG".as_ref());
    assert_eq!(p.parse(b"PNG".as_ref()).unwrap(), b"PNG".as_ref());
    assert!(p.parse(b"GIF".as_ref()).is_err());
}

// ── 2. Big-endian and little-endian integer parsers ───────────────────────────

#[test]
fn be_integers_parse_network_byte_order() {
    assert_eq!(be_u16::<&[u8]>().parse(&[0x00, 0xFF]).unwrap(), 0x00FF_u16);
    assert_eq!(
        be_u32::<&[u8]>().parse(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap(),
        0xDEAD_BEEF_u32
    );
    assert_eq!(
        be_u64::<&[u8]>()
            .parse(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08])
            .unwrap(),
        0x0102_0304_0506_0708_u64
    );
}

#[test]
fn le_integers_parse_x86_byte_order() {
    assert_eq!(le_u16::<&[u8]>().parse(&[0xFF, 0x00]).unwrap(), 0x00FF_u16);
    assert_eq!(
        le_u32::<&[u8]>().parse(&[0xEF, 0xBE, 0xAD, 0xDE]).unwrap(),
        0xDEAD_BEEF_u32
    );
    assert_eq!(
        le_u64::<&[u8]>()
            .parse(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01])
            .unwrap(),
        0x0102_0304_0506_0708_u64
    );
}

#[test]
fn be_signed_integers_handle_negatives() {
    assert_eq!(be_i16::<&[u8]>().parse(&[0xFF, 0xFF]).unwrap(), -1_i16);
    assert_eq!(be_i16::<&[u8]>().parse(&[0x80, 0x00]).unwrap(), i16::MIN);
}

#[test]
fn any_on_single_byte_reads_one_byte() {
    // `any()` on `&[u8]` is the natural single-byte reader (Token = u8).
    assert_eq!(any::<&[u8]>().parse(&[0xAB]).unwrap(), 0xAB_u8);
}

#[test]
fn be_f64_round_trips_a_float() {
    let val: f64 = std::f64::consts::PI;
    let bytes = val.to_be_bytes();
    let parsed = be_f64::<&[u8]>().parse(bytes.as_ref()).unwrap();
    assert_eq!(parsed.to_bits(), val.to_bits());
}

#[test]
fn truncated_integer_errors_with_message() {
    let err = be_u16::<&[u8]>().parse(&[0x01]).unwrap_err();
    assert_eq!(err.reason, "expected 2 bytes for u16");
}

// ── 3. String-like patterns ───────────────────────────────────────────────────

/// A null-terminated C-style string from `&[u8]`.
fn c_string<'i>() -> impl Parser<&'i [u8], Output = &'i [u8]> {
    take_while(|b: u8| b != 0).then_ignore(byte(0))
}

#[test]
fn c_string_parsed_from_bytes() {
    assert_eq!(c_string().parse(b"hello\0".as_ref()).unwrap(), b"hello");
    assert_eq!(c_string().parse(b"\0".as_ref()).unwrap(), b"");
    assert!(c_string().parse(b"no_null".as_ref()).is_err());
}

#[test]
fn c_string_partial_parse_leaves_remainder() {
    let (s, remaining) = c_string().parse_partial(b"hi\0more".as_ref()).unwrap();
    assert_eq!(s, b"hi");
    assert_eq!(remaining, b"more");
}

/// A Pascal-style length-prefixed string: 1-byte length + that many bytes.
/// Uses `any()` to read the length byte as `u8`.
fn pascal_string<'i>() -> impl Parser<&'i [u8], Output = &'i [u8]> {
    length_take(any::<&[u8]>().map(|n: u8| n as usize))
}

#[test]
fn pascal_string_parsed_from_bytes() {
    assert_eq!(
        pascal_string().parse(b"\x05hello".as_ref()).unwrap(),
        b"hello"
    );
    assert_eq!(pascal_string().parse(b"\x00".as_ref()).unwrap(), b"");
    assert!(pascal_string().parse(b"\x0Aabc".as_ref()).is_err());
}

#[test]
fn length_take_with_be_u16_prefix() {
    let p = length_take(be_u16::<&[u8]>().map(|n| n as usize));
    let input: &[u8] = &[0x00, 0x03, b'a', b'b', b'c'];
    assert_eq!(p.parse(input).unwrap(), b"abc");
}

#[test]
fn length_take_error_message_for_truncated_payload() {
    let p = length_take(any::<&[u8]>().map(|n: u8| n as usize));
    let err = p.parse(b"\x05ab".as_ref()).unwrap_err();
    assert_eq!(err.reason, "expected 5 bytes after length prefix");
}

// ── 4. UTF-8 decoding from bytes ──────────────────────────────────────────────

#[test]
fn utf8_char_decodes_ascii() {
    assert_eq!(utf8_char::<&[u8]>().parse(b"A".as_ref()).unwrap(), 'A');
    assert_eq!(utf8_char::<&[u8]>().parse(b"z".as_ref()).unwrap(), 'z');
}

#[test]
fn utf8_char_decodes_multibyte_sequences() {
    // 'é' = 0xC3 0xA9 (2 bytes)
    assert_eq!(utf8_char::<&[u8]>().parse(&[0xC3, 0xA9]).unwrap(), 'é');
    // '€' = 0xE2 0x82 0xAC (3 bytes)
    assert_eq!(
        utf8_char::<&[u8]>().parse(&[0xE2, 0x82, 0xAC]).unwrap(),
        '€'
    );
    // '𝄞' (musical symbol G-clef) = 0xF0 0x9D 0x84 0x9E (4 bytes)
    assert_eq!(
        utf8_char::<&[u8]>()
            .parse(&[0xF0, 0x9D, 0x84, 0x9E])
            .unwrap(),
        '𝄞'
    );
}

#[test]
fn utf8_char_rejects_invalid_sequences() {
    assert!(utf8_char::<&[u8]>().parse(&[0xFF, 0x00]).is_err());
    assert!(utf8_char::<&[u8]>().parse(&[0xC3]).is_err()); // truncated
    assert!(utf8_char::<&[u8]>().parse(&[0xC3, 0xFF]).is_err()); // bad continuation
}

#[test]
fn utf8_char_repeated_decodes_a_utf8_string_from_bytes() {
    let p = utf8_char::<&[u8]>()
        .repeated()
        .map(|cs: Vec<char>| cs.into_iter().collect::<String>());
    assert_eq!(p.parse("hello 🌍".as_bytes()).unwrap(), "hello 🌍");
}

// ── 5. Magic-byte / file-header detection ────────────────────────────────────

/// PNG magic: 0x89 50 4E 47 0D 0A 1A 0A
const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[test]
fn png_signature_detected() {
    let p = literal::<&[u8], _>(PNG_MAGIC);
    assert_eq!(p.parse(PNG_MAGIC).unwrap(), PNG_MAGIC);
    assert!(p.parse(b"GIF87a\x00\x00".as_ref()).is_err());
}

/// Parse a minimal GZIP header: magic 0x1F 0x8B + method 0x08.
fn gzip_header<'i>() -> impl Parser<&'i [u8], Output = u8> {
    // Use a byte-slice literal pattern — &[u8] Compare<&[u8]>
    literal::<&[u8], _>(b"\x1f\x8b".as_ref()).ignore_then(byte(0x08))
}

#[test]
fn gzip_header_parsed() {
    assert_eq!(gzip_header().parse(&[0x1F, 0x8B, 0x08]).unwrap(), 0x08);
    assert!(gzip_header().parse(&[0x1F, 0x8B, 0x09]).is_err());
}

// ── 6. TLV (Type-Length-Value) records — nom style ───────────────────────────

#[derive(Debug, PartialEq)]
struct TlvRecord<'i> {
    tag: u8,
    value: &'i [u8],
}

fn tlv_record<'i>() -> impl Parser<&'i [u8], Output = TlvRecord<'i>> {
    // tag byte, then 1-byte length, then `length` bytes.
    any::<&[u8]>()
        .then(length_take(any::<&[u8]>().map(|n: u8| n as usize)))
        .map(|(tag, value)| TlvRecord { tag, value })
}

#[test]
fn tlv_record_parsed() {
    let data: &[u8] = &[0x01, 0x03, 0xAA, 0xBB, 0xCC];
    assert_eq!(
        tlv_record().parse(data).unwrap(),
        TlvRecord {
            tag: 0x01,
            value: &[0xAA, 0xBB, 0xCC],
        }
    );
}

#[test]
fn tlv_sequence_parsed() {
    let data: &[u8] = &[
        0x01, 0x02, 0xAA, 0xBB, // tag=1, len=2, val=[AA,BB]
        0x02, 0x01, 0xFF, // tag=2, len=1, val=[FF]
        0x03, 0x00, // tag=3, len=0, val=[]
    ];
    let records = tlv_record().repeated().parse(data).unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].tag, 0x01);
    assert_eq!(records[0].value, &[0xAA, 0xBB]);
    assert_eq!(records[1].tag, 0x02);
    assert_eq!(records[1].value, &[0xFF]);
    assert_eq!(records[2].tag, 0x03);
    assert!(records[2].value.is_empty());
}

// ── 7. DNS-style name parsing (length-prefixed labels) ────────────────────────

/// A single DNS label: non-zero length byte followed by that many bytes.
fn dns_label<'i>() -> impl Parser<&'i [u8], Output = &'i [u8]> {
    // Reject the root label (0x00 terminator) before trying to parse.
    not(byte(0_u8)).ignore_then(pascal_string())
}

/// A full DNS name: repeated labels terminated by 0x00.
fn dns_name<'i>() -> impl Parser<&'i [u8], Output = Vec<&'i [u8]>> {
    dns_label().repeated().then_ignore(byte(0_u8))
}

#[test]
fn dns_name_parsed() {
    // "www.example.com" in DNS wire format.
    let data: &[u8] = b"\x03www\x07example\x03com\x00";
    let labels = dns_name().parse(data).unwrap();
    assert_eq!(
        labels,
        vec![b"www".as_ref(), b"example".as_ref(), b"com".as_ref()]
    );
}

#[test]
fn dns_root_name_is_empty_list() {
    let labels = dns_name().parse(b"\x00".as_ref()).unwrap();
    assert!(labels.is_empty());
}

// ── 8. BSON-inspired type-tag dispatch ───────────────────────────────────────

#[derive(Debug, PartialEq)]
enum BsonValue<'i> {
    Double(f64),
    Str(&'i [u8]),
    Bool(bool),
}

fn bson_value<'i>() -> impl Parser<&'i [u8], Output = BsonValue<'i>> {
    let double_val = byte(0x01_u8).ignore_then(be_f64()).map(BsonValue::Double);
    let str_val = byte(0x02_u8)
        .ignore_then(length_take(be_u16::<&[u8]>().map(|n| n as usize)))
        .map(BsonValue::Str);
    let bool_val = byte(0x03_u8)
        .ignore_then(byte_range(0x00, 0x01))
        .map(|b| BsonValue::Bool(b != 0));
    choice((double_val, str_val, bool_val))
}

#[test]
fn bson_double_parsed() {
    let val = 1.5_f64;
    let mut data = vec![0x01_u8];
    data.extend_from_slice(&val.to_be_bytes());
    assert_eq!(
        bson_value().parse(data.as_slice()).unwrap(),
        BsonValue::Double(1.5)
    );
}

#[test]
fn bson_string_parsed() {
    let mut data: Vec<u8> = vec![0x02, 0x00, 0x05]; // tag, length u16 BE = 5
    data.extend_from_slice(b"hello");
    assert_eq!(
        bson_value().parse(data.as_slice()).unwrap(),
        BsonValue::Str(b"hello")
    );
}

#[test]
fn bson_bool_parsed() {
    assert_eq!(
        bson_value().parse(&[0x03, 0x01]).unwrap(),
        BsonValue::Bool(true)
    );
    assert_eq!(
        bson_value().parse(&[0x03, 0x00]).unwrap(),
        BsonValue::Bool(false)
    );
}

#[test]
fn bson_unknown_type_tag_fails() {
    assert!(bson_value().parse(&[0xFF, 0x01]).is_err());
}

// ── 9. Fixed-width binary struct (nom style) ──────────────────────────────────

/// A minimal Ethernet-like frame header: 6B dst, 6B src, 2B EtherType.
#[derive(Debug, PartialEq)]
struct EtherHeader<'i> {
    dst: &'i [u8],
    src: &'i [u8],
    ether_type: u16,
}

fn ether_header<'i>() -> impl Parser<&'i [u8], Output = EtherHeader<'i>> {
    take(6)
        .then(take(6))
        .then(be_u16())
        .map(|((dst, src), ether_type)| EtherHeader {
            dst,
            src,
            ether_type,
        })
}

#[test]
fn ethernet_header_parsed() {
    let data: &[u8] = &[
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // dst (broadcast)
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, // src
        0x08, 0x00, // EtherType = IPv4
    ];
    let hdr = ether_header().parse(data).unwrap();
    assert_eq!(hdr.dst, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(hdr.src, &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
    assert_eq!(hdr.ether_type, 0x0800);
}

// ── 10. Netstring protocol ────────────────────────────────────────────────────

/// A netstring: ASCII decimal digit count, colon, that many bytes.
fn netstring<'i>() -> impl Parser<&'i [u8], Output = &'i [u8]> {
    take_while1::<&[u8], _>(|b: u8| b.is_ascii_digit())
        .map(|digits: &[u8]| {
            digits
                .iter()
                .fold(0usize, |acc, &b| acc * 10 + (b - b'0') as usize)
        })
        .then_ignore(byte(b':'))
        .flat_map(take)
}

#[test]
fn netstring_parsed_from_bytes() {
    assert_eq!(netstring().parse(b"5:hello".as_ref()).unwrap(), b"hello");
    assert_eq!(netstring().parse(b"0:".as_ref()).unwrap(), b"");
    assert!(netstring().parse(b"5:hi".as_ref()).is_err());
}

#[test]
fn netstring_multi_digit_count() {
    let mut ns = b"12:".to_vec();
    ns.extend_from_slice(b"0123456789AB");
    assert_eq!(
        netstring().parse(ns.as_slice()).unwrap(),
        b"0123456789AB".as_ref()
    );
}

// ── 11. `eventually` on byte stream ──────────────────────────────────────────

#[test]
fn eventually_finds_magic_in_byte_stream() {
    // Find the JPEG Start of Image (SOI) marker 0xFF 0xD8.
    let soi: &[u8] = &[0xFF, 0xD8];
    let find_soi = eventually(literal::<&[u8], _>(soi));
    let data: &[u8] = &[0x00, 0x00, 0x00, 0xFF, 0xD8, 0x01, 0x02];
    let (found, remaining) = find_soi.parse_partial(data).unwrap();
    assert_eq!(found, soi);
    assert_eq!(remaining, &[0x01, 0x02]);
}

// ── 12. Streaming / Partial<&[u8]> ────────────────────────────────────────────

#[test]
fn be_u32_on_partial_stream_returns_incomplete_on_short_input() {
    // The winnow / nom streaming pattern: too few bytes → Incomplete, not error.
    let result = be_u32::<Partial<&[u8]>>().parse_partial(Partial(&[0x01, 0x02]));
    match result {
        Err(ParseError::Incomplete(Needed::Unknown)) => {}
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn be_u32_on_partial_stream_succeeds_with_enough_bytes() {
    let (val, remaining) = be_u32::<Partial<&[u8]>>()
        .parse_partial(Partial(&[0xDE, 0xAD, 0xBE, 0xEF, 0xFF]))
        .unwrap();
    assert_eq!(val, 0xDEAD_BEEF);
    assert_eq!(remaining.0, &[0xFF]);
}

#[test]
fn pascal_string_on_partial_stream_returns_incomplete_for_truncated_payload() {
    // The length byte says 5 but only 3 payload bytes are available.
    let p = length_take(any::<Partial<&[u8]>>().map(|n: u8| n as usize));
    let result = p.parse_partial(Partial(b"\x05abc"));
    match result {
        Err(ParseError::Incomplete(_)) => {}
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn take_on_partial_stream_returns_incomplete_when_not_enough() {
    let result = take::<Partial<&[u8]>>(8).parse_partial(Partial(&[0x01, 0x02, 0x03]));
    match result {
        Err(ParseError::Incomplete(_)) => {}
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn sequence_on_partial_propagates_incomplete() {
    // tag byte + 4-byte value: value is truncated → Incomplete propagates.
    let p = byte::<Partial<&[u8]>>(0x42).ignore_then(be_u32());
    let result = p.parse_partial(Partial(&[0x42, 0x00, 0x00])); // only 3 of 4 bytes
    match result {
        Err(ParseError::Incomplete(_)) => {}
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

// ── 13. `choice` over byte values ────────────────────────────────────────────

#[test]
fn choice_over_byte_alternatives_type_tag_dispatch() {
    #[derive(Debug, PartialEq)]
    enum Msg<'i> {
        Ping,
        Data(&'i [u8]),
        Error(u16),
    }

    let ping_msg = byte::<&[u8]>(0x01).map(|_| Msg::Ping);
    let data_msg = byte(0x02)
        .ignore_then(length_take(be_u16::<&[u8]>().map(|n| n as usize)))
        .map(Msg::Data);
    let err_msg = byte(0x03).ignore_then(be_u16()).map(Msg::Error);
    let msg = choice((ping_msg, data_msg, err_msg));

    assert_eq!(msg.parse(&[0x01]).unwrap(), Msg::Ping);
    assert_eq!(
        msg.parse(&[0x02, 0x00, 0x03, b'a', b'b', b'c']).unwrap(),
        Msg::Data(b"abc")
    );
    assert_eq!(msg.parse(&[0x03, 0x00, 0x2A]).unwrap(), Msg::Error(42));
    assert!(msg.parse(&[0xFF]).is_err());
}

// ── 14. `recursive` on byte stream ────────────────────────────────────────────

#[test]
fn recursive_grammar_on_byte_stream_counts_leaves() {
    // A simple nested-list format:
    //   node = 0x00 (leaf) | 0x01 <left:node> <right:node> (branch)
    // The parser counts the number of leaves.
    fn leaf_count<'i>() -> impl Parser<&'i [u8], Output = u32> {
        recursive(|node| {
            byte::<&[u8]>(0x00)
                .map(|_| 1u32) // leaf contributes 1
                .or(byte(0x01)
                    .ignore_then(node.clone())
                    .then(node)
                    .map(|(l, r)| l + r)) // branch: sum children
        })
    }

    assert_eq!(leaf_count().parse(&[0x00]).unwrap(), 1); // single leaf
    assert_eq!(leaf_count().parse(&[0x01, 0x00, 0x00]).unwrap(), 2); // 2 leaves
    assert_eq!(
        leaf_count().parse(&[0x01, 0x01, 0x00, 0x00, 0x00]).unwrap(),
        3
    ); // branch(branch(L,L), L)
}

// ── 15. Error messages for binary parsers ─────────────────────────────────────

#[test]
fn bytes_too_short_error_on_byte_slice() {
    let err = take::<&[u8]>(10).parse(&[0x01, 0x02]).unwrap_err();
    assert_eq!(err.reason, "expected 10 bytes");
}

#[test]
fn byte_mismatch_error_shows_hex() {
    let err = byte::<&[u8]>(0xAA).parse(&[0xBB]).unwrap_err();
    assert_eq!(err.reason, "expected byte 0xaa, found 0xbb");
}

#[test]
fn byte_range_out_of_range_error() {
    let err = byte_range::<&[u8]>(0x41, 0x5A).parse(&[0x61]).unwrap_err();
    assert_eq!(err.reason, "expected byte in 0x41..=0x5a");
}

// ── 16. Lookahead / not on byte streams ──────────────────────────────────────

#[test]
fn not_rejects_specific_byte() {
    let p = not(byte::<&[u8]>(0xFF)).ignore_then(any()).repeated();
    assert_eq!(
        p.parse(&[0x01, 0x02, 0x03]).unwrap(),
        vec![0x01, 0x02, 0x03]
    );
    // Stops at 0xFF.
    let (items, remaining) = p.parse_partial(&[0x01, 0xFF, 0x03]).unwrap();
    assert_eq!(items, vec![0x01]);
    assert_eq!(remaining, &[0xFF, 0x03]);
}

// ── 17. `take_while` byte predicates — winnow style ──────────────────────────

#[test]
fn take_while_collects_ascii_letters() {
    let p = take_while::<&[u8], _>(|b: u8| b.is_ascii_alphabetic());
    let (matched, remaining) = p.parse_partial(b"hello123".as_ref()).unwrap();
    assert_eq!(matched, b"hello");
    assert_eq!(remaining, b"123");
}

#[test]
fn take_while1_fails_on_no_match() {
    let p = take_while1::<&[u8], _>(|b: u8| b.is_ascii_alphabetic());
    assert!(p.parse(b"123".as_ref()).is_err());
    assert_eq!(p.parse(b"abc".as_ref()).unwrap(), b"abc");
}

// ── 18. NimbleParsec parity — bytes on &[u8] behaves like NimbleParsec bytes ─

#[test]
fn bytes_alias_works_identically_to_take_on_byte_slice() {
    // NimbleParsec's `bytes(n)` returns n raw bytes. On &[u8] both `bytes` and
    // `take` do exactly that — no UTF-8 boundary check applies.
    use nimble_parsec_rs::typed::bytes;
    let data: &[u8] = &[0x01, 0x02, 0x03, 0x04, 0x05];
    assert_eq!(
        bytes::<&[u8]>(3).parse_partial(data).unwrap().0,
        &[0x01, 0x02, 0x03]
    );
    assert_eq!(
        take::<&[u8]>(3).parse_partial(data).unwrap().0,
        &[0x01, 0x02, 0x03]
    );
}

#[test]
fn bytes_on_byte_slice_never_has_utf8_boundary_errors() {
    use nimble_parsec_rs::typed::bytes;
    // Any split of &[u8] is valid — is_valid_split always returns true for bytes.
    let data: &[u8] = &[0xC3, 0xA9, 0xFF, 0xFE]; // invalid UTF-8 if interpreted as text
    assert_eq!(bytes::<&[u8]>(1).parse_partial(data).unwrap().0, &[0xC3]);
    assert_eq!(
        bytes::<&[u8]>(2).parse_partial(data).unwrap().0,
        &[0xC3, 0xA9]
    );
    assert_eq!(
        bytes::<&[u8]>(3).parse_partial(data).unwrap().0,
        &[0xC3, 0xA9, 0xFF]
    );
}
