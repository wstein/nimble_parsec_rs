//! Tests for bit-level parsing via `Bits<S>`, `take_bits`, `bit_bool`,
//! `bits()`, and `byte_aligned()`.
//!
//! Bit order: MSB-first within each byte (same convention as nom's `bits`).
//! So byte `0xAB` = `1010_1011`:
//!   - bit 0 → true  (MSB)
//!   - bit 1 → false
//!   - bit 7 → true  (LSB)

use nimble_parsec_rs::typed::{bit_bool, bits, byte_aligned, take, take_bits, BitOutput, Parser};
use nimble_parsec_rs::{Bits, Partial, Stream};

// ── BitOutput trait ──────────────────────────────────────────────────────────

#[test]
fn bit_output_zero_and_one() {
    assert_eq!(<u8 as BitOutput>::zero(), 0u8);
    assert_eq!(<u8 as BitOutput>::one(), 1u8);
    assert_eq!(<u32 as BitOutput>::zero(), 0u32);
    assert_eq!(<u32 as BitOutput>::one(), 1u32);
    assert_eq!(<u64 as BitOutput>::zero(), 0u64);
    assert_eq!(<u128 as BitOutput>::zero(), 0u128);
}

// ── take_bits — single byte, aligned ────────────────────────────────────────

#[test]
fn take_bits_upper_nibble() {
    // 0xAB = 1010_1011 → upper 4 bits = 1010 = 0x0A
    let v = bits(take_bits::<u8, &[u8]>(4))
        .parse(b"\xAB".as_ref())
        .unwrap();
    assert_eq!(v, 0x0A);
}

#[test]
fn take_bits_lower_nibble() {
    // 0xAB → lower 4 bits = 0x0B
    let (hi, lo) = bits(take_bits::<u8, &[u8]>(4).then(take_bits::<u8, &[u8]>(4)))
        .parse(b"\xAB".as_ref())
        .unwrap();
    assert_eq!(hi, 0x0A);
    assert_eq!(lo, 0x0B);
}

#[test]
fn take_bits_full_byte() {
    let v = bits(take_bits::<u8, &[u8]>(8))
        .parse(b"\xFF".as_ref())
        .unwrap();
    assert_eq!(v, 0xFF);
}

#[test]
fn take_bits_zero_byte() {
    let v = bits(take_bits::<u8, &[u8]>(8))
        .parse(b"\x00".as_ref())
        .unwrap();
    assert_eq!(v, 0x00);
}

// ── take_bits — cross-byte fields ───────────────────────────────────────────

#[test]
fn take_bits_cross_byte_12_bits() {
    // [0xAB, 0xCD] = 1010_1011_1100_1101
    // First 12 bits = 1010_1011_1100 = 0xABC
    let v = bits(take_bits::<u16, &[u8]>(12))
        .parse(b"\xAB\xC0".as_ref())
        .unwrap();
    assert_eq!(v, 0xABC);
}

#[test]
fn take_bits_cross_byte_sequential() {
    // [0xAB, 0xCD]: bits 0..=3 from first byte, then bits 4..=7 from first
    // byte (nibbles), then 8 bits of second byte.
    let (hi4, lo4, second) = bits(
        take_bits::<u8, &[u8]>(4)
            .then(take_bits::<u8, &[u8]>(4))
            .then(take_bits::<u8, &[u8]>(8))
            .map(|((a, b), c)| (a, b, c)),
    )
    .parse(b"\xAB\xCD".as_ref())
    .unwrap();
    assert_eq!(hi4, 0x0A); // upper nibble of 0xAB
    assert_eq!(lo4, 0x0B); // lower nibble of 0xAB
    assert_eq!(second, 0xCD);
}

#[test]
fn take_bits_24_bit_field() {
    // 3-byte big-endian 24-bit value: [0x01, 0x23, 0x45] = 0x01_2345
    let v = bits(take_bits::<u32, &[u8]>(24))
        .parse(b"\x01\x23\x45".as_ref())
        .unwrap();
    assert_eq!(v, 0x00_01_23_45);
}

// ── bit_bool ─────────────────────────────────────────────────────────────────

#[test]
fn bit_bool_single_true() {
    // 0x80 = 1000_0000 → first bit is 1
    let v = bits(bit_bool::<&[u8]>())
        .parse_partial(b"\x80".as_ref())
        .unwrap()
        .0;
    assert!(v);
}

#[test]
fn bit_bool_single_false() {
    // 0x40 = 0100_0000 → first bit is 0
    let v = bits(bit_bool::<&[u8]>())
        .parse_partial(b"\x40".as_ref())
        .unwrap()
        .0;
    assert!(!v);
}

#[test]
fn bit_bool_all_bits_in_byte() {
    // 0x80 = 1000_0000 → [true, false, false, false, false, false, false, false]
    let bools = bits(bit_bool::<&[u8]>().repeated())
        .parse(b"\x80".as_ref())
        .unwrap();
    assert_eq!(
        bools,
        vec![true, false, false, false, false, false, false, false]
    );
}

