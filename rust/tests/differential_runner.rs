use std::collections::HashMap;
use std::process::Command;

use nimble_parsec_rs::{
    ascii_char, concat, ignore, integer_exact, integer_min, lookahead, repeat_while, string,
    AsciiPredicate, RepeatWhileControl, Value,
};

/// True when an Elixir `mix` is runnable, so the differential test can be
/// skipped gracefully on machines (e.g. CI) without the Elixir toolchain.
fn mix_available() -> bool {
    Command::new("mix")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn differential_runner_matches_shared_elixir_scenarios() {
    if !mix_available() {
        eprintln!("skipping differential test: `mix` is not available on PATH");
        return;
    }

    let output = Command::new("mix")
        .arg("run")
        .arg("rust/tests/fixtures/differential_cases.exs")
        .current_dir("..")
        .output()
        .expect("failed to execute mix run for differential fixtures");

    assert!(
        output.status.success(),
        "elixir fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut elixir = HashMap::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 6 {
            elixir.insert(parts[1].to_string(), parts);
        }
    }

    let rust_datetime = datetime_parser()
        .parse("2010-04-17T14:12:34")
        .expect("datetime parse");
    assert_case(
        elixir.get("datetime").expect("missing datetime case"),
        &rust_datetime,
    );

    let rust_lookahead = lookahead_digit_parser()
        .parse("a0")
        .expect("lookahead parse");
    assert_case(
        elixir
            .get("lookahead_digit")
            .expect("missing lookahead_digit case"),
        &rust_lookahead,
    );

    let rust_repeat = repeat_while_digits_parser()
        .parse("12345")
        .expect("repeat_while parse");
    assert_case(
        elixir
            .get("repeat_while_digits")
            .expect("missing repeat_while_digits case"),
        &rust_repeat,
    );
}

fn assert_case(elixir: &[&str], rust: &nimble_parsec_rs::ParseSuccess<'_>) {
    assert_eq!(elixir[0], "ok");
    assert_eq!(elixir[2], rust.rest);
    assert_eq!(
        elixir[3].parse::<usize>().expect("offset parse"),
        rust.cursor.byte_offset
    );
    assert_eq!(
        elixir[4].parse::<usize>().expect("token_count parse"),
        rust.tokens.len()
    );
    assert_eq!(elixir[5], format_tokens(&rust.tokens));
}

/// Mirrors `format_tokens/1` in the Elixir fixture so token values can be
/// compared directly, not just by count.
fn format_tokens(tokens: &[Value]) -> String {
    tokens
        .iter()
        .map(format_token)
        .collect::<Vec<_>>()
        .join(",")
}

fn format_token(value: &Value) -> String {
    match value {
        Value::Int(n) => n.to_string(),
        Value::Str(s) => format!("s:{s}"),
        Value::List(inner) => format!("l:({})", format_tokens(inner)),
        Value::Tagged(name, inner) => format!("t:{name}:({})", format_tokens(inner)),
        Value::KeyValue(name, inner) => format!("kv:{name}:{}", format_token(inner)),
    }
}

fn datetime_parser() -> nimble_parsec_rs::Parser {
    let date = concat(
        concat(
            integer_exact(4),
            concat(
                ignore(string("-")),
                concat(integer_exact(2), ignore(string("-"))),
            ),
        ),
        integer_exact(2),
    );

    let time = concat(
        integer_exact(2),
        concat(
            ignore(string(":")),
            concat(
                integer_exact(2),
                concat(ignore(string(":")), integer_exact(2)),
            ),
        ),
    );

    concat(date, concat(ignore(string("T")), time))
}

fn lookahead_digit_parser() -> nimble_parsec_rs::Parser {
    concat(
        ascii_char(vec![AsciiPredicate::Any]),
        lookahead(integer_min(1)),
    )
}

fn repeat_while_digits_parser() -> nimble_parsec_rs::Parser {
    repeat_while(
        concat(
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
            ascii_char(vec![AsciiPredicate::Range(b'0'..=b'9')]),
        ),
        |rest, _| {
            if rest.starts_with('3') {
                RepeatWhileControl::Halt
            } else {
                RepeatWhileControl::Cont
            }
        },
        0,
        None,
    )
}
