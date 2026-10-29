//! GNU checks a special form's shape as the form runs (eval.c Flet, FletX,
//! Fsetq, Fcond, Fcondition_case, Fdefvar, Fdefconst): a malformed one
//! signals GNU's error then, after what GNU evaluates before it, never
//! when the enclosing closure is made (let_forms.rs). Pinned GNU 32.0.50
//! answers (lexical-binding t).
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

/// Each FORM as the body of a closure that is made, then called.
#[test]
fn malformed_special_forms_signal_gnus_error_when_they_run() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for (form, expected) in [
        (r#"(condition-case (a b) 1 (error 2))"#, r#"(wrong-type-argument symbolp (a b))"#),
        (r#"(setq a)"#, r#"(wrong-number-of-arguments setq 1)"#),
        (r#"(setq 1 2)"#, r#"(wrong-type-argument symbolp 1)"#),
        (r#"(setq a 1 b)"#, r#"(wrong-number-of-arguments setq 3)"#),
        (r#"(cond 1)"#, r#"(wrong-type-argument listp 1)"#),
        (r#"(cond (1 . 2))"#, r#"nil"#),
        (r#"(if)"#, r#"(wrong-number-of-arguments if 0)"#),
        (r#"(while)"#, r#"(wrong-number-of-arguments while 0)"#),
        (r#"(catch)"#, r#"(wrong-number-of-arguments catch 0)"#),
        (r#"(unwind-protect)"#, r#"(wrong-number-of-arguments unwind-protect 0)"#),
        (r#"(quote 1 2)"#, r#"(wrong-number-of-arguments quote 2)"#),
        (r#"(function 1 2)"#, r#"(wrong-number-of-arguments function 2)"#),
        (r#"(prog1)"#, r#"(wrong-number-of-arguments prog1 0)"#),
        (r#"(let)"#, r#"(wrong-number-of-arguments let 0)"#),
        (r#"(let*)"#, r#"(wrong-number-of-arguments let* 0)"#),
        (r#"(condition-case e 1 (2 3))"#, r#"(error "Invalid condition handler: (2 3)")"#),
        (r#"(condition-case e 1 . 2)"#, r#"(wrong-type-argument listp 2)"#),
        (r#"(defvar)"#, r#"(wrong-number-of-arguments defvar 0)"#),
        (r#"(defvar 1 2)"#, r#"(wrong-type-argument symbolp 1)"#),
        (r#"(defconst a)"#, r#"(wrong-number-of-arguments defconst 1)"#),
        (r#"(and . 1)"#, r#"(wrong-type-argument listp 1)"#),
        (r#"(or 1 . 2)"#, r#"(wrong-type-argument listp 2)"#),
        (r#"(progn . 3)"#, r#"(wrong-type-argument listp 3)"#),
        (r#"(save-excursion . 1)"#, r#"(wrong-type-argument listp 1)"#),
        (r#"(setq t 1)"#, r#"(setting-constant t)"#),
        (r#"(setq nil 2)"#, r#"(setting-constant nil)"#),
        (r#"(let ((a 1)) (setq a))"#, r#"(wrong-number-of-arguments setq 1)"#),
        (r#"(cond (1 2) . 3)"#, r#"(wrong-type-argument listp 3)"#),
        (r#"(defconst 1 2)"#, r#"(wrong-type-argument symbolp 1)"#),
        (r#"(defvar 1)"#, r#"(wrong-type-argument symbolp 1)"#),
        (r#"(defvar "a" 1)"#, r#"(wrong-type-argument symbolp "a")"#),
        (r#"(setq :k 1)"#, r#"(setting-constant :k)"#),
        (r#"(setq :k :k)"#, r#":k"#),
        (r#"(setq a 1 :k :k b 2)"#, r#"2"#),
        (r#"(setq a 1 2 3)"#, r#"(wrong-type-argument symbolp 2)"#),
        (r#"(cond (1 2 . 3))"#, r#"2"#),
        (r#"(cond (nil 1) 5 (t 2))"#, r#"(wrong-type-argument listp 5)"#),
        (r#"(condition-case e 1 nil (error 2))"#, r#"1"#),
        (r#"(condition-case e 1 3)"#, r#"(error "Invalid condition handler: 3")"#),
        (r#"(condition-case e 1 ("x" 2))"#, r#"(error "Invalid condition handler: (x 2)")"#),
        (r#"(condition-case e 1 ((error) 2))"#, r#"1"#),
        (r#"(condition-case 5 1)"#, r#"(wrong-type-argument symbolp 5)"#),
    ] {
        let source = format!("(condition-case e (funcall (eval (list 'lambda nil '{form}) t)) (error e))");
        assert_eq!(vm.eval_source(&source).map(|v| v.display().unwrap()), Ok(expected.to_owned()), "{form}");
    }
}

/// ROWS of (FORM EXPECTED): FORM run as `(eval 'FORM t)' with a watcher
/// logging (OPERATION NEWVAL) for special variable ww-x; EXPECTED is
/// pinned GNU's (RESULT-OR-ERROR LOG).
fn watched(rows: &[(&str, &str)]) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(progn (defvar ww-log nil) (defvar ww-x 0) (add-variable-watcher 'ww-x (lambda (_s n o _w) (setq ww-log (cons (list o n) ww-log)))))").unwrap();
    for (form, expected) in rows {
        let source = format!("(progn (setq ww-log nil) (list (condition-case e (eval '{form} t) (error e)) (reverse ww-log)))");
        assert_eq!(vm.eval_source(&source).map(|v| v.display().unwrap()), Ok(expected.to_string()), "{form}");
    }
}

/// xsignal conses its data afresh: a handler that mutates one error's data
/// changes no later error from the same code.
#[test]
fn a_malformed_form_signals_fresh_error_data_each_time() {
    for form in ["(let 1)", "(setq 1 2)", "(cond 1)"] {
        watched(&[(&format!("(let ((f (lambda () {form}))) (let ((e1 (condition-case e (funcall f) (error e)))) (setcar (cdr e1) 'changed) (condition-case e (funcall f) (error e))))"),
            if form == "(setq 1 2)" { "((wrong-type-argument symbolp 1) nil)" } else { "((wrong-type-argument listp 1) nil)" })]);
    }
}

/// Flet evaluates every value form, then binds in turn: the bindings before
/// a bad variable are made, and unwound by its signal; FletX binds as it
/// goes.
#[test]
fn a_malformed_let_makes_and_unwinds_the_bindings_before_it() {
    watched(&[
        ("(let ((ww-x 1) (nil 2)) 3)", "((setting-constant nil) ((let 1) (unlet 0)))"),
        ("(let ((ww-x 1) (2 3)) 4)", "((wrong-type-argument symbolp 2) ((let 1) (unlet 0)))"),
        ("(let ((ww-x (progn (setq ww-log (cons 'init ww-log)) 1)) (nil (setq ww-log (cons 'init2 ww-log)))) 3)", "((setting-constant nil) (init init2 (let 1) (unlet 0)))"),
        ("(let* ((ww-x 1) (nil 2)) 3)", "((setting-constant nil) ((let 1) (unlet 0)))"),
        ("(let ((:k 1)) :k)", "((setting-constant :k) nil)"),
    ]);
}
