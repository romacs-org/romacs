//! A string search of a cached pattern that reads no syntax or category
//! table (`reads_lookup') skips the syntax a search would prepare. Each
//! input that preparation reads is changed between two searches here: the
//! plain pattern answers alike and prepares nothing, and a pattern that
//! reads the table still sees the change.
use crate::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    vm.display(value).unwrap()
}

/// SOURCE's value, and how many searches prepared syntax for it.
fn prepared(vm: &mut Vm, source: &str) -> (String, usize) {
    let before = super::PREPARED.with(std::cell::Cell::get);
    let value = eval(vm, source);
    (value, super::PREPARED.with(std::cell::Cell::get) - before)
}

/// A fresh syntax table inheriting the standard one.
const NEW_TABLE: &str = "(let ((table (make-char-table 'syntax-table))) (set-char-table-parent table (standard-syntax-table)) table)";

#[test]
fn a_plain_pattern_prepares_no_syntax_once_cached() {
    let mut vm = Vm::new();
    // The first search compiles, against the syntax it prepares.
    assert_eq!(prepared(&mut vm, r#"(string-match "a-c" "xa-c")"#), ("1".into(), 1));
    assert_eq!(prepared(&mut vm, r#"(string-match "a-c" "xa-c")"#), ("1".into(), 0));
    // A pattern that reads the table prepares it every time.
    assert_eq!(prepared(&mut vm, r#"(string-match "\\w" "-a")"#), ("1".into(), 1));
    assert_eq!(prepared(&mut vm, r#"(string-match "\\w" "-a")"#), ("1".into(), 1));
}

#[test]
fn the_syntax_table_and_modify_syntax_entry() {
    let mut vm = Vm::new();
    eval(&mut vm, r#"(progn (string-match "a-c" "a-c") (string-match "\\w" "-"))"#);
    eval(&mut vm, r#"(modify-syntax-entry ?- "w" (standard-syntax-table))"#);
    assert_eq!(eval(&mut vm, r#"(list (string-match "a-c" "xa-c") (string-match "\\w" "-"))"#), "(1 0)");
    // The current buffer's own table, `a' punctuation in it.
    assert_eq!(
        eval(&mut vm, &format!(r#"(let ((table {NEW_TABLE})) (modify-syntax-entry ?a "." table) (set-syntax-table table) (list (string-match "a-c" "xa-c") (string-match "\\w" "a")))"#)),
        "(1 nil)"
    );
}

#[test]
fn the_subjects_syntax_properties() {
    let mut vm = Vm::new();
    eval(&mut vm, r#"(progn (string-match "a-c" "a-c") (string-match "\\w" "-"))"#);
    let subject = r#"(let ((s (copy-sequence "-a-c"))) (put-text-property 0 4 'syntax-table (cons 2 nil) s) s)"#;
    assert_eq!(
        eval(&mut vm, &format!(r#"(let ((parse-sexp-lookup-properties t) (s {subject})) (list (string-match "a-c" s) (string-match "\\w" s)))"#)),
        "(1 0)"
    );
}

#[test]
fn case_fold_search_and_the_case_table() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, r#"(let ((case-fold-search nil)) (list (string-match "ab" "xAB") (string-match "ab" "xAB")))"#), "(nil nil)");
    assert_eq!(eval(&mut vm, r#"(let ((case-fold-search t)) (list (string-match "ab" "xAB") (string-match "ab" "xAB")))"#), "(1 1)");
    assert_eq!(eval(&mut vm, r#"(let ((case-fold-search nil)) (string-match "ab" "xAB"))"#), "nil");
    // `[:upper:]' and `[:lower:]' read the case table, not syntax: plain, and
    // still folded by case-fold-search (pinned GNU: ((0 0) (nil nil))), cached
    // or not, either order.
    let classes = r#"(list (let ((case-fold-search t)) (list (string-match "[[:upper:]]" "a") (string-match "[[:lower:]]" "A"))) (let ((case-fold-search nil)) (list (string-match "[[:upper:]]" "a") (string-match "[[:lower:]]" "A"))))"#;
    assert_eq!(eval(&mut vm, classes), "((0 0) (nil nil))");
    assert_eq!(eval(&mut vm, classes), "((0 0) (nil nil))");
}

#[test]
fn the_subjects_and_patterns_representation() {
    let mut vm = Vm::new();
    // A cached pattern meets multibyte, unibyte and multibyte subjects in turn,
    // and a unibyte pattern is its own entry.
    assert_eq!(
        eval(&mut vm, r#"(list (string-match "b" "\u00e9b") (string-match "b" (string-to-unibyte "ab")) (string-match "b" "\u00e9b") (string-match (string-to-unibyte "b") "\u00e9b"))"#),
        "(1 1 1 1)"
    );
}

#[test]
fn the_no_match_data_path_reads_the_table_only_for_a_pattern_that_reads_it() {
    let mut vm = Vm::new();
    let handlers = r#"(list (cons "\\`/:" 'plain-handler) (cons "\\`/\\w+:" 'word-handler))"#;
    let lookup = format!(r#"(let ((file-name-handler-alist {handlers})) (list (find-file-name-handler "/:x" 'file-exists-p) (find-file-name-handler "/a-b:x" 'file-exists-p)))"#);
    assert_eq!(eval(&mut vm, &lookup), "(plain-handler nil)");
    let (value, count) = prepared(&mut vm, &lookup);
    assert_eq!(value, "(plain-handler nil)");
    assert_eq!(count, 2, "only the `\\w' entry prepares syntax, once per lookup");
    eval(&mut vm, r#"(modify-syntax-entry ?- "w" (standard-syntax-table))"#);
    assert_eq!(eval(&mut vm, &lookup), "(plain-handler word-handler)");
}
