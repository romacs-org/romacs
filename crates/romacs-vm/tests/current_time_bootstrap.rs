use romacs_vm::Vm;

#[test]
fn current_time_unblocks_pristine_gnu_version() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")])
        .unwrap();
    for relative in [
        "gnu-lisp/emacs-lisp/debug-early.el",
        "gnu-lisp/emacs-lisp/byte-run.el",
        "gnu-lisp/emacs-lisp/backquote.el",
        "gnu-lisp/subr.el",
        "gnu-lisp/keymap.el",
    ] {
        vm.load_file(&root.join(relative)).unwrap();
    }
    let path = root.join("gnu-lisp/version.el");
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
