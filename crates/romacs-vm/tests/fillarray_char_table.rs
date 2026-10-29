use romacs_vm::Vm;

/// Ffillarray (fns.c:3308) on a char-table: every character and the
/// default become ITEM, dropping sub-tables, and the parent still answers
/// for nil. Here GNU and VM2 agree; GNU alone keeps a stale ASCII cache,
/// so an ASCII character it had set reads its old value there
/// (VM2-FILLARRAY1), which VM2 does not reproduce.
#[test]
fn fillarray_fills_char_tables_as_gnu_does() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(list
 (let ((ct (make-char-table 'foo))) (aset ct 300 3) (set-char-table-range ct '(#x4e00 . #x9fff) 4) (list (eq ct (fillarray ct 2)) (aref ct 300) (aref ct #x5000) (aref ct #x10ffff) (char-table-range ct nil)))
 (let ((ct (make-char-table 'foo 7))) (fillarray ct nil) (list (aref ct 300) (char-table-range ct nil)))
 (let ((ct (make-char-table 'foo))) (set-char-table-parent ct (let ((p (make-char-table 'foo))) (aset p 955 9) p)) (fillarray ct nil) (aref ct 955))
 (let ((m (make-keymap))) (fillarray (car (cdr m)) 'undef) (list (lookup-key m "a") (lookup-key m [945]) (lookup-key m [f1]))))"#;
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
    let ascii = "(let ((ct (make-char-table 'foo))) (aset ct ?a 1) (fillarray ct 2) (list (aref ct ?a) (aref ct ?b)))";
    assert_eq!(vm.eval_source(ascii).unwrap().display().unwrap(), "(2 2)");
}
