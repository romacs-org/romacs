//! What a search or a syntax scan does at every call besides reading its
//! text, counted: after its first call in a buffer it looks nothing up by
//! name, reads no variable through its binding tables, converts no position
//! Lisp does not get, copies no string and reads no syntax-table entry twice.
use crate::Vm;

/// What the calls of one form cost.
#[derive(Debug, PartialEq, Eq, Default)]
struct Work {
    /// Symbols a primitive found by name.
    names: usize,
    /// Variables read through `Vm::variable_value`.
    variables: usize,
    /// Buffer positions converted between bytes and characters.
    converted: usize,
    /// Strings copied to be read.
    copied: usize,
    /// Syntax-table entries read from a table's nodes or rows, not from
    /// its flat view (`char_table::flat`) kept between calls.
    table_reads: usize,
}

fn work() -> Work {
    let [single, rows] = crate::char_table::reads();
    Work {
        names: crate::prim::name_lookups(),
        variables: crate::variable_runtime::variable_reads(),
        converted: super::CONVERTED.with(std::cell::Cell::get),
        copied: super::COPIED.with(std::cell::Cell::get),
        table_reads: single + rows,
    }
}

/// A multibyte buffer of code as a major mode leaves it: its own syntax
/// table, the parsing variables buffer-local, `syntax-table` properties
/// honoured and propertized to the end.
fn code_buffer() -> Vm {
    let mut vm = Vm::new();
    vm.eval_source_raw(
        r#"(progn
             (set-buffer (get-buffer-create "entry"))
             (let ((i 0)) (while (< i 50) (insert "  (defun exämple (arg)) ;; comment here\n") (setq i (1+ i))))
             (let ((table (make-char-table 'syntax-table)))
               (set-char-table-parent table (standard-syntax-table))
               (modify-syntax-entry ?\; "<" table) (modify-syntax-entry ?\n ">" table)
               (set-syntax-table table))
             (make-local-variable 'parse-sexp-ignore-comments) (setq parse-sexp-ignore-comments t)
             (make-local-variable 'parse-sexp-lookup-properties) (setq parse-sexp-lookup-properties t)
             (make-local-variable 'case-fold-search) (setq case-fold-search nil)
             (put-text-property 3 4 'syntax-table '(4 . 41))
             (setq syntax-propertize--done most-positive-fixnum)
             (goto-char 10))"#,
    )
    .unwrap();
    vm
}

/// The value and the work of a hundred evaluations of BODY, after a
/// hundred that warm it.
fn repeated(vm: &mut Vm, body: &str) -> (String, Work) {
    let source = format!("(let ((i 0) (r nil)) (while (< i 100) (setq r {body}) (setq i (1+ i))) r)");
    let function = vm.compile_source(&source).unwrap();
    vm.execute(function).unwrap();
    let before = work();
    let value = vm.execute(function).unwrap();
    let after = work();
    let value = value.display().unwrap();
    let spent = Work {
        names: after.names - before.names,
        variables: after.variables - before.variables,
        converted: after.converted - before.converted,
        copied: after.copied - before.copied,
        table_reads: after.table_reads - before.table_reads,
    };
    (value, spent)
}

/// [`repeated`] while no char table was written: a write to any table, in
/// any VM of the process (other tests' included), has what is held of a
/// syntax table read again.
fn repeated_undisturbed(vm: &mut Vm, body: &str) -> (String, Work) {
    for _ in 0..50 {
        let writes = crate::char_table::writes();
        let result = repeated(vm, body);
        if crate::char_table::writes() == writes {
            return result;
        }
    }
    panic!("{body}: char tables were written throughout fifty runs");
}

/// What a hundred calls may pay together: less than one each.
const ONCE: usize = 8;

#[test]
fn a_failed_looking_at_converts_no_position_and_reads_no_binding_table() {
    let mut vm = code_buffer();
    let (value, spent) = repeated(&mut vm, r#"(looking-at "[ \t]*(def")"#);
    assert_eq!(value, "nil");
    assert!(spent.variables <= ONCE && spent.names <= ONCE, "{spent:?}");
    assert_eq!((spent.converted, spent.copied), (0, 0), "{spent:?}");
}

#[test]
fn a_search_converts_only_the_positions_lisp_gets() {
    let mut vm = code_buffer();
    // Point's position, and the two ends of the match.
    let (value, spent) = repeated(&mut vm, r#"(progn (goto-char 10) (re-search-forward "ex\\(ä\\)mple" nil t))"#);
    assert_eq!(value, "17");
    assert!(spent.variables <= ONCE && spent.names <= ONCE, "{spent:?}");
    assert!(spent.converted <= 4 * 100 && spent.copied == 0, "{spent:?}");
    let groups = vm.eval_source_raw("(list (match-beginning 0) (match-end 0) (match-beginning 1) (match-end 1) (point))").unwrap();
    assert_eq!(vm.display(groups).unwrap(), "(10 17 12 13 17)");
    // A bound is a position Lisp gave: it and point are converted.
    let (value, spent) = repeated(&mut vm, r#"(progn (goto-char 10) (re-search-forward "zz" 30 'move) (point))"#);
    assert_eq!(value, "30");
    assert!(spent.converted <= 2 * 100, "{spent:?}");
}

#[test]
fn string_match_reads_its_subject_where_it_lies() {
    let mut vm = code_buffer();
    let (value, spent) = repeated(&mut vm, r#"(string-match "b+\\(ä\\)" "aäbbäcc")"#);
    assert_eq!(value, "2");
    assert!(spent.variables <= ONCE && spent.names <= ONCE, "{spent:?}");
    assert_eq!((spent.converted, spent.copied), (0, 0), "{spent:?}");
    let groups = vm.eval_source_raw("(list (match-beginning 0) (match-end 0) (match-beginning 1) (match-end 1))").unwrap();
    assert_eq!(vm.display(groups).unwrap(), "(2 5 4 5)");
}

#[test]
fn a_scan_reads_its_variables_and_its_table_once() {
    let mut vm = code_buffer();
    for (body, expected) in [
        ("(progn (goto-char 24) (forward-comment 1) (point))", "41"),
        // Past no character beyond ASCII: a table has those read from it.
        ("(scan-sexps 18 1)", "23"),
        ("(scan-lists 18 1 0)", "23"),
        (r#"(progn (goto-char 24) (skip-syntax-forward "^w") (point))"#, "28"),
        ("(progn (goto-char 12) (backward-prefix-chars) (point))", "12"),
        ("(nth 0 (parse-partial-sexp 1 12))", "1"),
    ] {
        let (value, spent) = repeated_undisturbed(&mut vm, body);
        assert_eq!(value, expected, "{body}");
        assert!(spent.variables <= ONCE && spent.names <= ONCE, "{body}: {spent:?}");
        assert!(spent.table_reads <= ONCE, "{body}: {spent:?}");
    }
}

/// The places hold for a buffer and the bindings it has: another buffer,
/// a new local binding, a killed one, a `let`, a default and a
/// collection each read as pinned GNU reads them.
#[test]
fn a_search_follows_its_variables() {
    let mut vm = code_buffer();
    let steps = |vm: &mut Vm, form: &str, steps: &[(&str, &str)]| {
        let got: Vec<String> = steps.iter().map(|(around, _)| {
            let value = vm.eval_source_raw(&around.replace("FORM", form)).unwrap();
            vm.display(value).unwrap()
        }).collect();
        let expected: Vec<&str> = steps.iter().map(|&(_, expected)| expected).collect();
        assert_eq!(got, expected, "{form}");
    };
    steps(&mut vm, r#"(if (string-match "B" "abc") 'fold 'exact)"#, &[
        ("FORM", "exact"),
        ("(let ((case-fold-search t)) FORM)", "fold"),
        ("FORM", "exact"),
        ("(progn (kill-local-variable 'case-fold-search) FORM)", "fold"),
        ("(progn (make-local-variable 'case-fold-search) (setq case-fold-search nil) FORM)", "exact"),
        ("(let ((here (current-buffer))) (set-buffer (get-buffer-create \"other\")) (prog1 FORM (set-buffer here)))", "fold"),
        ("FORM", "exact"),
        ("(progn (set-default 'case-fold-search nil) (kill-local-variable 'case-fold-search) FORM)", "exact"),
        ("(progn (set-default 'case-fold-search t) FORM)", "fold"),
    ]);
    // The scanning variables: a comment is passed, or its first word is the sexp.
    steps(&mut vm, "(scan-sexps 24 1)", &[
        ("FORM", "64"),
        ("(let ((parse-sexp-ignore-comments nil)) FORM)", "35"),
        ("FORM", "64"),
        ("(progn (kill-local-variable 'parse-sexp-ignore-comments) FORM)", "35"),
        ("(progn (set-default 'parse-sexp-ignore-comments t) FORM)", "64"),
        ("(progn (garbage-collect) FORM)", "64"),
        ("(progn (setq parse-sexp-ignore-comments nil) FORM)", "35"),
    ]);
}
