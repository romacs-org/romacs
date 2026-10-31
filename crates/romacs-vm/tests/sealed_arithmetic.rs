//! Three-argument `+', `-', `*', `min' and `max' compile as byte-opt.el
//! rewrites them, `(OP (OP x y) z)' (DIVERGENCES.md VM2-SEALED1): the inner
//! operation runs before the third argument is evaluated, so a marker it
//! reads and the first signal are byte-compiled GNU's.
use romacs_vm::Vm;

#[test]
fn three_argument_arithmetic_matches_compiled_gnu() {
    let case = r#"(let ((m (copy-marker 2)) (n 10) (out nil))
  (insert "xyz")
  (setq out (list (+ m n (progn (goto-char 1) (insert "abc") n))
                  (- m n (progn (insert "d") n))
                  (* n n n) (min n m (progn (insert "e") n)) (max m n (point))
                  (condition-case e (+ 'a n (error "late")) (error e))
                  (condition-case e (* n 'b (error "late")) (error e))
                  (+ n n 1) (+ 1.5 n n)))
  out)"#;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary)
        .args(["-Q", "--batch", "--eval", "(setq lexical-binding t byte-compile-warnings nil)",
            "--eval", &format!("(princ (prin1-to-string (funcall (byte-compile (lambda () {case})))))")])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    let actual = vm.eval_source(case).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
