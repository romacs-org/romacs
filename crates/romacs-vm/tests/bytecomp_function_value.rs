//! VM2-BYTECOMP1: under the loadup bootstrap, `byte-compile' of a function
//! value (VM2 code, compiled when defined) returns it, and a `(lambda ...)'
//! list is evaluated as GNU compiles it. The advice is in place before
//! bytecomp.el loads, so its own load-time calls see it. cc-defs.el's
//! `(if (byte-code-function-p f) f (byte-compile f))' is the consumer.
use romacs_vm::Vm;
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

fn query(vm: &mut Vm, form: &str) -> String {
    let source = format!("(condition-case e (prin1-to-string {form}) (error (format \"signal %S\" e)))");
    match vm.eval_source(&source).and_then(|v| v.display()) {
        Ok(text) => serde_json::from_str::<String>(&text).unwrap_or(text),
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn byte_compile_returns_an_already_compiled_function() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    for (form, expected) in [
        ("(list (featurep 'bytecomp) (and (advice-member-p #'romacs-bytecomp--function-value 'byte-compile) t))", "(nil t)"),
        ("(progn (defun bfv-f (x) (+ x 1)) (eq (byte-compile 'bfv-f) (symbol-function 'bfv-f)))", "t"),
        ("(list (featurep 'bytecomp) (and (advice-member-p #'romacs-bytecomp--function-value 'byte-compile) t))", "(t t)"),
        ("(let ((f (lambda (x) (* x 2)))) (eq (byte-compile f) f))", "t"),
        ("(funcall (byte-compile '(lambda (x) (* x 3))) 4)", "12"),
        ("(progn (defmacro bfv-m (x) x) (car (byte-compile 'bfv-m)))", "macro"),
        ("(progn (defalias 'bfv-l '(lambda (x) (* x 5))) (byte-compile 'bfv-l) (list (interpreted-function-p (symbol-function 'bfv-l)) (bfv-l 2)))", "(t 10)"),
    ] {
        assert_eq!(query(&mut vm, form), expected, "{form}");
    }
}

/// A `(lambda ...)' list whose body bytecomp's front end refuses gives
/// GNU's diagnostic string and no function, as an argument (lexical) or as
/// a symbol's definition (dynamic, left unchanged). Pinned GNU 32.0.50
/// answers outside any file.
#[test]
fn byte_compile_of_a_malformed_lambda_list_returns_gnus_diagnostic() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    for (form, expected) in [
        ("(let ((lexical-binding t)) (byte-compile '(lambda () (let 1))))", r#"" Error: Wrong type argument: listp, 1""#),
        ("(let ((lexical-binding t)) (byte-compile '(lambda () (cond 1))))", r#"" Error: Wrong type argument: listp, 1""#),
        ("(progn (defalias 'bfv-bad '(lambda () (let 1))) (list (byte-compile 'bfv-bad) (symbol-function 'bfv-bad)))", r#"(" Error: Wrong type argument: listp, 1" (lambda nil (let 1)))"#),
        ("(let ((lexical-binding t)) (funcall (byte-compile '(lambda (x) (* x 3))) 4))", "12"),
    ] {
        assert_eq!(query(&mut vm, form), expected, "{form}");
    }
}

/// bytecomp expands a lambda list's macros once, in its front end; the
/// function evaluates that expansion rather than expanding again, as an
/// argument, a closure inside it, or a symbol's definition. Pinned GNU
/// 32.0.50 answers.
#[test]
fn byte_compile_of_a_lambda_list_expands_its_macros_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    query(&mut vm, "(progn (defvar bfv-count 0) (defmacro bfv-count-m () (setq bfv-count (1+ bfv-count)) 1))");
    for (form, expected) in [
        ("(let ((lexical-binding t)) (setq bfv-count 0) (list (funcall (byte-compile '(lambda () (bfv-count-m)))) bfv-count))", "(1 1)"),
        ("(let ((lexical-binding t)) (setq bfv-count 0) (list (funcall (funcall (byte-compile '(lambda (x) (lambda () (+ (bfv-count-m) x)))) 5)) bfv-count))", "(6 1)"),
        ("(progn (setq bfv-count 0) (defalias 'bfv-s '(lambda () (bfv-count-m))) (byte-compile 'bfv-s) (list (bfv-s) (bfv-s) bfv-count))", "(1 1 1)"),
    ] {
        assert_eq!(query(&mut vm, form), expected, "{form}");
    }
}
