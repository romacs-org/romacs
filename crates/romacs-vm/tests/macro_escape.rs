//! What a macro's body signals, throws or quits while it expands meets the
//! handlers GNU's would, against pinned GNU batch. `eval' interprets, so a
//! macro call expands when evaluation reaches it, inside the handlers
//! around it there; `load' expands each top-level form first
//! (macroexp.el `internal-macroexpand-for-load'), so an error there is
//! "Eager macro-expansion failure" and a quit passes as it is. Romacs
//! expands `eval''s macro calls as it compiles and defers only an
//! expansion that fails to its reach (DIVERGENCES.md VM2-EVAL-EXPAND1).
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const MACROS: &str = r#"(progn
  (defvar rt-log nil)
  (defmacro rt-m () (error "boom"))
  (defmacro rt-t () (throw 'rt-tag 7))
  (defmacro rt-u () (unwind-protect (error "uw") (push 'cleanup rt-log)))
  (defmacro rt-s () (signal 'quit nil))
  (defmacro rt-a () (/ 1 0))
  (defmacro rt-hb () (catch 'done (handler-bind-1 (lambda () (error "boom")) 'error (lambda (_) (throw 'done 7))))))"#;

/// Forms `eval' runs lexically, each one's value or its escape printed.
const EVALUATED: [&str; 9] = [
    r#"(condition-case e (rt-m) (error (list 'caught e)))"#,
    r#"(progn (defun rt-f () (rt-m)) (condition-case e (rt-f) (error (list 'caught e))))"#,
    r#"(condition-case e (progn (defmacro rt-m2 () (error "boom2")) (rt-m2)) (error (list 'caught e)))"#,
    r#"(catch 'rt-tag (rt-t))"#,
    r#"(progn (defun rt-g () (rt-t)) (catch 'rt-tag (rt-g)))"#,
    r#"(condition-case e (rt-s) (quit (list 'quit-caught e)))"#,
    r#"(progn (defun rt-q () (rt-s)) (condition-case e (rt-q) (quit (list 'quit-caught e))))"#,
    r#"(condition-case e (macroexpand '(rt-m)) (error (list 'caught e)))"#,
    r#"(condition-case e (macroexpand-all '(progn (rt-m))) (error (list 'caught e)))"#,
];

/// Forms evaluated as they are, each around an `eval' of its own: what the
/// eval'd form's own handlers catch, no handler around the `eval' sees, and
/// a deferred call's expansion defers the calls inside it in turn.
const AROUND: [&str; 13] = [
    r#"(catch 'rt-tag (list 'outer (eval '(catch 'rt-tag (rt-t)) t)))"#,
    r#"(progn (setq rt-log nil) (list (catch 'rt-tag (eval '(unwind-protect (rt-t) (push 'cleanup rt-log)) t)) rt-log))"#,
    r#"(progn (setq rt-log nil) (list (handler-bind ((error (lambda (e) (push e rt-log)))) (eval '(condition-case nil (rt-m) (error 'inner)) t)) rt-log))"#,
    r#"(eval '(progn (defmacro rt-new () '(condition-case nil (rt-m) (error 'inner))) (rt-new)) t)"#,
    // No `signal-hook-function' call for a throw the form's catch takes.
    r#"(progn (setq rt-log nil) (list (let ((signal-hook-function (lambda (c _) (push c rt-log)))) (eval '(catch 'rt-tag (rt-t)) t)) rt-log))"#,
    // The hook hears an error a handler-bind inside the macro leaves.
    r#"(progn (setq rt-log nil) (list (let ((signal-hook-function (lambda (c _) (push c rt-log)))) (eval '(rt-hb) t)) rt-log))"#,
    // A call whose function is undefined when reached signals
    // void-function before any argument runs, alone, after what ran
    // before it, and inside the form's own handler; one a form before it
    // defines runs with its arguments.
    r#"(progn (setq rt-log nil) (list (condition-case e (eval '(rt-undefined (push 'arg rt-log)) t) (error e)) rt-log))"#,
    r#"(progn (setq rt-log nil) (list (condition-case e (eval '(progn (push 'before rt-log) (rt-undefined (push 'arg rt-log))) t) (error e)) rt-log))"#,
    r#"(progn (setq rt-log nil) (list (eval '(condition-case e (rt-undefined (push 'arg rt-log)) (void-function (list 'inner e))) t) rt-log))"#,
    r#"(progn (setq rt-log nil) (list (eval '(progn (defun rt-late (x) (list 'late x)) (rt-late (push 'arg rt-log))) t) rt-log))"#,
    // An argument's failing expansion is never reached past the void head;
    // the call, reached again once defined, runs.
    r#"(condition-case e (eval '(rt-undefined (condition-case nil (rt-m) (error 'inner))) t) (error e))"#,
    r#"(eval '(let ((r nil)) (dotimes (_ 2) (push (condition-case nil (rt-later) (void-function (defalias 'rt-later (lambda () 'ok)) 'void)) r)) r) t)"#,
    // No debugger for an error the form's own handler catches.
    r#"(progn (setq rt-log nil) (list (let ((debug-on-error t) (debugger (lambda (&rest _) (push 'debugged rt-log) nil))) (eval '(condition-case nil (rt-m) (error 'caught)) t)) rt-log))"#,
];

/// Files `load' reads after the macros, and the variable each sets.
const LOADED: [&str; 5] = [
    "(setq rt-r (condition-case e (rt-m) (error (list 'caught e))))",
    "(defun rt-f () (rt-m))\n(setq rt-r 'defined)",
    "(setq rt-r (catch 'rt-tag (rt-t)))",
    "(setq rt-r (condition-case e (rt-s) (quit (list 'quit-caught e))))",
    "(setq rt-r (rt-a))",
];

fn escape(form: &str) -> String {
    format!("(prin1-to-string (condition-case e {form} (error (list 'outer e)) (no-catch (list 'outer-throw e)) (quit (list 'outer-quit e))))")
}

#[test]
fn macro_expansion_escapes_reach_gnus_handlers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    vm.eval_source(MACROS).unwrap();
    let scratch = std::env::temp_dir().join(format!("macro-escape-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut cases: Vec<String> = EVALUATED.iter().map(|form| escape(&format!("(eval '{form} t)"))).collect();
    cases.extend(AROUND.iter().map(|form| escape(form)));
    for (index, body) in LOADED.iter().enumerate() {
        let file = scratch.join(format!("case-{index}.el"));
        std::fs::write(&file, format!(";;; -*- lexical-binding: t -*-\n{body}\n")).unwrap();
        // The loader's own handler stops an expansion's error before any
        // handler around the load sees it (macroexp.el
        // internal-macroexpand-for-load).
        let load = format!("(progn (setq rt-log nil) (list (handler-bind ((arith-error (lambda (_) (push 'outer-bind rt-log)))) (condition-case e (load {:?} nil t t) (error (list 'load-error e)) (quit (list 'load-quit e)))) (prog1 (bound-and-true-p rt-r) (setq rt-r nil)) rt-log))", file.to_str().unwrap());
        cases.push(escape(&load));
    }
    // The loader's clause is `(debug error)': the debugger sees the
    // expansion's error before it is wrapped.
    let file = scratch.join("case-debug.el");
    std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(setq rt-r (rt-a))\n").unwrap();
    cases.push(escape(&format!("(progn (setq rt-log nil) (list (let ((debug-on-error t) (debugger (lambda (&rest a) (push (cons 'debugged (cdr a)) rt-log) nil))) (condition-case e (load {:?} nil t t) (error (list 'load-error e)))) rt-log))", file.to_str().unwrap())));
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = vec![];
    for case in &cases {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", MACROS, "--eval", &format!("(princ {case})")])
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let printed = vm.eval_source(case).unwrap().display().unwrap();
        let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
        if actual != expected {
            failures.push(format!("{case}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A macro whose expansion fails runs once while `eval' compiles the form,
/// then again at its reach, inside the handler (VM2-EVAL-EXPAND1): its
/// cleanup is pushed twice, the first before the form's own body runs,
/// where pinned GNU gives (handled (body cleanup (caught (error "uw")))).
#[test]
fn a_failing_expansion_runs_again_at_its_reach() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    vm.eval_source(MACROS).unwrap();
    let form = "(progn (setq rt-log nil) (eval '(list (condition-case e (progn (push 'body rt-log) (rt-u)) (error (push (list 'caught e) rt-log) 'handled)) (reverse rt-log)) t))";
    let printed = vm.eval_source(&format!("(prin1-to-string {form})")).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, r#"(handled (cleanup body cleanup (caught (error "uw"))))"#);
}

/// A function `eval' defines compiles at its first call, expanding each
/// macro call in its body then and keeping the expansion: 1000 calls
/// expand it once (VM2-EVAL-EXPAND1; GNU's interpreter expands it at every
/// call), so a call costs what a loaded function's does.
#[test]
fn an_eval_defined_function_expands_each_call_site_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let source = "(progn (defvar rt-count 0)
      (defmacro rt-counted (x) (setq rt-count (1+ rt-count)) `(+ ,x 1))
      (eval '(defun rt-h (x) (when x (rt-counted x))) t)
      (let ((defined rt-count))
        (dotimes (i 1000) (rt-h i))
        (list defined rt-count (rt-h 41))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(0 1 42)");
}
