//! Interpreted closure environments and the nadvice consumers of slot 2,
//! compared with the pinned GNU interpreter on the loadup chain.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;

const EXPRESSIONS: &[&str] = &[
    "(aref (lambda (x) x) 2)",
    "(eval '(aref (lambda (x) x) 2) nil)",
    "(progn (defun ce-f (x) (interactive (list 1)) x) (aref (symbol-function 'ce-f) 2))",
    "(progn (defun ce-cmd (x) (interactive (list (+ 1 2))) x) (advice-add 'ce-cmd :around (lambda (f &rest a) (apply f a))) (list (commandp 'ce-cmd) (call-interactively 'ce-cmd) (car (interactive-form 'ce-cmd))))",
    r#"(progn
      (defun ce-captured (x) (interactive "p") x)
      (advice-add 'ce-captured :around
        (let ((k 1)) (lambda (o &rest a) (interactive (list k)) (apply o a)))
        '((depth . -99)))
      (list (commandp 'ce-captured) (interactive-form 'ce-captured)
            (call-interactively 'ce-captured)))"#,
    r"(progn
      (defun ce-iform-function (x) (interactive (list 2)) x)
      (advice-add 'ce-iform-function :around
        (let ((k 40))
          (lambda (o &rest a)
            (interactive (lambda (spec) (list (+ k (car (advice-eval-interactive-spec spec))))))
            (apply o a))))
      (list (commandp 'ce-iform-function) (call-interactively 'ce-iform-function)))",
    r"(progn
      (defun ce-once (x) (interactive (list 2)) x)
      (let ((k 40) (offset 3))
        (letrec ((wrapper (lambda (o x)
                            (interactive (list k))
                            (advice-remove 'ce-once wrapper)
                            (funcall o (+ offset x)))))
          (advice-add 'ce-once :around wrapper)))
      (list (commandp 'ce-once) (call-interactively 'ce-once) (call-interactively 'ce-once)))",
    "(let ((k 1) (unused 9)) (aref (lambda (x) (interactive (list k)) x) 2))",
    "(let ((y 5) (unused 9)) (aref (lambda (x) (+ x y)) 2))",
    "(let ((y 5)) (aref (lambda (x) x) 2))",
    "(let ((y 5)) (let* ((f (lambda () (setq y (1+ y)))) (env (aref f 2))) (setcdr (assq 'y env) 40) (list (funcall f) y)))",
    r#"(list (length (lambda (x) x))
             (length (lambda (x) "d" x))
             (length (lambda (x) (interactive "p") x))
             (length (lambda (x) (interactive) x)))"#,
    r#"(let ((f (lambda (x) "doc" (interactive "p" foo-mode) x)))
         (list (aref f 0) (aref f 3) (aref f 4) (aref f 5)
               (arrayp f) (sequencep f) (vectorp f)))"#,
    "(condition-case e (aref (lambda (x) x) 3) (args-out-of-range (list (car e) (nth 2 e))))",
    "(condition-case e (aset (lambda (x) x) 0 nil) (wrong-type-argument (list (car e) (nth 1 e))))",
];

#[test]
fn closure_slots_and_advice_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = gnu_lisp(&[]);
    let mut failures = Vec::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
