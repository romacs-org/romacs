use romacs_vm::Vm;
use std::path::PathBuf;

#[test]
fn minor_mode_docstring_search_can_save_dead_buffer_match_data() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu");
    let mut vm = Vm::new();
    vm.set_load_path(&[root.clone(), root.join("emacs-lisp")]).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in ["emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote", "subr", "keymap", "emacs-lisp/pcase", "emacs-lisp/gv", "emacs-lisp/inline", "custom"] {
        vm.load_file(&root.join(format!("{name}.el"))).unwrap_or_else(|e| panic!("MINOR_MODE_SETUP {name}: {e}"));
    }
    vm.eval_source("(with-temp-buffer (insert \"x\\n\\n\") (ensure-empty-lines))").unwrap();
    let marker = vm.eval_source("(let ((m (car (match-data)))) (list (eq (type-of m) 'marker) (marker-position m) (marker-buffer m)))").and_then(|v| v.display());
    assert_eq!(marker.as_deref(), Ok("(t nil nil)"), "MINOR_MODE_DETACHED_MATCH_DATA");
    let saved = vm.eval_source("(save-match-data 17)").and_then(|v| v.display());
    assert_eq!(saved.as_deref(), Ok("17"), "MINOR_MODE_SAVE_MATCH_DATA");
    let bindings = vm.load_file(&root.join("bindings.el"));
    assert!(bindings.is_ok(), "BINDINGS_COMPLETE: {bindings:?}");
    let files = vm.load_file(&root.join("files.el"));
    assert!(files.is_ok(), "FILES_MINOR_MODE_LIFETIME: {files:?}");
    assert_eq!(vm.eval_source("(list (featurep 'files) (fboundp 'lock-file-mode))").and_then(|v| v.display()).as_deref(), Ok("(t t)"));
}