#[test]
fn bit_bool_alternating_pattern() {
    // 0xAA = 1010_1010
    let bools = bits(bit_bool::<&[u8]>().repeated())
        .parse(b"\xAA".as_ref())
        .unwrap();
    assert_eq!(
        bools,
        vec![true, false, true, false, true, false, true, false]
    );
}

#[test]
fn bit_bool_two_bytes() {
    // 0xFF, 0x00
    let bools = bits(bit_bool::<&[u8]>().repeated())
        .parse(b"\xFF\x00".as_ref())
        .unwrap();
    assert_eq!(bools.len(), 16);
    assert!(bools[..8].iter().all(|&b| b));
    assert!(bools[8..].iter().all(|&b| !b));
}

// ── bits() combinator ────────────────────────────────────────────────────────

#[test]
fn bits_returns_to_byte_stream() {
    // After bits() consumes a full byte, the outer stream advances by 1.
    let (_, rest) = bits(take_bits::<u8, &[u8]>(8))
        .parse_partial(b"\xAB\xCD".as_ref())
        .unwrap();
    assert_eq!(rest, b"\xCD".as_ref());
}

#[test]
fn bits_partial_byte_advances_full_byte() {
    // Consuming 4 bits still advances the outer stream by 1 full byte
    // (the partial byte is consumed).
    let (_, rest) = bits(take_bits::<u8, &[u8]>(4))
        .parse_partial(b"\xAB\xCD".as_ref())
        .unwrap();
    assert_eq!(rest, b"\xCD".as_ref());
}

#[test]
fn bits_multiple_bytes() {
    // Consuming 12 bits spans 2 bytes; outer stream advances by 2.
    let (_, rest) = bits(take_bits::<u16, &[u8]>(12))
        .parse_partial(b"\xAB\xCD\xEF".as_ref())
        .unwrap();
    assert_eq!(rest, b"\xEF".as_ref());
}

#[test]
fn bits_followed_by_byte_parser() {
    // bits() reads 8 bits, then a normal `take(1)` reads the next byte.
    let (nibble_val, next_byte) = bits(take_bits::<u8, &[u8]>(8))
        .then(take::<&[u8]>(1))
        .parse(b"\xAB\xCD".as_ref())
        .unwrap();
    assert_eq!(nibble_val, 0xAB);
    assert_eq!(next_byte, b"\xCD".as_ref());
}

// ── byte_aligned() combinator ────────────────────────────────────────────────

#[test]
fn byte_aligned_already_aligned() {
    // 8 bits consumed → already on byte boundary, no padding needed.
    let v = bits(byte_aligned(take_bits::<u8, &[u8]>(8)))
        .parse(b"\xAB".as_ref())
        .unwrap();
    assert_eq!(v, 0xAB);
}

#[test]
fn byte_aligned_pads_to_next_byte() {
    // 3-bit field, then byte_aligned pads 5 bits, then read next byte.
    let (field, next) =
        bits(byte_aligned(take_bits::<u8, &[u8]>(3)).then(take_bits::<u8, &[u8]>(8)))
            .parse(b"\xE0\xFF".as_ref())
            .unwrap();
    // 0xE0 = 1110_0000 → top 3 bits = 0b111 = 7
    assert_eq!(field, 0b111);
    assert_eq!(next, 0xFF);
}

#[test]
fn byte_aligned_one_bit_pads_seven() {
    // Read 1 bit, byte_aligned pads 7, then read next byte.
    let (bit_val, next) =
        bits(byte_aligned(take_bits::<u8, &[u8]>(1)).then(take_bits::<u8, &[u8]>(8)))
            .parse(b"\x80\xAB".as_ref())
            .unwrap();
    assert_eq!(bit_val, 1u8); // MSB of 0x80
    assert_eq!(next, 0xAB);
}

// ── IPv4-style header field parsing ─────────────────────────────────────────

#[test]
fn ipv4_version_and_ihl() {
    // First byte of IPv4 header: version (4 bits) | IHL (4 bits)
    // 0x45 = 0100_0101 → version=4, IHL=5
    let (version, ihl) = bits(take_bits::<u8, &[u8]>(4).then(take_bits::<u8, &[u8]>(4)))
        .parse(b"\x45".as_ref())
        .unwrap();
    assert_eq!(version, 4);
    assert_eq!(ihl, 5);
}

#[test]
fn ipv4_dscp_and_ecn() {
    // Second byte of IPv4 header: DSCP (6 bits) | ECN (2 bits)
    // 0x28 = 0010_1000 → DSCP=10, ECN=0
    let (dscp, ecn) = bits(take_bits::<u8, &[u8]>(6).then(take_bits::<u8, &[u8]>(2)))
        .parse(b"\x28".as_ref())
        .unwrap();
    assert_eq!(dscp, 10);
    assert_eq!(ecn, 0);
}

