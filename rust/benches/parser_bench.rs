use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nimble_parsec_rs::{
    ascii_char, compile_parser, concat, ignore, integer_exact, AsciiPredicate,
};

fn build_runtime_parser() -> nimble_parsec_rs::Parser {
    let date = concat(
        concat(
            integer_exact(4),
            concat(ignore(nimble_parsec_rs::string("-")), concat(integer_exact(2), ignore(nimble_parsec_rs::string("-")))),
        ),
        integer_exact(2),
    );

    let time = concat(
        integer_exact(2),
        concat(
            ignore(nimble_parsec_rs::string(":")),
            concat(integer_exact(2), concat(ignore(nimble_parsec_rs::string(":")), integer_exact(2))),
        ),
    );

    concat(date, concat(ignore(nimble_parsec_rs::string("T")), time))
}

fn build_macro_parser() -> nimble_parsec_rs::Parser {
    compile_parser!(concat(
        concat(
            concat(
                integer_exact(4),
                concat(ignore(nimble_parsec_rs::string("-")), concat(integer_exact(2), ignore(nimble_parsec_rs::string("-"))))
            ),
            integer_exact(2)
        ),
        concat(
            ignore(nimble_parsec_rs::string("T")),
            concat(
                integer_exact(2),
                concat(
                    ignore(nimble_parsec_rs::string(":")),
                    concat(integer_exact(2), concat(ignore(nimble_parsec_rs::string(":")), integer_exact(2)))
                )
            )
        )
    ))
}

fn parser_benchmark(c: &mut Criterion) {
    let input = black_box("2010-04-17T14:12:34");

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
