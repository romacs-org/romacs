use romacs_vm::Vm;
use std::path::PathBuf;

#[test]
fn ordered_real_ert_dependency_frontier() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu");
    let mut vm = Vm::new();
    vm.set_load_path(&[root.clone(), root.join("emacs-lisp")])
        .unwrap();
    vm.install_lisp_prelude().unwrap();
    for (name, count) in [
        ("emacs-lisp/debug-early", 6),
        ("emacs-lisp/byte-run", 56),
        ("emacs-lisp/backquote", 16),
        ("subr", 522),
        ("keymap", 25),
        ("emacs-lisp/pcase", 53),
        ("emacs-lisp/gv", 176),
        ("emacs-lisp/inline", 22),
        ("custom", 91),
        ("emacs-lisp/easy-mmode", 18),
    ] {
        let file = root.join(format!("{name}.el"));
        vm.load_file(&file)
            .unwrap_or_else(|e| panic!("ORDERED_LOAD {name}: {e}"));
        let report = vm.load_report();
        let steps = report["steps"].as_array().unwrap();
        let suffix = format!("/{name}.el");
        assert_eq!(
            steps
                .iter()
                .filter(
                    |s| s["file"].as_str().unwrap_or("").ends_with(&suffix) && s["status"] == "ok"
                )
                .count(),
            count,
            "ORDERED_FORMS {name}"
        );
    }
    assert_eq!(vm.eval_source("(list (featurep 'pcase) (featurep 'gv) (featurep 'easy-mmode) (car (symbol-function 'define-minor-mode)))").unwrap().display().unwrap(), "(t t t macro)");
    let error = vm
        .load_file(&root.join("emacs-lisp/cl-lib.el"))
        .unwrap_err();
    assert!(
        error.contains("line 599 form 95") && error.contains("(void-variable emacs-lisp-docstring-fill-column)"),
        "ORDERED_CL_LIB_FRONTIER: {error}"
    );
    // A separate dependency diagnosis, not permission to resume this failed chain.
    let bindings = vm.load_file(&root.join("bindings.el"));
    assert!(bindings.is_ok(), "ORDERED_BINDINGS_COMPLETE: {bindings:?}");
    assert_eq!(vm.eval_source("(fboundp 'bound-and-true-p)").unwrap().display().unwrap(), "t");
}
