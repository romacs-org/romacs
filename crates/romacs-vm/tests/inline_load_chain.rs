//! GNU source-only dependency frontier for define-inline.
use romacs_vm::Vm;
use std::path::PathBuf;
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use source_only_bootstrap::SourceOnlyEmacsLisp;

#[test]
fn inline_dependency_frontier() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), source_only.path().to_owned()]).unwrap();
    for file in ["emacs-lisp/debug-early.el", "emacs-lisp/byte-run.el", "emacs-lisp/backquote.el", "subr.el", "keymap.el", "emacs-lisp/pcase.el", "emacs-lisp/gv.el", "emacs-lisp/inline.el"] {
        let result = vm.load_file(&root.join("lisp/gnu").join(file)).map(|_| ());
        println!("{file}: {result:?}");
        assert!(result.is_ok(), "GNU dependency must load: {file}: {result:?}");
    }
    let result = vm.load_file(&root.join("lisp/gnu/emacs-lisp/ert.el")).map(|_| ());
    println!("ERT: {result:?}");
    let report = vm.load_report();
    for (name, completed, total, ordinal, line) in [
        ("inline.el", 22, 22, 0, 0),
        ("macroexp.el", 50, 50, 0, 0),
        ("cl-macs.el", 178, 178, 0, 0),
        ("cl-lib.el", 94, 97, 95, 599),
        ("ert.el", 0, 204, 1, 57),
    ] {
        let steps: Vec<_> = report["steps"].as_array().unwrap().iter()
            .filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{name}")))
            .collect();
        assert_eq!(steps.iter().filter(|s| s["status"] == "ok").count(), completed,
            "remeasure successful prefix of {name}/{total}");
        if ordinal != 0 {
            let failed = steps.iter().find(|s| s["status"] != "ok").unwrap();
            assert_eq!(failed["ordinal"], ordinal);
            assert_eq!(failed["line"], line);
            assert!(failed["error"].as_str().unwrap().ends_with("(void-function define-minor-mode)"));
        }
    }
    assert_eq!(vm.eval_source("(list (featurep 'inline) (featurep 'macroexp) (car (symbol-function 'define-inline)))").unwrap().display().unwrap(), "(t t macro)");
}
