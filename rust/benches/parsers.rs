//! Criterion benchmarks for `nimble_parsec_rs`.
//!
//! Measures throughput for common parsing patterns on both `&str` (char tokens)
//! and `&[u8]` (byte tokens) streams so regressions and cross-stream differences
//! are visible.
//!
//! Run with:
//!
//! ```sh
//! cargo bench
//! # or for a specific group:
//! cargo bench -- literal
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nimble_parsec_rs::typed::{
    be_u32, choice, digits, integer, literal, take, take_while, take_while1, Parser,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

// ── literal ───────────────────────────────────────────────────────────────────

fn bench_literal(c: &mut Criterion) {
    let mut group = c.benchmark_group("literal");

    // Text stream: find a literal at the start of a 1 KB input.
    let text_input = "Hello, world! ".repeat(70); // ~980 bytes
    group.throughput(Throughput::Bytes(text_input.len() as u64));
    group.bench_function("str/1kb", |b| {
        let p = literal("Hello, world! ");
        b.iter(|| p.parse_partial(text_input.as_str()).unwrap());
    });

    // Byte stream: same pattern on &[u8].
    let bytes_input: Vec<u8> = b"Hello, world! ".repeat(70);
    group.throughput(Throughput::Bytes(bytes_input.len() as u64));
    group.bench_function("bytes/1kb", |b| {
        let p = literal::<&[u8], _>(b"Hello, world! ".as_ref());
        b.iter(|| p.parse_partial(bytes_input.as_slice()).unwrap());
    });

    group.finish();
}

// ── take_while ────────────────────────────────────────────────────────────────

fn bench_take_while(c: &mut Criterion) {
    let mut group = c.benchmark_group("take_while");

    // Collect all ASCII letters from a homogeneous alphabetic string.
    for size in [64, 512, 4096] {
        let str_input = "a".repeat(size);
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("str", size), &str_input, |b, s| {
            let p = take_while::<&str, _>(|c: char| c.is_ascii_alphabetic());
            b.iter(|| p.parse_partial(s.as_str()).unwrap());
        });

        let bytes_input: Vec<u8> = b"a".repeat(size);
        group.bench_with_input(BenchmarkId::new("bytes", size), &bytes_input, |b, s| {
            let p = take_while::<&[u8], _>(|b: u8| b.is_ascii_alphabetic());
            b.iter(|| p.parse_partial(s.as_slice()).unwrap());
        });
    }

    group.finish();
}

// ── take ──────────────────────────────────────────────────────────────────────

fn bench_take(c: &mut Criterion) {
    let mut group = c.benchmark_group("take");

    for size in [16, 128, 1024] {
        let bytes_input: Vec<u8> = (0u8..=255).cycle().take(size + 8).collect();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("bytes", size), &bytes_input, |b, s| {
            let p = take::<&[u8]>(size);
            b.iter(|| p.parse_partial(s.as_slice()).unwrap());
        });

        let str_input: String = "a".repeat(size + 8);
        group.bench_with_input(BenchmarkId::new("str", size), &str_input, |b, s| {
            let p = take::<&str>(size);
            b.iter(|| p.parse_partial(s.as_str()).unwrap());
        });
    }

    group.finish();
}

// ── be_u32 ────────────────────────────────────────────────────────────────────

fn bench_be_u32(c: &mut Criterion) {
    let mut group = c.benchmark_group("be_u32");

    // Parse a sequence of 256 big-endian u32 values (1024 bytes).
    let data: Vec<u8> = (0u32..256).flat_map(|n| n.to_be_bytes()).collect();
    group.throughput(Throughput::Bytes(data.len() as u64));

    group.bench_function("sequential/256", |b| {
        let p = be_u32::<&[u8]>().repeated();
        b.iter(|| p.parse(data.as_slice()).unwrap());
    });

    group.finish();
}

// ── integer (text) ────────────────────────────────────────────────────────────

fn bench_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("integer");

    // A string of space-separated integers.
    let nums: String = (0i64..512)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    group.throughput(Throughput::Bytes(nums.len() as u64));
    group.bench_function("space_separated/512", |b| {
        let p = integer::<&str>()
            .then_ignore(literal(" ").optional())
            .repeated();
        b.iter(|| p.parse(nums.as_str()).unwrap());
    });

    group.finish();
}

// ── digits ────────────────────────────────────────────────────────────────────

fn bench_digits(c: &mut Criterion) {
    let mut group = c.benchmark_group("digits");

    for size in [16, 256, 4096] {
        let input = "1".repeat(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("str", size), &input, |b, s| {
            let p = digits::<&str>();
            b.iter(|| p.parse(s.as_str()).unwrap());
        });
    }

    group.finish();
}

// ── choice ────────────────────────────────────────────────────────────────────

fn bench_choice(c: &mut Criterion) {
    let mut group = c.benchmark_group("choice");

    // A sequence of single-char tokens; each is one of 4 alternatives.
    // Best case: the first alternative always matches.
    let best_case = "a".repeat(1024);
    // Worst case: the last alternative always matches.
    let worst_case = "d".repeat(1024);

    group.throughput(Throughput::Bytes(1024));

    group.bench_function("str/4-alt/best", |b| {
        let p = choice([literal("a"), literal("b"), literal("c"), literal("d")]).repeated();
        b.iter(|| p.parse(best_case.as_str()).unwrap());
    });

    group.bench_function("str/4-alt/worst", |b| {
        let p = choice([literal("a"), literal("b"), literal("c"), literal("d")]).repeated();
        b.iter(|| p.parse(worst_case.as_str()).unwrap());
    });

    group.finish();
}

// ── take_while1 ───────────────────────────────────────────────────────────────

fn bench_take_while1(c: &mut Criterion) {
    let mut group = c.benchmark_group("take_while1");

    for size in [64, 1024] {
        // Tokenize a run of identifier characters (letters + digits + '_').
        let ident = "ident_name_123".repeat(size / 14 + 1);
        let ident = &ident[..size];

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("str", size), ident, |b, s| {
            let p = take_while1::<&str, _>(|c: char| c.is_ascii_alphanumeric() || c == '_');
            b.iter(|| p.parse_partial(s).unwrap());
        });

        let bytes: Vec<u8> = ident.bytes().collect();
        group.bench_with_input(BenchmarkId::new("bytes", size), &bytes, |b, s| {
            let p = take_while1::<&[u8], _>(|b: u8| b.is_ascii_alphanumeric() || b == b'_');
            b.iter(|| p.parse_partial(s.as_slice()).unwrap());
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_literal,
    bench_take_while,
    bench_take,
    bench_be_u32,
    bench_integer,
    bench_digits,
    bench_choice,
    bench_take_while1,
);
criterion_main!(benches);
