//! `(aref CLOSURE 2)', an interpreted closure's environment, for a closure
//! made where no lexical binding is visible: GNU's is the dialect's empty
//! one, (t) under lexical binding and nil under dynamic. nadvice reads it
//! to run an advised command's computed interactive form in its context,
//! so `commandp' over an advised command (org-fold's outline advice, M-x
//! completion's predicate) needs it. Against pinned GNU, on the loadup
//! chain where nadvice is loaded. A closure inside a `let' still refuses
//! (VM2-CLOSEQ1).
use romacs_vm::Vm;
use std::path::Path;

const EXPRESSIONS: &[&str] = &[
    "(aref (lambda (x) x) 2)",
    "(eval '(aref (lambda (x) x) 2) nil)",
    "(progn (defun ce-f (x) (interactive (list 1)) x) (aref (symbol-function 'ce-f) 2))",
    "(progn (defun ce-cmd (x) (interactive (list (+ 1 2))) x) (advice-add 'ce-cmd :around (lambda (f &rest a) (apply f a))) (list (commandp 'ce-cmd) (call-interactively 'ce-cmd) (car (interactive-form 'ce-cmd))))",
];

fn romacs() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

#[test]
fn a_toplevel_closure_environment_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = romacs();
    let mut failures = Vec::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    let inner = vm.eval_source("(condition-case e (let ((y 1)) (aref (lambda (x) x) 2)) (error (car e)))").unwrap().display().unwrap();
    assert_eq!(inner, "wrong-type-argument", "a closure under a let keeps refusing");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
