use romacs_vm::Vm;

#[test]
fn current_time_unblocks_pristine_gnu_version() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), root.join("lisp/gnu/emacs-lisp")])
        .unwrap();
    for relative in [
        "lisp/gnu/emacs-lisp/debug-early.el",
        "lisp/gnu/emacs-lisp/byte-run.el",
        "lisp/gnu/emacs-lisp/backquote.el",
        "lisp/gnu/subr.el",
        "lisp/gnu/keymap.el",
    ] {
        vm.load_file(&root.join(relative)).unwrap();
    }
    let path = root.join("lisp/gnu/version.el");
    let result = vm.load_file(&path).map(drop);
    assert!(result.is_ok(), "CURRENT_TIME_VERSION_COMPLETES: {result:?}");
    let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
    let report = vm.load_report();
    let completed = report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["file"] == canonical && s["status"] == "ok")
        .count();
    // Measured warm census, GNU pin 45b0f7699df: all 24 forms complete.
    assert_eq!(completed, 24, "CURRENT_TIME_VERSION_COMPLETION_PIN_MOVED");
    assert_eq!(
        vm.eval_source("(consp emacs-build-time)")
            .unwrap()
            .display()
            .unwrap(),
        "t",
        "CURRENT_TIME_BUILD_TIME_RECORDED"
    );
}
