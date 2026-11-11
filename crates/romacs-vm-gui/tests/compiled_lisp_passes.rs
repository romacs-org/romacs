//! The file compiler's source passes (lisp/romacs/romacs-compile.el:
//! constant folding, and a literal lambda given to mapcar or mapc made a
//! loop) through the compiled store: a file compiled with them gives
//! what the same file gives read from source, and is compiled, not
//! refused.  Folding keeps a value the store cannot hold out of it.
use std::path::Path;
use std::process::Command;

const PASSES: &str = r#";;; -*- lexical-binding: t -*-
(defvar t-pass-dyn)
(defun t-pass-fold () (list (concat "abc" "def") (symbol-name 'foo) (take 2 '(a b c))))
(defun t-pass-table () (hash-table-p (json-parse-string "{}")))
(defun t-pass-map (l) (mapcar (lambda (x) (* 10 x)) l))
(defun t-pass-mapc (l) (let ((s 0)) (mapc (lambda (x) (setq s (+ s x))) l) s))
(defun t-pass-special () (mapcar (lambda (t-pass-dyn) (boundp 't-pass-dyn)) '(1)))
(defun t-pass-shapes () (list (t-pass-map '(1 2)) (t-pass-map [1 2]) (t-pass-map "ab")
  (condition-case e (t-pass-map '(1 . 2)) (error (car e)))
  (let ((c (list 1))) (setcdr c c) (condition-case e (t-pass-map c) (error (car e))))))
(defun t-pass-circular (xs) (mapcar (lambda (_) '#1=(a . #1#)) xs))
(defun t-pass-nested (x) (mapcar (lambda (x) (mapcar (lambda (x) (mapcar (lambda (x) (mapcar (lambda (x) (list x)) (list x))) (list x))) (list x))) (list x)))
(provide 't-pass)
"#;

/// A batch session over ROOT's store, compiled Lisp on or off: what it prints.
fn session(root: &Path, compiled: bool) -> String {
    let home = root.join(if compiled { "on" } else { "off" });
    for dir in ["config", "cache/romacs/worlds", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-L"]).arg(root.join("lib"))
        .args(["--eval", "(progn (require 't-pass) (prin1 (list (t-pass-fold) (t-pass-table) (t-pass-mapc '(1 2 3)) (t-pass-special) (t-pass-shapes) (eq (car (t-pass-circular '(1))) (cdr (car (t-pass-circular '(1))))) (t-pass-nested 7))))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_COMPILED_LISP", if compiled { "on" } else { "off" }).env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter()
              .filter_map(|name| Some((name, std::env::var_os(name)?))))
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_file_compiled_with_the_passes_answers_as_its_source() {
    let root = std::env::temp_dir().join(format!("romacs-compiled-passes-{}", std::process::id()));
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(root.join("lib/t-pass.el"), PASSES).unwrap();
    // A sibling `.elc`: GNU would load compiled code, so Romacs does.
    std::fs::write(root.join("lib/t-pass.elc"), "").unwrap();
    let source = session(&root, false);
    assert_eq!(source, "((\"abcdef\" \"foo\" (a b)) t 6 (nil) ((10 20) (10 20) (970 980) wrong-type-argument circular-list) t (((((7))))))");
    assert_eq!(session(&root, true), source);
    // The file was compiled into the store, not refused and read as source.
    let artifacts = std::fs::read_dir(root.join("store")).unwrap().flatten()
        .filter(|g| g.file_name().len() == 16)
        .flat_map(|g| std::fs::read_dir(g.path()).unwrap().flatten())
        .flat_map(|key| std::fs::read_dir(key.path()).into_iter().flatten().flatten())
        .filter(|f| f.path().extension().is_some_and(|e| e == "rlc"))
        .count();
    assert!(artifacts >= 1, "no compiled artifact for t-pass.el");
    let _ = std::fs::remove_dir_all(&root);
}