#[test]
fn ipv4_first_two_bytes() {
    // version(4) | IHL(4) | DSCP(6) | ECN(2)
    // [0x45, 0x28]
    let (version, ihl, dscp, ecn) = bits(
        take_bits::<u8, &[u8]>(4)
            .then(take_bits::<u8, &[u8]>(4))
            .then(take_bits::<u8, &[u8]>(6))
            .then(take_bits::<u8, &[u8]>(2))
            .map(|(((v, i), d), e)| (v, i, d, e)),
    )
    .parse(b"\x45\x28".as_ref())
    .unwrap();
    assert_eq!(version, 4);
    assert_eq!(ihl, 5);
    assert_eq!(dscp, 10);
    assert_eq!(ecn, 0);
}

// ── Error cases ──────────────────────────────────────────────────────────────

#[test]
fn take_bits_not_enough_bits() {
    // 1 byte = 8 bits; asking for 9 should fail.
    let result = bits(take_bits::<u16, &[u8]>(9)).parse(b"\xAB".as_ref());
    assert!(result.is_err());
}

#[test]
fn bit_bool_on_empty_fails() {
    let result = bits(bit_bool::<&[u8]>()).parse(b"".as_ref());
    assert!(result.is_err());
}

#[test]
fn take_bits_zero_bits() {
    // Zero bits consumed → result is 0, no bytes consumed.
    let (v, rest) = bits(take_bits::<u8, &[u8]>(0))
        .parse_partial(b"\xAB".as_ref())
        .unwrap();
    assert_eq!(v, 0);
    // 0 bits → 0 bytes consumed (⌈0/8⌉ = 0)
    assert_eq!(rest, b"\xAB".as_ref());
}

// ── Repetition ───────────────────────────────────────────────────────────────

#[test]
fn take_bits_repeated_nibbles() {
    // [0xAB, 0xCD] split into four nibbles: [A, B, C, D]
    let nibbles = bits(take_bits::<u8, &[u8]>(4).repeated())
        .parse(b"\xAB\xCD".as_ref())
        .unwrap();
    assert_eq!(nibbles, vec![0x0A, 0x0B, 0x0C, 0x0D]);
}

#[test]
fn bit_bool_repeated_three_bytes() {
    let bools = bits(bit_bool::<&[u8]>().repeated())
        .parse(b"\xFF\x00\xF0".as_ref())
        .unwrap();
    assert_eq!(bools.len(), 24);
    assert!(bools[..8].iter().all(|&b| b));
    assert!(bools[8..16].iter().all(|&b| !b));
    // 0xF0 = 1111_0000
    let expected_f0 = [true, true, true, true, false, false, false, false];
    assert_eq!(&bools[16..], &expected_f0);
}

// ── Optional ─────────────────────────────────────────────────────────────────

#[test]
fn take_bits_optional_on_empty() {
    let v = bits(take_bits::<u8, &[u8]>(4).optional())
        .parse(b"".as_ref())
        .unwrap();
    assert_eq!(v, None);
}

#[test]
fn take_bits_optional_on_enough() {
    let v = bits(take_bits::<u8, &[u8]>(4).optional())
        .parse_partial(b"\xAB".as_ref())
        .unwrap()
        .0;
    assert_eq!(v, Some(0x0A));
}

// ── BitSlice ─────────────────────────────────────────────────────────────────

#[test]
fn bit_slice_fields() {
    let input: &[u8] = b"\xAB\xCD";
    let bits_stream = Bits::new(input);
    // split_at(4) on a fresh stream: start_bit=0, len_bits=4
    let (slice, rest) = bits_stream.split_at(4);
    assert_eq!(slice.start_bit, 0);
    assert_eq!(slice.len_bits, 4);
    // rest should have bit_offset=4
    assert_eq!(rest.bit_offset(), 4);
}

#[test]
fn bit_slice_cross_byte() {
    let input: &[u8] = b"\xAB\xCD";
    let bits_stream = Bits::new(input);
    // Take 12 bits: spans 2 bytes
    let (slice, _rest) = bits_stream.split_at(12);
    assert_eq!(slice.start_bit, 0);
    assert_eq!(slice.len_bits, 12);
    assert_eq!(slice.bytes.len(), 2);
}

// ── Streaming (Partial) ──────────────────────────────────────────────────────

#[test]
fn bits_on_partial_incomplete() {
    // 1 byte available, ask for 9 bits → Incomplete on Partial
    let partial_input = Partial(b"\xAB".as_ref());
    let result = bits(take_bits::<u16, Partial<&[u8]>>(9)).parse_partial(partial_input);
    assert!(matches!(
        result,
        Err(nimble_parsec_rs::ParseError::Incomplete(_))
    ));
}

#[test]
fn bits_on_partial_sufficient() {
    // 2 bytes available, 12 bits requested → succeeds
    let partial_input = Partial(b"\xAB\xCD".as_ref());
    let (v, _rest) = bits(take_bits::<u16, Partial<&[u8]>>(12))
        .parse_partial(partial_input)
        .unwrap();
    assert_eq!(v, 0xABC);
}
