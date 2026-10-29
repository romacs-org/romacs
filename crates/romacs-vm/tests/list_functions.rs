//! GNU `(lambda ...)' list functions: `defalias' stores the list as given
//! (data.c Fdefalias -> Ffset) and funcall_general calls it through
//! funcall_lambda with dynamic binding (eval.c:3172). Expected answers are
//! pinned GNU 32.0.50's (`emacs -Q --batch`).
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap_or_else(|e| format!("VM-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[test]
fn a_lambda_list_is_stored_and_called_with_dynamic_binding() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    for (source, expected) in [
        ("(defalias 'lf-a '(lambda (x) (interactive \"P\") (list x lf-dyn)))", "lf-a"),
        ("(defvar lf-dyn 1)", "lf-dyn"),
        ("(funcall 'lf-a 2)", "(2 1)"),
        ("(let ((lf-dyn 5)) (lf-a 3))", "(3 5)"),
        ("(eq (symbol-function 'lf-a) (symbol-function 'lf-a))", "t"),
        ("(car (symbol-function 'lf-a))", "lambda"),
        ("(apply '(lambda (&rest r) r) 1 '(2 3))", "(1 2 3)"),
        ("(mapcar '(lambda (x) (* x x)) '(1 2 3))", "(1 4 9)"),
        // The list body is dynamic code even inside lexical code.
        ("(funcall (let ((f '(lambda () lf-dyn))) f))", "1"),
        ("(condition-case e (funcall '(lambda)) (error e))", "(invalid-function (lambda))"),
        ("(condition-case e (funcall '(lambda . 5)) (error e))", "(invalid-function (lambda . 5))"),
    ] {
        assert_eq!(eval(&mut vm, source), expected, "{source}");
    }
}

/// funcall_lambda's argument walk with the list as the function datum:
/// a bad arglist is invalid-function, a wrong count
/// wrong-number-of-arguments (eval-tests.el eval-bad-specbind).
#[test]
fn a_lambda_list_with_a_bad_arglist_or_count_signals_as_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(r#"(list (condition-case e (funcall '(lambda ((a b) 3.15) 84) 5 4) (error e)) (condition-case e (eval '(funcall '(lambda ((a b) 3.15) 84) 5 4)) (error e)) (condition-case e (funcall '(lambda (x) x)) (error e)) (condition-case e (funcall '(lambda (x) x) 1 2) (error e)) (condition-case e (funcall '(lambda (&rest) 1)) (error e)) (condition-case e (funcall '(lambda (&optional &optional) 1)) (error e)) (condition-case e (funcall '(lambda (a . b) 1) 1) (error e)) (condition-case e (apply '(lambda (a 5) 1) '(1)) (error e)) (funcall '(lambda (a &optional b &rest c) (list a b c)) 1 2 3 4) (condition-case e (mapcar '(lambda (&rest &optional) 1) '(1)) (error e)))"#).unwrap().display().unwrap(),
        r#"((invalid-function (lambda ((a b) 3.15) 84)) (invalid-function (lambda ((a b) 3.15) 84)) (wrong-number-of-arguments (lambda (x) x) 0) (wrong-number-of-arguments (lambda (x) x) 2) (invalid-function (lambda (&rest) 1)) (invalid-function (lambda (&optional &optional) 1)) (invalid-function (lambda (a . b) 1)) (invalid-function (lambda (a 5) 1)) (1 2 (3 4)) (invalid-function (lambda (&rest &optional) 1)))"#);
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

/// funcall_lambda checks, takes the argument and binds one parameter at a
/// time, so a bad parameter signals after the earlier ones are bound (their
/// watchers see the let and the unlet), and binding nil, t, a keyword or a
/// protected variable is setting-constant.
#[test]
fn a_lambda_list_binds_each_parameter_before_checking_the_next() {
    watched(&[
        ("(funcall '(lambda (nil 5) 1) 2 3)", "((setting-constant nil) nil)"),
        ("(funcall '(lambda (ww-x nil) 1) 2 3)", "((setting-constant nil) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (ww-x 5) 1) 2 3)", "((invalid-function (lambda (ww-x 5) 1)) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (ww-x y) 1) 2)", "((wrong-number-of-arguments (lambda (ww-x y) 1) 1) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (ww-x . y) 1) 2)", "((invalid-function (lambda (ww-x . y) 1)) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (ww-x &rest) 1) 2)", "((invalid-function (lambda (ww-x &rest) 1)) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (ww-x) 1) 2 3)", "((wrong-number-of-arguments (lambda (ww-x) 1) 2) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (&rest ww-x) 1) 2 3)", "(1 ((let (2 3)) (unlet 0)))"),
        ("(funcall '(lambda (&rest ww-x &optional) 1) 2 3)", "((invalid-function (lambda (&rest ww-x &optional) 1)) ((let (2 3)) (unlet 0)))"),
        ("(funcall '(lambda (:k) 1) 2)", "((setting-constant :k) nil)"),
        ("(funcall '(lambda (t) 1) 2)", "((setting-constant t) nil)"),
        ("(funcall '(lambda (ww-x &optional y nil) 1) 2)", "((setting-constant nil) ((let 2) (unlet 0)))"),
        ("(funcall '(lambda (most-positive-fixnum 5) 1) 2)", "((setting-constant most-positive-fixnum) nil)"),
        ("(funcall '(lambda (enable-multibyte-characters 5) 1) 2)", "((setting-constant enable-multibyte-characters) nil)"),
        ("(funcall '(lambda (ww-x most-positive-fixnum) 1) 2 3 4)", "((setting-constant most-positive-fixnum) ((let 2) (unlet 0)))"),
    ]);
}

/// The failed walk binds the caller's own argument objects and signals with
/// the function object itself: a watcher that mutates its new value
/// changes the caller's argument, and the error datum is `eq' to the
/// function, also across a collection a watcher runs. Pinned GNU 32.0.50
/// answers.
#[test]
fn a_lambda_list_binds_and_signals_with_the_callers_objects() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(progn (defvar ww-y nil) (add-variable-watcher 'ww-y (lambda (_s n o _w) (when (eq o 'let) (if (consp n) (setcar n 'changed) (aset n 0 'changed))))) (defvar ww-z nil) (add-variable-watcher 'ww-z (lambda (_s n o _w) (when (eq o 'let) (garbage-collect) (setcar n (list 'collected))))))").unwrap();
    for (form, expected) in [
        ("(let ((arg (list 1 2))) (list (condition-case e (funcall '(lambda (ww-y 5) 1) arg) (error (car e))) arg))", "(invalid-function (changed 2))"),
        ("(let ((arg (vector 1 2))) (list (condition-case e (funcall '(lambda (ww-y nil) 1) arg 3) (error (car e))) arg))", "(setting-constant [changed 2])"),
        ("(let ((arg (list 1 2))) (list (condition-case e (funcall '(lambda (&rest ww-y) (car ww-y)) arg) (error (car e))) arg))", "(changed (1 2))"),
        ("(let* ((fn (list 'lambda '(x 5) 1)) (err (condition-case e (funcall fn 1) (error e)))) (list (car err) (eq (car (cdr err)) fn)))", "(invalid-function t)"),
        ("(let* ((fn (list 'lambda '(x) 1)) (err (condition-case e (funcall fn) (error e)))) (list (car err) (eq (car (cdr err)) fn)))", "(wrong-number-of-arguments t)"),
        ("(let* ((fn (list 'lambda '(x) 1)) (err (condition-case e (apply fn '(1 2)) (error e)))) (list (car err) (eq (car (cdr err)) fn)))", "(wrong-number-of-arguments t)"),
        ("(let* ((arg (list 1 2)) (fn (list 'lambda '(&rest ww-z &optional) 1)) (err (condition-case e (funcall fn arg) (error e)))) (list (car err) (eq (car (cdr err)) fn) arg))", "(invalid-function t (1 2))"),
    ] {
        let source = format!("(eval '{form} t)");
        assert_eq!(vm.eval_source(&source).map(|v| v.display().unwrap()), Ok(expected.to_string()), "{form}");
    }
}
