use romacs_vm::Vm;
use serde_json::Value;
use std::process::Command;

fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/where_is/cases.json")).unwrap()
}

#[test]
fn where_is_matches_recorded_gnu_bindings() {
    for case in cases() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude()
            .expect("install T3 keymap traversal");
        let source = case["source"].as_str().unwrap();
        let result = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(
            result.as_deref(),
            Ok(case["expected"].as_str().unwrap()),
            "{}: {source}",
            case["name"]
        );
    }
}

#[test]
fn where_is_differential_oracle() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE")
        .expect("set ROMACS_GNU_ORACLE to GNU pin 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea");
    for case in cases() {
        let source = case["source"].as_str().unwrap();
        let output = Command::new(&oracle)
            .args([
                "--batch",
                "-Q",
                "--eval",
                &format!("(prin1 (eval '{source} t))"),
            ])
            .output()
            .expect("run pinned GNU");
        assert!(
            output.status.success(),
            "{}: {}",
            case["name"],
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let result = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(
            result.as_deref(),
            Ok(case["expected"].as_str().unwrap()),
            "{}: {source}",
            case["name"]
        );
    }
}

/// The walk over the keymaps calls no Lisp; a menu item's `:filter` runs
/// when a sequence found is looked up again.  What the lookup holds then
/// (its arguments and the filters' results) survives a collection there.
#[test]
fn where_is_survives_a_collection_in_a_menu_filter() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    vm.eval_source(
        "(defun wi-churn (definition)
           (let ((i 0)) (while (< i 12000) (cons i i) (setq i (1+ i))))
           (garbage-collect) definition)",
    )
    .unwrap();
    let before = vm.heap_stats().collections;
    let result = vm
        .eval_source(
            "(let ((m (make-sparse-keymap)) (p (make-sparse-keymap)))
      (define-key p [97] (list 'menu-item \"A\" 'wi-target :filter 'wi-churn))
      (define-key m [120] p) (define-key m [121] p)
      (define-key m [98] (list 'menu-item \"B\" 'wi-target :filter 'wi-churn))
      (list (where-is-internal 'wi-target (list m)) (where-is-internal 'wi-target (list m) t)))",
        )
        .and_then(|v| v.display());
    assert_eq!(result.as_deref(), Ok("(([98] [121 97] [120 97]) [98])"));
    assert!(
        vm.heap_stats().collections > before + 3,
        "the filters must collect while the lookup is suspended"
    );
}

// GNU 32.0.50: a menu item's own definition is found; its :filter is not
// what where-is-internal reports.
#[test]
fn where_is_reports_menu_items_with_filters_as_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let found = vm.eval_source("(list (where-is-internal 'wi-target (list (list 'keymap (cons 97 '(menu-item \"x\" wi-target :filter identity))))) (where-is-internal 'wi-target (list (list 'keymap (cons 97 '(menu-item \"x\" ignore :filter (lambda (_) 'wi-target)))))))")
        .and_then(|v| v.display());
    assert_eq!(found.as_deref(), Ok("(([97]) nil)"));
}
