use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nimble_parsec_rs::{
    ascii_char, compile_parser, concat, ignore, integer_exact, AsciiPredicate, Value,
};
use num_bigint::BigInt;

fn build_runtime_parser() -> nimble_parsec_rs::Parser {
    let date = concat(
        concat(
            integer_exact(4),
            concat(
                ignore(nimble_parsec_rs::string("-")),
                concat(integer_exact(2), ignore(nimble_parsec_rs::string("-"))),
            ),
        ),
        integer_exact(2),
    );

    let time = concat(
        integer_exact(2),
        concat(
            ignore(nimble_parsec_rs::string(":")),
            concat(
                integer_exact(2),
                concat(ignore(nimble_parsec_rs::string(":")), integer_exact(2)),
            ),
        ),
    );

    concat(date, concat(ignore(nimble_parsec_rs::string("T")), time))
}

fn build_macro_parser() -> nimble_parsec_rs::Parser {
    compile_parser!(concat(
        concat(
            concat(
                integer_exact(4),
                concat(
                    ignore(nimble_parsec_rs::string("-")),
                    concat(integer_exact(2), ignore(nimble_parsec_rs::string("-")))
                )
            ),
            integer_exact(2)
        ),
        concat(
            ignore(nimble_parsec_rs::string("T")),
            concat(
                integer_exact(2),
                concat(
                    ignore(nimble_parsec_rs::string(":")),
                    concat(
                        integer_exact(2),
                        concat(ignore(nimble_parsec_rs::string(":")), integer_exact(2))
                    )
                )
            )
        )
    ))
}

/// A hand-written datetime parser representing the ceiling of what a perfect
/// `compile_parser!` specialization could emit (no AST, no per-node dispatch).
/// Benchmarking this against the interpreter shows whether codegen is worth its
/// complexity. Returns the consumed byte length, like the combinator parsers'
/// final offset.
fn handwritten_datetime(input: &str) -> Option<usize> {
    let b = input.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
    if !digits(&b[0..4]) || b[4] != b'-' || !digits(&b[5..7]) || b[7] != b'-' || !digits(&b[8..10])
    {
        return None;
    }
    if b[10] != b'T' {
        return None;
    }
    if !digits(&b[11..13])
        || b[13] != b':'
        || !digits(&b[14..16])
        || b[16] != b':'
        || !digits(&b[17..19])
    {
        return None;
    }
    Some(19)
}

/// Like [`handwritten_datetime`], but produces the same `Vec<Value>` tokens as
/// the combinator parser. This is the *fair* codegen ceiling: a specializer
/// must emit identical tokens, so the gap between this and the interpreter is
/// the dispatch overhead codegen could remove (token allocation is unavoidable
/// either way).
fn handwritten_datetime_tokens(input: &str) -> Option<Vec<Value>> {
    let b = input.as_bytes();
    handwritten_datetime(input)?;
    let int = |s: &[u8]| -> Value {
        Value::Int(std::str::from_utf8(s).unwrap().parse::<BigInt>().unwrap())
    };
    Some(vec![
        int(&b[0..4]),
        int(&b[5..7]),
        int(&b[8..10]),
        int(&b[11..13]),
        int(&b[14..16]),
        int(&b[17..19]),
    ])
}

fn parser_benchmark(c: &mut Criterion) {
    let input = black_box("2010-04-17T14:12:34");

    c.bench_function("handwritten_native_parse_datetime", |b| {
        b.iter(|| {
            let consumed = handwritten_datetime(input).expect("native parser should succeed");
            black_box(consumed);
        })
    });

    c.bench_function("handwritten_tokens_parse_datetime", |b| {
        b.iter(|| {
            let tokens =
                handwritten_datetime_tokens(input).expect("native token parser should succeed");
            black_box(tokens.len());
        })
    });

    c.bench_function("runtime_builder_parse_datetime", |b| {
        let parser = build_runtime_parser();
        b.iter(|| {
            let ok = parser.parse(input).expect("runtime parser should succeed");
            black_box(ok.cursor.byte_offset);
        })
    });

    c.bench_function("proc_macro_builder_parse_datetime", |b| {
        let parser = build_macro_parser();
        b.iter(|| {
            let ok = parser.parse(input).expect("macro parser should succeed");
            black_box(ok.cursor.byte_offset);
        })
    });

    c.bench_function("runtime_ascii_char", |b| {
        let parser = ascii_char(vec![AsciiPredicate::Range(b'a'..=b'z')]);
        b.iter(|| {
            let ok = parser.parse("abc").expect("ascii parser should succeed");
            black_box(ok.rest);
        })
    });
}

criterion_group!(benches, parser_benchmark);
criterion_main!(benches);
