//! current-load-list while a file loads, against pinned GNU (lread.c
//! loadhist_initialize): it starts as (FILE) and the file stays its last
//! element, which macroexp-file-name reads. VM2 does not attach
//! definitions to it (DIVERGENCES.md VM2-LOADHIST1).
use romacs_vm::Vm;

const FILE: &str = "\
;;; -*- lexical-binding: t -*-
(setq rt-load-first (list (nth (1- (length current-load-list)) current-load-list) (length current-load-list)))
(defalias 'rt-load-f (lambda () 1))
(defvar rt-load-v 3)
(setq rt-load-later (nth (1- (length current-load-list)) current-load-list))
";

#[test]
fn current_load_list_during_a_load_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-load-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::fs::canonicalize(&dir).unwrap().join("rt-load-list.el");
    std::fs::write(&path, FILE).unwrap();
    let file = path.to_str().unwrap();
    // Both runs load the same path, so its name compares as is.
    let report = "(prin1-to-string (list rt-load-first rt-load-later))";
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(progn (load {file:?} nil t) (princ {report}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&path).unwrap();
    let actual = vm.eval_source(report).and_then(|v| v.display()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(actual.trim_matches('"').replace("\\\"", "\""), expected);
}
