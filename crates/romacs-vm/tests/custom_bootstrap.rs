use romacs_vm::Vm;

#[test]
fn custom_loads_and_files_frontier_is_pinned() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    for (relative, expected) in [("gnu-lisp/emacs-lisp/byte-run.el", 56), ("gnu-lisp/subr.el", 522), ("gnu-lisp/emacs-lisp/macroexp.el", 50), ("gnu-lisp/emacs-lisp/pcase.el", 53), ("gnu-lisp/custom.el", 91), ("gnu-lisp/files.el", 49)] {
        let path = root.join(relative);
        let result = vm.load_file(&path).map(drop);
        let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let report = vm.load_report();
        let completed = report["steps"].as_array().unwrap().iter().filter(|s| s["file"] == canonical && s["status"] == "ok").count();
        eprintln!("CUSTOM_BOOTSTRAP_CHAIN {relative}: completed={completed}; result={result:?}");
        assert_eq!(completed, expected, "CUSTOM_BOOTSTRAP_FRONTIER_MOVED {relative}: {result:?}");
        if relative == "gnu-lisp/files.el" {
            let error = result.unwrap_err();
            assert!(error.contains("line 494 form 50:") && error.ends_with("(void-variable emacs-lisp-docstring-fill-column)"), "CUSTOM_FILES_FRONTIER_MOVED: {error}");
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
