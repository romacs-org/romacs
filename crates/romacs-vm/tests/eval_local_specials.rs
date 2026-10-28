//! A file's `(defvar NAME)` reaches code a macro evaluates while the file
//! loads: readevalloop binds `macroexp--dynvars' for the file and
//! macroexpand--all-toplevel conses each declaration onto it, and `eval'
//! treats a bare symbol in its LEXICAL environment as locally special
//! (src/eval.c Fdefvar, Flet). eval-and-compile inside pcase-defmacro
//! depends on both (rx.el's `rx--pcase-vars').
use romacs_vm::Vm;

const FIXTURE: &str = ";;; els.el --- -*- lexical-binding: t -*-
(defvar els-v)
(defun els-get () (if (boundp 'els-v) els-v 'unbound))
(defmacro els-dynvars () (list 'quote macroexp--dynvars))
(defmacro els-at-expansion () (list 'quote (eval '(let ((els-v 1)) (els-get)) (or macroexp--dynvars t))))
(setq els-result
      (list (els-dynvars)
            (els-at-expansion)
            (eval '(let ((els-v 2)) (els-get)) '(els-v))
            (funcall (eval '(lambda () (let ((els-v 3)) (els-get))) '(els-v)))
            (eval '(let ((els-v 4)) (els-get)) t)
            (eval '(let ((els-v 5)) (els-get)) '(t els-v))
            (eval '(let ((els-v 6)) (els-get)) '((other . 0) els-v))
            (special-variable-p 'els-v)))
";

#[test]
fn file_declarations_reach_expansion_time_eval_as_in_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("eval-local-specials-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("els.el");
    std::fs::write(&file, FIXTURE).unwrap();
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "-l"]).arg(&file)
        .args(["--eval", "(princ (prin1-to-string els-result))"])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, "((els-v) 1 2 3 unbound 5 6 nil)");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&file).unwrap();
    let mine = vm.eval_source("(prin1-to-string els-result)").unwrap().display().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}

/// A LEXICAL environment that both binds and declares a name: the supplied
/// binding stays lexical, so references read it, while the declaration
/// makes the form's own `let' of the name dynamic (src/eval.c eval_sub
/// looks the lexical binding up first).
#[test]
fn eval_keeps_supplied_bindings_lexical_beside_declarations() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(list (eval '(list x (let ((x 2)) x)) '((x . 1) x))
      (eval '(let ((z 3)) (list z (funcall (lambda () z)) (symbol-value 'z))) '((z . 1) z))
      (eval '(let ((w 4)) (list w (symbol-value 'w))) '(w (w . 1))))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}
