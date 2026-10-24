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
