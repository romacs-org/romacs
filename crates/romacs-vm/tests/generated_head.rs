//! A form whose head is a symbol no name reaches, uninterned or of another
//! obarray, evaluates as GNU's eval_sub does: through the symbol's function
//! cell. The expected answers are pinned GNU's.
use romacs_vm::Vm;

fn eval(source: &str) -> String {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let wrapped = format!("(condition-case e {source} (error e))");
    vm.eval_source(&wrapped).unwrap().display().unwrap()
}

const CALLED: &str = "(fset u (lambda (&rest a) (cons 'called a)))";

fn with_u(body: &str) -> String {
    eval(&format!("(let ((u (make-symbol \"u\"))) {CALLED} {body})"))
}

#[test]
fn a_call_through_the_function_cell() {
    assert_eq!(with_u("(eval (list u) t)"), "(called)");
    assert_eq!(with_u("(eval (list u) nil)"), "(called)");
    assert_eq!(with_u("(eval (list u 1 '(+ 1 1)) t)"), "(called 1 2)");
    assert_eq!(with_u("(eval (list 'list (list u 3)) t)"), "((called 3))");
}

#[test]
fn inside_a_function_body() {
    assert_eq!(with_u("(funcall (list 'lambda nil (list u 4)))"), "(called 4)");
    assert_eq!(with_u("(funcall (eval (list 'lambda nil (list u 5)) t))"), "(called 5)");
}

#[test]
fn the_cell_is_read_when_the_call_is_reached() {
    assert_eq!(
        with_u("(eval (list 'progn (list 'fset (list 'quote u) '(lambda () 'new)) (list u)) t)"),
        "new",
    );
}

#[test]
fn a_macro_expands() {
    let form = "(let ((m (make-symbol \"m\")))
                  (fset m (cons 'macro (lambda (x) (list 'quote (list 'expanded x)))))
                  (eval (list m 9) t))";
    assert_eq!(eval(form), "(expanded 9)");
}

#[test]
fn an_undefined_one_signals_void_function() {
    assert_eq!(eval("(let ((v (make-symbol \"v\"))) (eval (list v) t))"), "(void-function v)");
}

#[test]
fn a_symbol_of_another_obarray() {
    let form = "(let* ((ob (obarray-make)) (s (intern \"s\" ob)))
                  (fset s (lambda () 'other-obarray))
                  (eval (list s) t))";
    assert_eq!(eval(form), "other-obarray");
}

#[test]
fn a_special_form_dispatches() {
    assert_eq!(eval("(let ((u (make-symbol \"u\"))) (fset u (symbol-function 'quote)) (eval (list u 'abc) t))"), "abc");
    assert_eq!(eval("(let ((u (make-symbol \"u\"))) (fset u 'quote) (eval (list u 'abc) t))"), "abc");
}

const MACRO: &str = "(cons 'macro (lambda (x) (list 'quote (list 'expanded x))))";
const BROKEN: &str = "(cons 'macro (lambda () (/ 1 0)))";

#[test]
fn a_macro_installed_by_an_earlier_form_expands_when_reached() {
    let form = format!(
        "(let ((m (make-symbol \"m\")))
           (eval (list 'progn (list 'fset (list 'quote m) (list 'quote {MACRO})) (list m 9)) t))");
    assert_eq!(eval(&form), "(expanded 9)");
}

#[test]
fn an_expander_that_signals_waits_for_its_reach() {
    let broken = |body: &str| format!("(let ((m (make-symbol \"m\"))) (fset m {BROKEN}) {body})");
    assert_eq!(eval(&broken("(eval (list 'if nil (list m) 42) t)")), "42");
    assert_eq!(eval(&broken("(eval (list 'condition-case nil (list m) '(arith-error 'caught)) t)")), "caught");
    assert_eq!(eval(&broken("(eval (list m) t)")), "(arith-error)");
}

#[test]
fn void_function_comes_before_the_arguments() {
    assert_eq!(eval("(let ((v (make-symbol \"v\"))) (eval (list v 'missing-arg) t))"), "(void-function v)");
}

#[test]
fn a_special_form_is_known_by_identity_not_spelling() {
    let form = "(let ((u (make-symbol \"u\")) (q (make-symbol \"quote\")))
                  (fset q (lambda (x) (list 'called x)))
                  (fset u q)
                  (eval (list u '(+ 1 2)) t))";
    assert_eq!(eval(form), "(called 3)");
}

#[test]
fn a_function_defined_after_a_void_call_counts_on_the_retry() {
    let form = "(let ((v (make-symbol \"v\")) (result nil))
                  (setq result
                        (eval (list 'let '((n 0) (out nil))
                                    (list 'while '(and (< n 2) (not out))
                                          '(setq n (1+ n))
                                          (list 'condition-case nil
                                                (list 'setq 'out (list v))
                                                (list 'void-function
                                                      (list 'fset (list 'quote v) '(lambda () 9)))))
                                    'out)
                              t))
                  result)";
    assert_eq!(eval(form), "9");
}

/// An emptied cell is void, as one never set: named heads and these alike.
#[test]
fn an_emptied_function_cell_is_void() {
    assert_eq!(eval("(let ((v (make-symbol \"v\"))) (fmakunbound v) (eval (list v 'missing-arg) t))"), "(void-function v)");
    assert_eq!(eval("(let ((v (make-symbol \"v\"))) (fset v nil) (eval (list v 'missing-arg) t))"), "(void-function v)");
    assert_eq!(eval("(progn (defalias 'named-x #'ignore) (fmakunbound 'named-x) (eval '(named-x missing-arg) t))"), "(void-function named-x)");
    assert_eq!(eval("(progn (defalias 'named-y #'ignore) (fset 'named-y nil) (eval '(named-y missing-arg) t))"), "(void-function named-y)");
}

#[test]
fn a_macro_installed_over_an_emptied_cell_expands_when_reached() {
    let form = format!(
        "(let ((m (make-symbol \"m\")))
           (fmakunbound m)
           (eval (list 'progn (list 'fset (list 'quote m) (list 'quote {MACRO})) (list m 9)) t))");
    assert_eq!(eval(&form), "(expanded 9)");
}
