use romacs_vm::Vm;
use serde_json::Value;

#[test]
fn subr_arity_matches_recorded_gnu_and_lisp_representation() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/subr_arity.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        if case["romacs_representation"] == "lisp" {
            vm.install_lisp_prelude().unwrap();
            let name = case["name"].as_str().unwrap();
            let source = format!("(let ((f (symbol-function '{name}))) (condition-case e (subr-arity f) (wrong-type-argument (list (car e) (car (cdr e)) (eq f (car (cdr (cdr e))))))))");
            assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), "(wrong-type-argument subrp t)");
            continue;
        }
        let source = case["source"].as_str().unwrap();
        let actual = vm.eval_source(source).and_then(|value| value.display());
        assert_eq!(
            actual.as_deref(),
            Ok(case["expected"].as_str().unwrap()),
            "{}: {source}",
            case["name"]
        );
    }
}

#[test]
fn subr_arity_reports_current_native_handler_bounds() {
    for (name, expected) in [
        ("apply", "(2 . many)"),
        ("signal", "(2 . 2)"),
        ("plist-get", "(2 . 2)"),
        ("buffer-size", "(0 . 0)"),
        ("search-forward", "(1 . 4)"),
        ("copy-marker", "(1 . 2)"),
        ("overlays-at", "(1 . 1)"),
        ("set-window-vscroll", "(3 . 3)"),
        ("local-variable-if-set-p", "(1 . 1)"),
        ("make-interpreted-closure", "(5 . 5)"),
        ("buffer-encode-utf8", "(0 . 0)"),
        ("buffer-decode-utf8", "(0 . 0)"),
        ("eval-cache-stats", "(0 . 0)"),
        ("eval-cache-clear", "(0 . 0)"),
        ("eval-cache-entries", "(0 . 0)"),
        ("romacs--read-host-event", "(0 . 0)"),
        ("#:vm2-yank-part", "(1 . 1)"),
        ("#:vm2-yank-finalize", "(4 . 4)"),
        ("#:vm2-yank-property-run", "(3 . 3)"),
        ("#:vm2-hash-next", "(2 . 2)"),
        ("#:vm2-hash-key", "(2 . 2)"),
        ("#:vm2-hash-value", "(2 . 2)"),
        ("#:vm2-defface", "(2 . 2)"),
        ("#:vm2-defvar-boundp", "(1 . 1)"),
        ("#:vm2-defvar-set-default", "(2 . 2)"),
        ("#:vm2-temp-buffer", "(0 . 1)"),
        ("#:vm2-closure-metadata", "(6 . 6)"),
        ("#:vm2-closure-document", "(2 . 3)"),
    ] {
        let mut vm = Vm::new();
        let source = format!("(subr-arity (symbol-function (intern \"{name}\")))");
        let actual = vm.eval_source(&source).and_then(|value| value.display());
        assert_eq!(actual.as_deref(), Ok(expected), "{name}");
    }
}

#[test]
fn subr_arity_results_and_underlying_function_survive_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(
        "(setq subr-arity-test-function (symbol-function 'time-convert))
         (setq subr-arity-test-result (subr-arity subr-arity-test-function))
         (fset 'subr-arity-test-alias subr-arity-test-function)
         (fset 'time-convert (symbol-function 'car))",
    )
    .unwrap();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    let actual = vm
        .eval_source(
            "(list subr-arity-test-result
              (subr-arity subr-arity-test-function)
              (subr-arity (symbol-function 'subr-arity-test-alias))
              (subr-arity (symbol-function 'time-convert)))",
        )
        .and_then(|value| value.display());
    assert_eq!(actual.as_deref(), Ok("((1 . 2) (1 . 2) (1 . 2) (1 . 1))"));
    assert!(vm.heap_stats().collections > before);
}
