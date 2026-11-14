use romacs_vm::Vm;

#[test]
fn custom_loads_and_files_frontier_is_pinned() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), root.join("lisp/gnu/emacs-lisp")]).unwrap();
    for (relative, expected) in [("lisp/gnu/emacs-lisp/byte-run.el", 56), ("lisp/gnu/subr.el", 522), ("lisp/gnu/emacs-lisp/macroexp.el", 50), ("lisp/gnu/emacs-lisp/pcase.el", 53), ("lisp/gnu/custom.el", 91), ("lisp/gnu/files.el", 49)] {
        let path = root.join(relative);
        let result = vm.load_file(&path).map(drop);
        let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let report = vm.load_report();
        let completed = report["steps"].as_array().unwrap().iter().filter(|s| s["file"] == canonical && s["status"] == "ok").count();
        eprintln!("CUSTOM_BOOTSTRAP_CHAIN {relative}: completed={completed}; result={result:?}");
        assert_eq!(completed, expected, "CUSTOM_BOOTSTRAP_FRONTIER_MOVED {relative}: {result:?}");
        if relative == "lisp/gnu/files.el" {
            let error = result.unwrap_err();
            assert!(error.contains(" form 50: ") && error.ends_with("(void-variable emacs-lisp-docstring-fill-column)"), "CUSTOM_FILES_FRONTIER_MOVED: {error}");
            assert!(report["steps"].as_array().unwrap().iter().any(|s| s["file"] == canonical && s["status"] == "error" && s["source"].as_str().unwrap().starts_with("(define-minor-mode auto-save-visited-mode")), "CUSTOM_FILES_FRONTIER_MOVED: {error}");
        } else {
            assert!(result.is_ok(), "CUSTOM_BOOTSTRAP_LOAD_FAILED: {result:?}");
        }
    }
    // A completion pin for custom; an attribution pin, NOT a usability claim,
    // for files. The missing functions are GNU Lisp defined after form 45.
    let result = vm.eval_source("(list (fboundp 'defgroup) (fboundp 'find-file) (fboundp 'find-file-noselect) (fboundp 'read-file-name) (fboundp 'locate-file))").unwrap().display().unwrap();
    eprintln!("CUSTOM_BOOTSTRAP_FUNCTIONS {result}");
    assert_eq!(result, "(t nil nil nil nil)", "CUSTOM_BOOTSTRAP_FRONTIER_FUNCTIONS_MOVED");
}
