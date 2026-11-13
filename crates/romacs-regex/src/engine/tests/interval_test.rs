//! Interval syntax `\{n,m\}`: every malformed form signals the error GNU's
//! `GET_INTERVAL_COUNT` and `handle_interval` (regex-emacs.c) give it, and
//! every well-formed one compiles to its bounds.  The messages were recorded
//! from the pinned GNU with `(string-match RE "")`.

use crate::engine::compile::regex_compile;
use crate::engine::matcher::re_match;
use crate::engine::syntax::DefaultSyntaxLookup;

const UNMATCHED: &str = "Unmatched \\{";
const INVALID: &str = "Invalid content of \\{\\}";
const TRAILING: &str = "Trailing backslash";

fn compile_error(pattern: &str) -> String {
    crate::test_utils::init_test_tracing();
    match regex_compile(pattern, false, false) {
        Ok(_) => panic!("{pattern:?} should not compile"),
        Err(err) => err.message,
    }
}

/// Length of the match of PATTERN at the start of TEXT, if any.
fn match_len(pattern: &str, text: &str) -> Option<usize> {
    crate::test_utils::init_test_tracing();
    let compiled = regex_compile(pattern, false, false)
        .unwrap_or_else(|err| panic!("{pattern:?} should compile: {err:?}"));
    re_match(&compiled, text.as_bytes(), 0, text.len(), &DefaultSyntaxLookup, 0).map(|(end, _)| end)
}

#[test]
fn open_brace_at_end_is_unmatched() {
    assert_eq!(compile_error("a\\{"), UNMATCHED);
}

#[test]
fn lower_bound_at_end_is_unmatched() {
    assert_eq!(compile_error("a\\{1"), UNMATCHED);
    assert_eq!(compile_error("a\\{2"), UNMATCHED);
}

#[test]
fn comma_at_end_is_unmatched() {
    assert_eq!(compile_error("a\\{1,"), UNMATCHED);
    assert_eq!(compile_error("\\(a\\)\\{1,"), UNMATCHED);
    assert_eq!(compile_error("\\{1,"), UNMATCHED);
}

#[test]
fn upper_bound_at_end_is_unmatched() {
    assert_eq!(compile_error("a\\{1,2"), UNMATCHED);
}

#[test]
fn missing_lower_bound_is_zero() {
    assert_eq!(match_len("a\\{,2\\}", "aaa"), Some(2));
    assert_eq!(match_len("a\\{,2\\}", "b"), Some(0));
}

#[test]
fn missing_upper_bound_is_unbounded() {
    assert_eq!(match_len("a\\{1,\\}", "aaa"), Some(3));
    assert_eq!(match_len("a\\{1,\\}", "b"), None);
    assert_eq!(match_len("a\\{,\\}", "aaa"), Some(3));
}

#[test]
fn empty_braces_match_nothing() {
    assert_eq!(match_len("a\\{\\}", "aaa"), Some(0));
}

#[test]
fn descending_bounds_are_invalid() {
    for pattern in ["a\\{2,1\\}", "a\\{5,2\\}", "a\\{3,0\\}"] {
        assert_eq!(compile_error(pattern), INVALID, "{pattern:?}");
    }
}

#[test]
fn ascending_and_equal_bounds_compile() {
    assert_eq!(match_len("a\\{2,3\\}", "aaaa"), Some(3));
    assert_eq!(match_len("a\\{2,2\\}", "aaaa"), Some(2));
    assert_eq!(match_len("a\\{0,2\\}", "aaaa"), Some(2));
}

#[test]
fn bound_over_re_dup_max_is_invalid() {
    assert!(regex_compile("a\\{65535\\}", false, false).is_ok());
    assert_eq!(compile_error("a\\{65536\\}"), INVALID);
    assert_eq!(compile_error("a\\{1,65536\\}"), INVALID);
    // GNU rejects the count as soon as it overflows, before the pattern ends.
    assert_eq!(compile_error("a\\{99999"), INVALID);
}

#[test]
fn stray_character_in_braces_is_invalid() {
    assert_eq!(compile_error("a\\{0,x"), INVALID);
    assert_eq!(compile_error("a\\{1x\\}"), INVALID);
    assert_eq!(compile_error("a\\{1,2x"), INVALID);
}

#[test]
fn backslash_not_before_close_brace_is_invalid() {
    assert_eq!(compile_error("a\\{1\\x"), INVALID);
}

#[test]
fn backslash_at_end_is_trailing() {
    assert_eq!(compile_error("a\\{1\\"), TRAILING);
    assert_eq!(compile_error("a\\{1,2\\"), TRAILING);
}

#[test]
fn interval_without_preceding_atom_is_literal() {
    assert_eq!(match_len("\\{1\\}", "{1}"), Some(3));
}
