use romacs_vm::Vm;
use std::path::PathBuf;
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use source_only_bootstrap::SourceOnlyEmacsLisp;

#[test]
fn mapc_bootstrap_frontiers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    for target in ["cl-lib.el", "ert.el"] {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_load_path(&[root.join("lisp/gnu"), source_only.path().to_owned()]).unwrap();
        for file in ["emacs-lisp/debug-early.el", "emacs-lisp/byte-run.el", "emacs-lisp/backquote.el", "subr.el", "keymap.el", "emacs-lisp/pcase.el", "emacs-lisp/gv.el"] {
            vm.load_file(&root.join("lisp/gnu").join(file)).unwrap();
        }
        let result = vm.load_file(&root.join("lisp/gnu/emacs-lisp").join(target)).map(|_| ());
        eprintln!("{target}: {result:?}");
        let report = vm.load_report();
        let steps = report["steps"].as_array().unwrap();
        let own: Vec<_> = steps.iter().filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{target}"))).collect();
        eprintln!("{target}: {} successful / {} attempted", own.iter().filter(|s| s["status"] == "ok").count(), own.len());
        for s in steps.iter().filter(|s| s["status"] != "ok") { eprintln!("{target} failure: {s}"); }
        let expected = if target == "cl-lib.el" { 91 } else { 0 };
        assert_eq!(own.iter().filter(|s| s["status"] == "ok").count(), expected, "remeasure the mapc bootstrap frontier");
        assert!(result.is_err(), "remeasure if the target completes");
        let failure = own.iter().find(|s| s["status"] != "ok").unwrap();
        assert_eq!(failure["ordinal"], if target == "cl-lib.el" { 92 } else { 1 });
        let head = if target == "cl-lib.el" { "(unless (load \"cl-loaddefs\"" } else { "(require 'cl-lib)" };
        assert!(failure["source"].as_str().unwrap().starts_with(head), "failing form of {target}: {failure}");
        assert!(failure["error"].as_str().unwrap().contains("(void-function define-inline)"));
        let cause = steps.iter().find(|s| s["status"] != "ok" && s["file"].as_str().unwrap().ends_with("/cl-macs.el")).unwrap();
        assert_eq!(cause["ordinal"], 159);
        assert!(cause["source"].as_str().unwrap().starts_with("(define-inline cl-typep "), "failing form of cl-macs.el: {cause}");
        assert_eq!(cause["error"], "(void-function define-inline)");
    }
}
