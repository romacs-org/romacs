use romacs_vm::Vm;

/// GNU Fdefvar (eval.c:944) against the pinned GNU: a void current
/// binding, a let's included, is set as set-default sets it; a let that
/// shadows an unbound toplevel value gets that saved value written; a bound
/// variable is left alone. Each case reports the binding inside the let
/// and the value after it.
#[test]
fn defvar_sets_the_binding_gnu_sets() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(let ((out nil))
        (defvar dv-a) (defvar dv-b) (defvar dv-c 3) (defvar dv-d)
        (push (let ((dv-a 1)) (makunbound 'dv-a) (eval '(defvar dv-a 5) t) (list dv-a)) out)
        (push (boundp 'dv-a) out)
        (push (let ((dv-b 1)) (eval '(defvar dv-b 5) t) (list dv-b)) out)
        (push (list (boundp 'dv-b) (and (boundp 'dv-b) dv-b)) out)
        (push (let ((dv-c 1)) (eval '(defvar dv-c 5) t) (list dv-c)) out)
        (push dv-c out)
        (eval '(defvar dv-d 7) t)
        (push dv-d out)
        (nreverse out))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert!(gnu.starts_with("((5) nil (1) (t 5)"), "{gnu}");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
}
