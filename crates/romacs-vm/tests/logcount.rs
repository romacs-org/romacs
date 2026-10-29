use romacs_vm::Vm;

/// Flogcount (data.c:3530): the one bits of VALUE, or the zero bits of a
/// negative VALUE, fixnum or bignum; a non-integer is (wrong-type-argument
/// integerp X), a marker included.
#[test]
fn logcount_matches_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(mapcar (lambda (x) (condition-case e (logcount x) (error (list (car e) (car (cdr e))))))
 (list 0 1 2 3 7 255 -1 -2 -256 most-positive-fixnum most-negative-fixnum (1+ most-positive-fixnum) (1- most-negative-fixnum)
       (expt 2 200) (- (expt 2 200)) (1- (expt 2 200)) 1.0 'a nil (point-marker)))";
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
}
