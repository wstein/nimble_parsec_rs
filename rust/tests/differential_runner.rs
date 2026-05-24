use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use nimble_parsec_rs::{
    ascii_char, ascii_string, concat, duplicate, generate, ignore, integer_exact, integer_min,
    lookahead, repeat_while, string, tag, wrap, AsciiPredicate, RepeatWhileControl, Value,
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

    let rust_tagged = tag("n", integer_min(1)).parse("42").expect("tag parse");
    assert_case(
        elixir.get("tagged_int").expect("missing tagged_int case"),
        &rust_tagged,
    );

    let rust_wrapped = wrap(concat(
        integer_exact(2),
        concat(ignore(string(",")), integer_exact(2)),
    ))
    .parse("12,34")
    .expect("wrap parse");
    assert_case(
        elixir
            .get("wrapped_pair")
            .expect("missing wrapped_pair case"),
        &rust_wrapped,
    );

    let rust_ascii = ascii_string(vec![AsciiPredicate::Range(b'a'..=b'z')], 1, None)
        .parse("abc123")
        .expect("ascii_string parse");
    assert_case(
        elixir.get("ascii_lower").expect("missing ascii_lower case"),
        &rust_ascii,
    );

    let rust_dup = duplicate(string("ab"), 3)
        .parse("ababab")
        .expect("duplicate parse");
    assert_case(
        elixir.get("dup_ab").expect("missing dup_ab case"),
        &rust_dup,
    );
}

#[test]
fn differential_generate_fuzzing_datetime() {
    if !mix_available() {
        eprintln!("skipping fuzz test: `mix` is not available on PATH");
        return;
    }

    let parser = datetime_parser();
    let inputs: Vec<String> = (0..30u64).map(|seed| generate(&parser, seed)).collect();
    let stdin_data = format!("{}\n", inputs.join("\n"));

    let mut child = Command::new("mix")
        .arg("run")
        .arg("rust/tests/fixtures/fuzz_cases.exs")
        .current_dir("..")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn mix for fuzz fixtures");

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin_data.as_bytes())
        .expect("failed to write generated inputs to mix");

    let output = child.wait_with_output().expect("failed to wait for mix");
    assert!(
        output.status.success(),
        "elixir fuzz fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let elixir_lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        elixir_lines.len(),
        inputs.len(),
        "expected one Elixir result per generated input"
    );

    for (input, line) in inputs.iter().zip(elixir_lines) {
        let rust = parser
            .parse(input)
            .unwrap_or_else(|e| panic!("Rust rejected generated {input:?}: {}", e.reason));
        let parts: Vec<&str> = line.split('|').collect();
        assert_eq!(parts[0], "ok", "Elixir rejected generated {input:?}");
        assert_eq!(parts[1], rust.rest, "rest mismatch for {input:?}");
        assert_eq!(
            parts[2].parse::<usize>().expect("offset"),
            rust.cursor.byte_offset,
            "offset mismatch for {input:?}"
        );
        assert_eq!(
            parts[3].parse::<usize>().expect("count"),
            rust.tokens.len(),
            "token count mismatch for {input:?}"
        );
        assert_eq!(
            parts[4],
            format_tokens(&rust.tokens),
            "token value mismatch for {input:?}"
        );
    }
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
        // `Value` is #[non_exhaustive]; the differential fixtures only emit the
        // variants above.
        other => unreachable!("unexpected Value variant in differential serializer: {other:?}"),
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
        |rest, _, _| {
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
