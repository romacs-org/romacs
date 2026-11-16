use romacs_vm::Vm;

/// A call whose head is nil or t signals void-function with that very
/// symbol, as the pinned GNU does: dash's `-some-->' example compares the
/// condition with `(void-function nil)'.
#[test]
fn a_call_of_nil_signals_with_nil_itself() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(mapcar (lambda (form) (condition-case e (eval form t) (error (list (car e) (eq (car (cdr e)) (car form))))))
        '((nil) (nil 1 2) (t) (t 3)))";
    // An uninterned head keeps its identity, a void one and an uncallable one.
    let uninterned = "(let ((s (make-symbol \"nil\")) (u (make-symbol \"v\")))
        (fset u 7)
        (list (condition-case e (eval (list s) t) (error (eq (car (cdr e)) s)))
              (condition-case e (eval (list (make-symbol \"w\")) t) (error (car e)))
              (condition-case e (eval (list u) t) (error (eq (car (cdr e)) u)))))";
    let output = std::process::Command::new(&oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
    let output = std::process::Command::new(&oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {uninterned}))")])
        .output().unwrap();
    let gnu = String::from_utf8(output.stdout).unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {uninterned})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
}
