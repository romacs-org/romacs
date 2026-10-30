//! A quoted `(lambda ...)' list is a function: eval.c funcall_lambda calls
//! it as a dynamically scoped interpreted function, through funcall, apply
//! and hooks, as pinned GNU answers.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    r#"(funcall '(lambda () 1))"#,
    r#"(funcall '(lambda (x &optional y &rest z) (list x y z)) 1 2 3 4)"#,
    r#"(funcall '(lambda () "Documented." 7))"#,
    r#"(apply '(lambda (&rest xs) (length xs)) 1 '(2 3))"#,
    r#"(progn (defvar lambda-list-dynamic 5) (let ((lambda-list-dynamic 6)) (funcall '(lambda () lambda-list-dynamic))))"#,
    r#"(progn (defvar lambda-list-hook nil) (defvar lambda-list-ran nil)
              (setq lambda-list-hook '(lambda () (setq lambda-list-ran 'once)))
              (run-hooks 'lambda-list-hook) lambda-list-ran)"#,
    r#"(progn (defvar lambda-list-rec '(lambda (n) (if (< n 1) 0 (+ n (funcall lambda-list-rec (1- n))))))
              (funcall lambda-list-rec 4))"#,
    r#"(condition-case err (funcall '(lambda () (car 1))) (error err))"#,
    r#"(condition-case err (funcall '(lambda)) (error err))"#,
    r#"(condition-case err (funcall '(lambda (x) x)) (error (car err)))"#,
];

#[test]
fn lambda_lists_are_called_as_pinned_gnu_calls_them() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    let mut failures = Vec::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string {expression})");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display())
            .map(|printed| serde_json::from_str::<String>(&printed).unwrap_or(printed))
            .unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
