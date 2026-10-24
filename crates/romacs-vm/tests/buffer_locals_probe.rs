use romacs_vm::Vm;

#[test]
fn measure_native_buffer_local_and_overlay_availability() {
    let mut vm = Vm::new();
    for name in [
        "kill-all-local-variables",
        "local-variable-p",
        "delete-all-overlays",
        "local-variable-if-set-p",
        "buffer-local-value",
        "buffer-local-variables",
        "kill-local-variable",
        "make-local-variable",
        "make-variable-buffer-local",
        "make-overlay",
        "delete-overlay",
        "overlayp",
        "overlay-start",
        "overlay-end",
        "overlay-buffer",
        "overlay-get",
        "overlay-put",
        "overlays-at",
        "overlays-in",
        "next-overlay-change",
        "previous-overlay-change",
        "move-overlay",
    ] {
        let source = format!("(list (fboundp '{name}) (functionp '{name}))");
        println!(
            "AVAILABILITY {name} {}",
            vm.eval_source(&source).unwrap().display().unwrap()
        );
    }
}

#[test]
fn measure_buffer_local_target_frontiers() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for target in ["simple.el", "files.el"] {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")])
            .unwrap();
        for file in [
            "emacs-lisp/debug-early.el",
            "emacs-lisp/byte-run.el",
            "emacs-lisp/backquote.el",
            "subr.el",
            "keymap.el",
            "emacs-lisp/pcase.el",
            "emacs-lisp/gv.el",
        ] {
            vm.load_file(&root.join("gnu-lisp").join(file)).unwrap();
        }
        let outcome = vm.load_file(&root.join("gnu-lisp").join(target)).map(|_| ());
        println!("TARGET {target} {outcome:?}");
        let report = vm.load_report();
        for step in report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["status"] != "ok")
        {
            println!("DEPENDENCY_FAILURE {target} {step}");
        }
        let steps: Vec<_> = report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{target}")))
            .collect();
        println!(
            "FRONTIER {target} completed={} attempted={}",
            steps.iter().filter(|s| s["status"] == "ok").count(),
            steps.len()
        );
        for step in steps.iter().filter(|s| s["status"] != "ok") {
            println!("FAILURE {target} {step}");
        }
    }
}

#[test]
fn native_buffer_local_primitives_are_callable() {
    let mut vm = Vm::new();
    let actual = vm.eval_source("(list (fboundp 'kill-all-local-variables) (fboundp 'local-variable-p) (fboundp 'delete-all-overlays))").unwrap().display().unwrap();
    assert_eq!(
        actual, "(t t t)",
        "GNU buffer-local primitives must be callable before loading Lisp"
    );
}
