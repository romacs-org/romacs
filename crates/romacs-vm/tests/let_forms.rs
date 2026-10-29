//! eval.c Flet and FletX check a `let' varlist as the form runs: a
//! malformed binding signals GNU's error then, after the value forms GNU
//! evaluates before it, never when the enclosing closure is made
//! (let_forms.rs). Pinned GNU 32.0.50 answers (lexical-binding t).
use romacs_vm::Vm;

#[test]
fn a_malformed_let_signals_gnus_error_when_it_runs() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(setq lexical-binding t)").unwrap();
    assert_eq!(vm.eval_source(r#"(progn (defvar lt-log nil) (list (condition-case e (funcall (lambda () (let 1))) (error e)) (condition-case e (funcall (lambda () (let* "a"))) (error e)) (condition-case e (funcall (lambda () (let [a] 1))) (error e)) (condition-case e (funcall (lambda () (let ((nil 1)) 2))) (error e)) (condition-case e (funcall (lambda () (let* ((t 1)) 2))) (error e)) (condition-case e (funcall (lambda () (let ((a 1 2)) 2))) (error e)) (condition-case e (funcall (lambda () (let (5) 2))) (error e)) (condition-case e (funcall (lambda () (let ((a . 1)) 2))) (error e)) (condition-case e (funcall (lambda () (let* ((a 1) . 3) a))) (error e)) (condition-case e (funcall (lambda () (let ((:k 1)) :k))) (error e)) (progn (setq lt-log nil) (list (condition-case e (funcall (lambda () (let ((x (setq lt-log (cons 1 lt-log))) ((a b) (setq lt-log (cons 2 lt-log))) (y (setq lt-log (cons 3 lt-log)))) 3))) (error e)) lt-log)) (progn (setq lt-log nil) (list (condition-case e (funcall (lambda () (let* ((x (setq lt-log (cons 1 lt-log))) ((a b) (setq lt-log (cons 2 lt-log))) (y (setq lt-log (cons 3 lt-log)))) 3))) (error e)) lt-log)) (progn (setq lt-log nil) (list (condition-case e (funcall (lambda () (let ((x (setq lt-log (cons 1 lt-log))) 5 (y (setq lt-log (cons 3 lt-log)))) 3))) (error e)) lt-log)) (progn (setq lt-log nil) (list (condition-case e (funcall (lambda () (let* ((x (setq lt-log (cons 1 lt-log))) (y (setq lt-log (cons 2 lt-log)) 9)) 3))) (error e)) lt-log)) (let* ((a 1) (b (+ a 1))) (list a b))))"#).unwrap().display().unwrap(),
        r#"((wrong-type-argument listp 1) (wrong-type-argument listp "a") (wrong-type-argument listp [a]) (setting-constant nil) (setting-constant t) (error "`let' bindings can have only one value-form" a 1 2) (wrong-type-argument listp 5) (wrong-type-argument listp 1) (wrong-type-argument listp ((a 1) . 3)) (setting-constant :k) ((wrong-type-argument symbolp (a b)) (3 2 1)) ((wrong-type-argument symbolp (a b)) (2 1)) ((wrong-type-argument listp 5) (1)) ((error "`let' bindings can have only one value-form" y (setq lt-log (cons 2 lt-log)) 9) (1)) (1 2))"#);
}

/// The enclosing definition is made; the error comes from calling it
/// (eval-tests.el's top-level form [8] defines such tests).
#[test]
fn a_closure_holding_a_malformed_let_can_be_made() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source("(progn (defalias 'lf-bad (lambda () (let 1))) (list (functionp 'lf-bad) (condition-case e (lf-bad) (error e))))").unwrap().display().unwrap(),
        "(t (wrong-type-argument listp 1))");
}
