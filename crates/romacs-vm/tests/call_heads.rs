//! A call form whose head is not a symbol. GNU `eval_sub` (src/eval.c) passes
//! such a head through `function`: a lambda becomes a closure applied to the
//! evaluated arguments, and anything else signals `(invalid-function HEAD)`
//! before any argument is evaluated. Expected values were printed by live
//! GNU 32.0.50 (`emacs -Q --batch`, lexical-binding t).
use romacs_vm::Vm;

fn eval(source: &str) -> String {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    match vm.eval_source(source) {
        Ok(v) => v.display().unwrap_or_else(|e| format!("DISPLAY-ERROR {e}")),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[test]
fn non_symbol_head_signals_invalid_function_before_arguments() {
    assert_eq!(
        eval("(let ((x 0)) (list (condition-case e ((or a b) (setq x 1)) (error e)) x))"),
        "((invalid-function (or a b)) 0)"
    );
    assert_eq!(eval("(condition-case e (1 2) (error e))"), "(invalid-function 1)");
    assert_eq!(eval("(condition-case e (\"s\") (error e))"), "(invalid-function \"s\")");
}

#[test]
fn lambda_head_applies_a_closure() {
    assert_eq!(eval("((lambda (x) (* x 2)) 21)"), "42");
    assert_eq!(eval("(let ((y 5)) ((lambda (x) (+ x y)) 1))"), "6");
}

// The bootstrap shape: macroexp.el defines functions whose bodies use `pcase`
// before pcase.el is loaded. GNU's interpreter defines them; so must we. The
// call itself resolves the missing head before evaluating pattern operands.
// Exact condition data is covered by the pinned head_order corpus.
#[test]
fn defun_with_undefined_pattern_macro_still_defines() {
    assert_eq!(
        eval("(list (defun pc--f (x) (pc--undefined x ((or 1 2) 'a))) (fboundp 'pc--f) (condition-case nil (pc--f 1) (error 'signalled)))"),
        "(pc--f t signalled)"
    );
}

// A dotted call form: eval_sub checks the argument list before the head, then
// a function call signals on the final atom of its argument list.
#[test]
fn dotted_call_forms_signal_as_gnu() {
    assert_eq!(eval("(condition-case e (car . 2) (error e))"), "(wrong-type-argument listp 2)");
    assert_eq!(eval("(condition-case e (car 1 . 2) (error e))"), "(wrong-type-argument listp 2)");
    assert_eq!(eval("(condition-case e (+ 1 2 . 3) (error e))"), "(wrong-type-argument listp 3)");
    assert_eq!(eval("(condition-case e ((lambda (&rest x) x) 1 . 3) (error e))"), "(wrong-type-argument listp 3)");
    assert_eq!(eval("(condition-case e (1 2 . 3) (error e))"), "(invalid-function 1)");
}

// GNU's byte code (src/bytecode.c `Bcall`) evaluates the arguments, then
// reads the function cell: a redefinition made by an argument is the function
// called, and a void head signals only after the arguments ran. Pinned GNU
// 32.0.50, byte-compiled: `(new 2)` and `((void-function undefined-fn) 1)`.
// A top-level form whose head has no function when it is compiled keeps the
// loader's order instead (GNU loads source through its interpreter): the
// head is named before any argument runs. A redefinition of an unsealed
// function takes effect at the next call of a compiled site, including one
// lowered to a primitive kernel.
#[test]
fn named_call_reads_the_cell_after_its_arguments() {
    assert_eq!(
        eval("(progn (defun ch-f (_ a) (list 'old a)) (ch-f (progn (fset 'ch-f (lambda (_ a) (list 'new a))) 1) 2))"),
        "(new 2)"
    );
    assert_eq!(
        eval("(progn (defvar zz 0) (defun ch-body () (ch-undefined (progn (setq zz 1) 2))) (list (condition-case e (ch-body) (error e)) zz))"),
        "((void-function ch-undefined) 1)"
    );
    assert_eq!(
        eval("(let ((zz 0)) (list (condition-case e (ch-undefined (progn (setq zz 1) 2)) (error e)) zz))"),
        "((void-function ch-undefined) 0)"
    );
    // The loader-order check is the primitives themselves: neither a redefined
    // `fboundp' or `signal' nor an alias of `symbol-function' to a special form
    // changes it.
    assert_eq!(eval("(progn (fset 'fboundp (lambda (_) nil)) (fset 'ch-late (lambda (x) x)) (ch-late 42))"), "42");
    assert_eq!(
        eval("(progn (fset 'symbol-function 'quote) (let ((seen 0)) (list (condition-case e (ch-missing (setq seen 1)) (error e)) seen)))"),
        "((void-function ch-missing) 0)"
    );
    assert_eq!(
        eval("(progn (fset 'signal (lambda (&rest _) nil)) (let ((seen 0)) (list (condition-case e (ch-missing (setq seen 1)) (error e)) seen)))"),
        "((void-function ch-missing) 0)"
    );
    assert_eq!(
        eval("(progn (defun ch-g (x) x) (defun ch-run (x) (ch-g x)) (ch-run 1) (ch-run 2) \
              (fset 'ch-g (lambda (x) (* x 10))) (list (ch-run 3) (progn (defun ch-g (x) (1+ x)) (ch-run 4))))"),
        "(30 5)"
    );
    assert_eq!(
        eval("(progn (defun ch-pg (l) (plist-get l 'a)) (ch-pg '(a 1)) (ch-pg '(a 2)) \
              (fset 'plist-get (lambda (_l _k) 'mine)) (ch-pg '(a 3)))"),
        "mine"
    );
}

/// GNU's `funcall_lambda` knows only `&optional` and `&rest`: any other `&'
/// name is an ordinary parameter, as a `defun` with `&key` finds.
#[test]
fn other_ampersand_names_are_ordinary_parameters() {
    assert_eq!(eval("(progn (defun ch-key (a &key b) (list a &key b)) (ch-key 1 2 3))"), "(1 2 3)");
    assert_eq!(eval("((lambda (&key) &key) 7)"), "7");
}
