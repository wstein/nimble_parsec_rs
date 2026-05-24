use nimble_parsec_rs::{compile_parser, concat, integer_min, string};

#[test]
fn compile_parser_builds_an_equivalent_parser() {
    let built = compile_parser!(concat(string("ab"), integer_min(1)));
    let runtime = concat(string("ab"), integer_min(1));

    let from_macro = built.parse("ab12").expect("macro parser should parse");
    let from_runtime = runtime.parse("ab12").expect("runtime parser should parse");

    assert_eq!(from_macro.tokens, from_runtime.tokens);
    assert_eq!(from_macro.rest, from_runtime.rest);
}
