use nimble_parsec_rs::{choice, concat, integer_min, map, recursive, string};

#[test]
fn parser_debug_is_structural() {
    let parser = concat(string("ab"), integer_min(1));
    let rendered = format!("{parser:?}");
    assert!(rendered.contains("Concat"), "got: {rendered}");
    assert!(rendered.contains("Str(\"ab\")"), "got: {rendered}");
    assert!(rendered.contains("Integer"), "got: {rendered}");
}

#[test]
fn debug_renders_closures_as_placeholder() {
    let parser = map(integer_min(1), |v| v);
    let rendered = format!("{parser:?}");
    assert!(rendered.contains("Map("), "got: {rendered}");
    assert!(rendered.contains("<fn>"), "got: {rendered}");
}

#[test]
fn recursive_parser_debug_terminates() {
    let parser = recursive(|expr| {
        choice(vec![
            concat(string("("), concat(expr, string(")"))),
            string("x"),
        ])
    });
    // Must not recurse forever through the reference cell.
    let rendered = format!("{parser:?}");
    assert!(rendered.contains("Reference(<ref>)"), "got: {rendered}");
}
