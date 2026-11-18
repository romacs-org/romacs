//! The compiled-pattern cache across `modify-syntax-entry`: a pattern whose
//! compilation read no syntax table keeps its compilation, and one whose
//! compilation read a table's content (`used_syntax`) is compiled again,
//! against the table as it is now.
use crate::Vm;
use romacs_regex as regex;

/// The compilation the cache holds for SOURCE, by identity.
fn compiled(vm: &Vm, source: &str) -> Option<*const std::cell::RefCell<regex::CompiledPattern>> {
    vm.regexp_cache.patterns.get(source.as_bytes()).and_then(|v| v.first()).map(|e| std::rc::Rc::as_ptr(&e.compiled))
}

/// A fresh syntax table inheriting the standard one, as `make-syntax-table` (Lisp) makes it.
const NEW_TABLE: &str = "(let ((table (make-char-table 'syntax-table))) (set-char-table-parent table (standard-syntax-table)) table)";

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    vm.display(value).unwrap()
}

#[test]
fn an_unrelated_modify_syntax_entry_keeps_patterns_that_read_no_syntax_table() {
    let mut vm = Vm::new();
    eval(&mut vm, r#"(progn (string-match "foo[0-9]+" "x foo12") (string-match "\\sw+" "ab") (string-match "[[:word:]]+x" "abx"))"#);
    let plain = compiled(&vm, "foo[0-9]+").expect("compiled and cached");
    let syntax_at_match = compiled(&vm, "\\sw+").expect("compiled and cached");
    assert!(compiled(&vm, "[[:word:]]+x").is_some(), "the word class is cached under its syntax table");

    eval(&mut vm, &format!(r#"(modify-syntax-entry ?_ "w" {NEW_TABLE})"#));

    assert_eq!(compiled(&vm, "foo[0-9]+"), Some(plain), "a pattern that read no syntax table keeps its compilation");
    assert_eq!(compiled(&vm, "\\sw+"), Some(syntax_at_match), "`\\sw' reads the table only while it matches");
    assert_eq!(compiled(&vm, "[[:word:]]+x"), None, "a pattern whose fastmap read the table is dropped");
    assert_eq!(eval(&mut vm, r#"(string-match "foo[0-9]+" "x foo12")"#), "2");
}

#[test]
fn kept_and_recompiled_patterns_match_by_the_table_as_modified() {
    let mut vm = Vm::new();
    let ends = r#"(list (progn (string-match "\\sw+" "a_b") (match-end 0)) (progn (string-match "[[:word:]]+" "a_b") (match-end 0)))"#;
    eval(&mut vm, &format!("(set-syntax-table {NEW_TABLE})"));
    assert_eq!(eval(&mut vm, ends), "(1 1)", "`_' is symbol syntax");
    let kept = compiled(&vm, "\\sw+").expect("compiled and cached");

    eval(&mut vm, r#"(modify-syntax-entry ?_ "w")"#);

    assert_eq!(eval(&mut vm, ends), "(3 3)", "`_' is word syntax now, for both");
    assert_eq!(compiled(&vm, "\\sw+"), Some(kept), "the match-time pattern was not compiled again");
}
