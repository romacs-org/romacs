use romacs_vm::Vm;
fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let result = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(result.as_deref(), Ok(expected), "charset alias shares canonical attributes");
}
#[test]
fn alias_returns_nil_and_shares_plist_identity() {
    check("(list (define-charset-alias 'ucs 'unicode) (let ((p (list :probe 9))) (set-charset-plist 'ucs p) (eq (charset-plist 'unicode) p)))", "(nil t)");
    check("(progn (define-charset-alias 'ucs 'unicode) (define-charset-alias 'ucs 'ascii) (set-charset-plist 'ucs 9) (list (charset-plist 'ascii) (eq (charset-plist 'unicode) 9)))", "(9 nil)");
}
#[test]
fn alias_invalid_target_preserves_condition_data() {
    check("(condition-case e (define-charset-alias 'ucs 'absent) (error e))", "(wrong-type-argument charsetp absent)");
    check("(list (define-charset-alias 42 'ascii) (condition-case e (charset-plist 42) (error e)))", "(nil (wrong-type-argument charsetp 42))");
}
