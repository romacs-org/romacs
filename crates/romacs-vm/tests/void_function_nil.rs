use romacs_vm::Vm;

/// A call whose head is nil or t signals void-function with that very
/// symbol, as the pinned GNU does: dash's `-some-->' example compares the
/// condition with `(void-function nil)'.
#[test]
fn a_call_of_nil_signals_with_nil_itself() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(mapcar (lambda (form) (condition-case e (eval form t) (error (list (car e) (eq (car (cdr e)) (car form))))))
        '((nil) (nil 1 2) (t) (t 3)))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
}
