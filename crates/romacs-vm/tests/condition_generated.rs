use romacs_vm::Vm;

#[test]
fn condition_case_preserves_uninterned_handler_identity() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(
        r#"(defmacro generated-handler (body)
        (let ((err (make-symbol "err")))
          (list 'condition-case err body
            (list 'error (list 'list (list 'quote 'caught) err))
            (list :success (list 'list (list 'quote 'success) err)))))"#,
    )
    .unwrap();
    let source = r#"(let ((err 'outer))
        (list (generated-handler (signal 'error '("witness")))
              (generated-handler 42) err))"#;
    let result = vm.eval_source(source).and_then(|v| v.display());
    let recorded: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/condition-generated-gnu32.json")).unwrap();
    assert_eq!(
        result.as_deref(),
        Ok(recorded["stdout"].as_str().unwrap()),
        "CONDITION_CASE_UNINTERNED_HANDLER_IDENTITY"
    );
}

#[test]
fn custom_bootstrap_frontier() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")])
        .unwrap();
    for relative in [
        "gnu-lisp/emacs-lisp/byte-run.el",
        "gnu-lisp/subr.el",
        "gnu-lisp/emacs-lisp/macroexp.el",
        "gnu-lisp/emacs-lisp/pcase.el",
        "gnu-lisp/custom.el",
        "gnu-lisp/files.el",
    ] {
        let path = root.join(relative);
        let result = vm.load_file(&path).map(drop);
        let canonical = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let report = vm.load_report();
        let steps = report["steps"].as_array().unwrap();
        let completed = steps
            .iter()
            .filter(|s| s["file"] == canonical && s["status"] == "ok")
            .count();
        eprintln!("CUSTOM_FRONTIER {relative}: completed={completed}; result={result:?}");
        for step in steps
            .iter()
            .filter(|s| s["file"] == canonical && s["status"] != "ok")
        {
            eprintln!("CUSTOM_FAILURE {step}");
        }
        if relative == "gnu-lisp/custom.el" {
            let callable = vm
                .eval_source("(fboundp 'custom--standard-value-p)")
                .unwrap()
                .display()
                .unwrap();
            assert_eq!(callable, "t", "CUSTOM_FORM33_COMPILES");
        }
    }
    eprintln!("CUSTOM_ENTRY_POINTS {}", vm.eval_source("(list (fboundp 'defgroup) (fboundp 'find-file) (fboundp 'find-file-noselect) (fboundp 'read-file-name) (fboundp 'locate-file))").unwrap().display().unwrap());
}
