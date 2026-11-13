//! Keep the authored source-only frontier distinct from generated-support loading.
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use romacs_vm::Vm;
use source_only_bootstrap::SourceOnlyEmacsLisp;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn source_only_fixture_omits_only_cl_loaddefs_and_cleans_up() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let production = root.join("lisp/gnu/emacs-lisp");
    let generated = fs::read(production.join("cl-loaddefs.el")).unwrap();
    assert!(
        !generated.is_empty(),
        "production generated cl-loaddefs remains present"
    );
    let directory;
    {
        let fixture = SourceOnlyEmacsLisp::new(&root);
        directory = fixture.path().to_owned();
        let mut expected = BTreeSet::new();
        for entry in fs::read_dir(&production).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() && entry.file_name() != "cl-loaddefs.el" {
                expected.insert(entry.file_name());
                assert_eq!(
                    fs::read(entry.path()).unwrap(),
                    fs::read(directory.join(entry.file_name())).unwrap(),
                    "GNU source bytes must not change: {:?}",
                    entry.path()
                );
            }
        }
        assert!(!expected.is_empty());
        let actual: BTreeSet<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            actual, expected,
            "only generated cl-loaddefs is intentionally unavailable"
        );
        assert!(!directory.join("cl-loaddefs.el").exists());
    }
    assert!(
        !directory.exists(),
        "RAII removes only the owned search directory"
    );
    assert_eq!(
        fs::read(production.join("cl-loaddefs.el")).unwrap(),
        generated
    );
}

#[test]
fn ordinary_search_path_still_loads_production_generated_support() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), root.join("lisp/gnu/emacs-lisp")])
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
        vm.load_file(&root.join("lisp/gnu").join(file)).unwrap();
    }
    let result = vm
        .load_file(&root.join("lisp/gnu/emacs-lisp/cl-lib.el"))
        .map(|_| ());
    assert!(
        result.is_err(),
        "remeasure generated-support cl-lib frontier"
    );
    let report = vm.load_report();
    let steps = report["steps"].as_array().unwrap();
    let generated: Vec<_> = steps
        .iter()
        .filter(|step| step["file"].as_str().unwrap().ends_with("/cl-loaddefs.el"))
        .collect();
    assert_eq!(
        generated.len(),
        279,
        "production generated support is actually consumed"
    );
    assert!(generated.iter().all(|step| step["status"] == "ok"));
    assert!(generated.iter().all(|step| {
        std::path::Path::new(step["file"].as_str().unwrap())
            .canonicalize()
            .unwrap()
            == root
                .join("lisp/gnu/emacs-lisp/cl-loaddefs.el")
                .canonicalize()
                .unwrap()
    }));
    assert!(
        !steps
            .iter()
            .any(|step| step["file"].as_str().unwrap().ends_with("/cl-macs.el")),
        "generated support bypasses GNU's source-only fallback"
    );
    let own: Vec<_> = steps
        .iter()
        .filter(|step| step["file"].as_str().unwrap().ends_with("/cl-lib.el"))
        .collect();
    assert_eq!(own.iter().filter(|step| step["status"] == "ok").count(), 94);
    assert_eq!(own.len(), 95);
    assert_eq!(own[94]["ordinal"], 95);
    assert!(own[94]["source"].as_str().unwrap().starts_with("(define-minor-mode cl-old-struct-compat-mode"), "{}", own[94]);
    assert_eq!(own[94]["error"], "(void-function define-minor-mode)");
}
