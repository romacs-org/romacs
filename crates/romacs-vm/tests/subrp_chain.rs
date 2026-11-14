use romacs_vm::Vm;
use std::path::PathBuf;
#[path = "support/source_only_bootstrap.rs"]
mod source_only_bootstrap;
use source_only_bootstrap::SourceOnlyEmacsLisp;

#[test]
fn subrp_distinguishes_subrs_from_other_objects() {
    let mut vm = Vm::new();
    for (form, expected) in [
        ("(subrp (symbol-function 'car))", "t"),
        ("(subrp nil)", "nil"),
        ("(subrp t)", "nil"),
        ("(subrp 'car)", "nil"),
        ("(subrp (lambda (x) x))", "nil"),
        ("(subrp '(lambda (x) x))", "nil"),
        ("(subrp 42)", "nil"),
        ("(subrp 1.5)", "nil"),
        ("(subrp '(a . b))", "nil"),
        ("(subrp [a 1])", "nil"),
        ("(subrp \"text\")", "nil"),
        ("(subrp (unibyte-string 255))", "nil"),
        ("(subrp (make-hash-table))", "nil"),
        ("(subrp (standard-syntax-table))", "nil"),
        ("(subrp (make-marker))", "nil"),
        ("(subrp (current-buffer))", "nil"),
        ("(subrp (selected-window))", "nil"),
        ("(subrp (make-overlay 1 1))", "nil"),
        (
            "(subrp (car (read-from-string \"#x10000000000000000000000000000\")))",
            "nil",
        ),
        ("(funcall 'subrp (symbol-function 'car))", "t"),
        ("(list (fboundp 'subrp) (functionp 'subrp))", "(t t)"),
    ] {
        let actual = vm.eval_source(form).and_then(|v| v.display());
        assert_eq!(actual, Ok(expected.into()), "subrp kind assertion: {form}");
    }
}

#[test]
fn measure_subrp_chain_frontier() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source_only = SourceOnlyEmacsLisp::new(&root);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), source_only.path().to_owned()])
        .unwrap();
    for file in [
        "emacs-lisp/debug-early.el",
        "emacs-lisp/byte-run.el",
        "emacs-lisp/backquote.el",
        "subr.el",
        "keymap.el",
        "emacs-lisp/pcase.el",
        "emacs-lisp/gv.el",
        "emacs-lisp/inline.el",
    ] {
        vm.load_file(&root.join("lisp/gnu").join(file)).unwrap();
    }
    let result = vm
        .load_file(&root.join("lisp/gnu/emacs-lisp/ert.el"))
        .map(|_| ());
    println!("ERT result: {result:?}");
    let report = vm.load_report();
    for name in ["cl-macs.el", "cl-lib.el", "ert.el"] {
        let steps: Vec<_> = report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["file"].as_str().unwrap().ends_with(&format!("/{name}")))
            .collect();
        let completed = steps.iter().filter(|s| s["status"] == "ok").count();
        match name {
            "cl-macs.el" => {
                assert_eq!(completed, 178, "all cl-macs forms must complete");
                assert_eq!(steps.len(), 178);
            }
            "cl-lib.el" | "ert.el" => {
                let (count, ordinal, head) = if name == "cl-lib.el" {
                    (94, 95, "(define-minor-mode cl-old-struct-compat-mode")
                } else {
                    (0, 1, "(require 'cl-lib)")
                };
                assert_eq!(completed, count, "completed {name} frontier");
                assert_eq!(steps.len(), count + 1);
                let failed = steps.iter().find(|s| s["status"] == "error").unwrap();
                assert_eq!(failed["ordinal"], ordinal);
                assert!(failed["source"].as_str().unwrap().starts_with(head), "failing form of {name}: {failed}");
                assert!(
                    failed["error"]
                        .as_str()
                        .unwrap()
                        .ends_with("(void-function define-minor-mode)")
                );
            }
            _ => unreachable!(),
        }
        println!(
            "{name}: completed={completed}, attempted={}, failure={:?}",
            steps.len(),
            steps.iter().find(|s| s["status"] == "error")
        );
    }
}
