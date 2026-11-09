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
    // Compiled once per list while it is unchanged (VM2-LAMBDA-LIST-EXPAND):
    // each change GNU's interpreter would see takes effect at the next call.
    r#"(let ((f (list 'lambda '() 1))) (list (funcall f) (progn (setcar (cdr (cdr f)) 2) (funcall f))))"#,
    r#"(progn (defmacro lambda-list-len (q) (length (car (cdr q))))
              (let* ((q (list 1 2)) (f (list 'lambda '() (list 'lambda-list-len (list 'quote q)))))
                (list (funcall f) (progn (setcdr q nil) (funcall f)))))"#,
    r#"(let* ((s (copy-sequence "ab")) (f (list 'lambda '() (list 'aref s 0)))) (list (funcall f) (progn (aset s 0 ?z) (funcall f))))"#,
    r#"(let* ((v (vector 1 2)) (f (list 'lambda '() (list 'aref v 0)))) (list (funcall f) (progn (aset v 0 9) (funcall f))))"#,
    r#"(progn (defmacro lambda-list-m () 1)
              (let ((f '(lambda () (lambda-list-m)))) (list (funcall f) (progn (defmacro lambda-list-m () 2) (funcall f)))))"#,
    r#"(progn (defun lambda-list-g () 'fn)
              (let ((f '(lambda () (lambda-list-g)))) (list (funcall f) (progn (defmacro lambda-list-g () ''mac) (funcall f)))))"#,
    r#"(progn (defvar lambda-list-gc '(lambda (n) (if (< n 1) (progn (garbage-collect) 0) (+ n (funcall lambda-list-gc (1- n))))))
              (list (funcall lambda-list-gc 3) (funcall lambda-list-gc 3)))"#,
    r#"(let* ((b (get-buffer-create " lambda-list-buffer")) (f (list 'lambda '() (list 'buffer-name b))))
         (list (funcall f) (progn (garbage-collect) (funcall f))))"#,
    // Replacing a quoted operand with an `equal' fresh list is a change:
    // GNU returns the new object.
    r#"(let* ((f (list 'lambda '() (list 'quote (list 'a)))) (first (funcall f)))
         (setcar (cdr (car (cdr (cdr f)))) (list 'a))
         (list (eq (funcall f) first) (equal (funcall f) first)))"#,
    r#"(let* ((c (list 1 2)) (f (list 'lambda '() (list 'quote c)))) (setcdr (cdr c) c) (funcall f) (eq (funcall f) c))"#,
    // A procedural expander (not a constant one): its expansion is reused
    // while its definition is unchanged.
    r#"(progn (defmacro lambda-list-proc (x) (if (consp x) (list 'quote (length x)) x))
              (let ((f '(lambda (n) (list (lambda-list-proc (a b c)) (lambda-list-proc n)))))
                (list (funcall f 1) (funcall f 2))))"#,
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

/// A list called again is not compiled again; a changed one is.
#[test]
fn a_lambda_list_compiles_once_while_unchanged() {
    let mut vm = Vm::new();
    vm.eval_source("(defvar lambda-list-once (list 'lambda '(x) '(+ x 1)))").unwrap();
    // The lambda-list compilations one call makes.
    let compiles = |vm: &mut Vm| {
        let before = vm.source_cache_stats().lambda_compiles;
        vm.eval_source("(funcall lambda-list-once 1)").unwrap();
        vm.source_cache_stats().lambda_compiles - before
    };
    compiles(&mut vm);
    let again = compiles(&mut vm);
    vm.eval_source("(setcar (cdr (cdr lambda-list-once)) '(+ x 2))").unwrap();
    let changed = compiles(&mut vm);
    let after = compiles(&mut vm);
    assert_eq!((again, changed, after), (0, 1, 0));
    assert_eq!(vm.eval_source("(funcall lambda-list-once 1)").unwrap().display().unwrap(), "3");
}

/// A collection is invisible to Lisp: a list keeps its compiled code across
/// one, so its macros expand no more often than with none.
#[test]
fn a_collection_does_not_compile_a_lambda_list_again() {
    let mut vm = Vm::new();
    vm.eval_source(
        "(progn (defvar lambda-list-expansions 0)
                (defmacro lambda-list-counted () (setq lambda-list-expansions (1+ lambda-list-expansions)) 1)
                (defun lambda-list-caller () (funcall '(lambda () (lambda-list-counted)))))",
    )
    .unwrap();
    let counts = vm
        .eval_source(
            "(list (progn (lambda-list-caller) (lambda-list-caller) (lambda-list-caller) lambda-list-expansions)
                   (progn (garbage-collect) (lambda-list-caller) (lambda-list-caller) lambda-list-expansions))",
        )
        .unwrap();
    assert_eq!(counts.display().unwrap(), "(1 1)");
}
