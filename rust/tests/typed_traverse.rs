//! `post_traverse` / `pre_traverse`: position-aware fallible transforms, and the
//! threaded-context pattern (interior-mutable state captured by the callback).

use std::cell::RefCell;
use std::collections::HashSet;

use nimble_parsec_rs::typed::{literal, take_while1, Parser};

#[test]
fn pre_traverse_tags_a_result_with_its_start_position() {
    // `pre_traverse` sees the cursor *before* the match.
    let word = take_while1(|c: char| c.is_alphabetic())
        .pre_traverse(|w, start| Ok((w, start.byte_offset)));
    let tagged = literal("..").ignore_then(word);
    assert_eq!(tagged.parse("..hi").unwrap(), ("hi", 2));
}

#[test]
fn post_traverse_sees_the_end_position_and_can_fail() {
    // Reject identifiers longer than 3 characters, using the end offset.
    let ident = take_while1(|c: char| c.is_alphabetic()).post_traverse(|w: &str, end| {
        if w.len() <= 3 {
            Ok((w, end.byte_offset))
        } else {
            Err(format!("identifier {w:?} too long"))
        }
    });
    assert_eq!(ident.parse("abc").unwrap(), ("abc", 3));
    let err = ident.parse("abcd").unwrap_err();
    assert_eq!(err.reason, "identifier \"abcd\" too long");
    assert!(err.expected.is_empty());
}

#[test]
fn context_threads_through_post_traverse_to_reject_duplicates() {
    // The canonical NimbleParsec context use: thread a symbol table to make
    // parsing context-dependent. Here, captured `seen` rejects a repeated key.
    let seen = RefCell::new(HashSet::new());

    let key = take_while1(|c: char| c.is_alphabetic()).post_traverse(|name: &str, _end| {
        if seen.borrow_mut().insert(name.to_string()) {
            Ok(name.to_string())
        } else {
            Err(format!("duplicate key {name:?}"))
        }
    });
    let list = key.then_ignore(literal(";").optional()).repeated();

    let parsed = list.parse("a;b;c").unwrap();
    assert_eq!(parsed, vec!["a", "b", "c"]);
    assert_eq!(seen.borrow().len(), 3);

    // A fresh context for a run with a repeat → context-dependent failure.
    let seen = RefCell::new(HashSet::new());
    let key = take_while1(|c: char| c.is_alphabetic()).post_traverse(|name: &str, _end| {
        if seen.borrow_mut().insert(name.to_string()) {
            Ok(name.to_string())
        } else {
            Err(format!("duplicate key {name:?}"))
        }
    });
    let list = key.then_ignore(literal(";").optional()).repeated();
    // `repeated` stops at the failing item, so "a;a" leaves the second `a`
    // unconsumed and the overall (full-input) parse fails.
    assert!(list.parse("a;a").is_err());
}
