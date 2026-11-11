//! Direct GNU-source dependency proof, not a replacement loadup sequence.
//! Normal warm loadup still stops earlier in version.el on system-name.
use romacs_vm::Vm;
use std::path::PathBuf;
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use source_only_bootstrap::SourceOnlyEmacsLisp;

fn prepared(source_only: Option<&SourceOnlyEmacsLisp>) -> (Vm, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let emacs_lisp = source_only.map_or_else(|| root.join("lisp/gnu/emacs-lisp"), |f| f.path().to_owned());
    vm.set_load_path(&[root.join("lisp/gnu"), emacs_lisp]).unwrap();
    for file in [
        "emacs-lisp/debug-early.el", "emacs-lisp/byte-run.el",
        "emacs-lisp/backquote.el", "subr.el", "keymap.el",
        "emacs-lisp/pcase.el",
    ] {
        vm.load_file(&root.join("lisp/gnu").join(file)).unwrap();
    }
    (vm, root)
}

#[test]
fn real_pcase_allows_real_gv_to_load_completely() {
    let (mut vm, root) = prepared(None);
    let result = vm.load_file(&root.join("lisp/gnu/emacs-lisp/gv.el")).map(|_| ());
    assert!(result.is_ok(), "GNU gv.el must load after GNU pcase.el: {result:?}");
    assert_eq!(vm.eval_source("(list (featurep 'pcase) (featurep 'gv) (car (symbol-function 'gv-define-expander)) (car (symbol-function 'pcase-defmacro)))").unwrap().display().unwrap(), "(t t macro macro)");
    let report = vm.load_report();
    for (name, count) in [("pcase.el", 53), ("gv.el", 176)] {
        let steps: Vec<_> = report["steps"].as_array().unwrap().iter()
            .filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{name}"))).collect();
        assert_eq!(steps.len(), count, "measured GNU source form count for {name}");
        assert!(steps.iter().all(|s| s["status"] == "ok"), "all GNU forms must execute: {name}");
    }
}

fn assert_identity_frontier(vm: &mut Vm) {
    assert_eq!(vm.eval_source("(list (fboundp 'cl--block-wrapper) (symbol-function 'cl--block-wrapper) (fboundp 'cl--block-throw) (symbol-function 'cl--block-throw) (fboundp 'identity) (cl--block-wrapper 42))").unwrap().display().unwrap(), "(t identity t throw t 42)");
    let report = vm.load_report();
    for (name, completed, ordinal, line) in [
        ("cl-lib.el", 91, 92, 553),
        ("cl-macs.el", 158, 159, 3579),
    ] {
        let steps: Vec<_> = report["steps"].as_array().unwrap().iter()
            .filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{name}"))).collect();
        assert_eq!(steps.iter().filter(|s| s["status"] == "ok").count(), completed, "remeasure {name}");
        assert_eq!(steps.len(), completed + 1, "attempted forms for {name}");
        let failure = steps.iter().find(|s| s["status"] != "ok").unwrap();
        assert_eq!(failure["ordinal"], ordinal);
        assert_eq!(failure["line"], line);
        if name == "cl-macs.el" {
            assert_eq!(failure["error"], "(void-function define-inline)");
        } else {
            assert!(failure["error"].as_str().unwrap().ends_with("(void-function define-inline)"));
        }
    }
}

#[test]
fn real_identity_moves_nested_cl_macs_to_define_inline() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    let (mut vm, root) = prepared(Some(&source_only));
    vm.load_file(&root.join("lisp/gnu/emacs-lisp/gv.el")).unwrap();
    let result = vm.load_file(&root.join("lisp/gnu/emacs-lisp/cl-lib.el")).map(|_| ());
    assert!(result.is_err(), "remeasure cl-lib if the frontier advances");
    assert_identity_frontier(&mut vm);
}

#[test]
fn real_ert_frontier_after_identity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    let (mut vm, root) = prepared(Some(&source_only));
    vm.load_file(&root.join("lisp/gnu/emacs-lisp/gv.el")).unwrap();
    let result = vm.load_file(&root.join("lisp/gnu/emacs-lisp/ert.el")).map(|_| ());
    assert!(result.is_err(), "remeasure ert if the frontier advances");
    assert_identity_frontier(&mut vm);
    let report = vm.load_report();
    let steps: Vec<_> = report["steps"].as_array().unwrap().iter()
        .filter(|s| s["file"].as_str().unwrap().ends_with("/ert.el")).collect();
    assert_eq!(steps.len(), 1, "ERT attempts its first require only");
    assert_eq!(steps[0]["status"], "error");
    assert_eq!(steps[0]["ordinal"], 1);
    assert_eq!(steps[0]["line"], 57);
    assert_eq!(steps[0]["source"], "(require 'cl-lib)");
    assert!(steps[0]["error"].as_str().unwrap().ends_with("(void-function define-inline)"));
}

#[test]
fn real_pcase_and_gv_allow_real_rx_to_load_completely() {
    let (mut vm, root) = prepared(None);
    vm.load_file(&root.join("lisp/gnu/emacs-lisp/gv.el")).unwrap();
    vm.load_file(&root.join("lisp/gnu/emacs-lisp/rx.el")).unwrap();
    assert_eq!(vm.eval_source("(featurep 'rx)").unwrap().display().unwrap(), "t");
    let report = vm.load_report();
    assert_eq!(report["steps"].as_array().unwrap().iter()
        .filter(|s| s["file"].as_str().unwrap().ends_with("/rx.el") && s["status"] == "ok").count(), 86);
}
