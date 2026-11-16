use romacs_vm::Vm;

/// GNU Fdefvar (eval.c) against the pinned GNU: a value-less `defvar' of a
/// variable already declared special adds nothing to the interpreter
/// environment, so a closure made after it is `equal' to the same lambda
/// made anywhere; one of a variable not yet special is a local special the
/// closure keeps. Bindings stay dynamic either way.
#[test]
fn a_closure_after_a_defvar_of_a_special_is_the_plain_lambda() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(progn
        (eval '(defvar cse-zz 1) t)
        (list (equal (eval '(progn (defvar cse-zz) (lambda () 1)) t) (eval '(lambda () 1) t))
              (equal (eval '(progn (defvar cse-zz 2) (lambda () 1)) t) (eval '(lambda () 1) t))
              (equal (eval '(progn (defvar cse-new) (lambda () 1)) t) (eval '(lambda () 1) t))
              (equal (eval '(progn (defvar cse-new2) (lambda () 1)) t) (eval '(progn (defvar cse-new2) (lambda () 1)) t))
              (equal (eval '(progn (defvar cse-zz) (lambda () (interactive) (insert \"x\"))) t)
                     (eval '(lambda () (interactive) (insert \"x\")) t))
              (eval '(progn (defvar cse-zz) (let ((cse-zz 5)) (funcall (lambda () (symbol-value 'cse-zz))))) t)
              (eval '(progn (defvar cse-new3) (let ((cse-new3 6)) (funcall (lambda () (symbol-value 'cse-new3))))) t)))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, "(t t nil t t 5 6)");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
}
