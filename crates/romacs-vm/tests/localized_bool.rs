use romacs_vm::{ImageRegion, Vm};
use std::process::Command;

fn shown(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"))
        .display()
        .unwrap()
}

fn check(source: &str, expected: &str) {
    if let Ok(oracle) = std::env::var("ROMACS_GNU_ORACLE") {
        let output = Command::new(oracle)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "GNU failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "pinned GNU: {source}"
        );
    }
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(shown(&mut vm, source), expected, "candidate: {source}");
}

#[test]
fn explicit_local_and_default_values_match_gnu() {
    check(
        "(with-temp-buffer (set-default 'inhibit-modification-hooks nil) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks 7) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks) (local-variable-p 'inhibit-modification-hooks) (local-variable-if-set-p 'inhibit-modification-hooks) (buffer-local-value 'inhibit-modification-hooks (current-buffer)) (progn (set-default 'inhibit-modification-hooks t) (setq inhibit-modification-hooks nil) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks))) (progn (kill-local-variable 'inhibit-modification-hooks) (list inhibit-modification-hooks (local-variable-p 'inhibit-modification-hooks) (local-variable-if-set-p 'inhibit-modification-hooks)))))",
        "(t nil t t t (nil t) (t nil nil))",
    );
    check(
        "(with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks '(payload)) (list (cdr (assq 'inhibit-modification-hooks (buffer-local-variables))) (boundp 'inhibit-modification-hooks) (default-boundp 'inhibit-modification-hooks) (special-variable-p 'inhibit-modification-hooks)))",
        "(t t t t)",
    );
}

#[test]
fn automatic_default_bindings_are_buffer_qualified() {
    check(
        "(let ((a (get-buffer-create \"a\")) (b (get-buffer-create \"b\"))) (make-variable-buffer-local 'inhibit-modification-hooks) (set-default 'inhibit-modification-hooks nil) (set-buffer a) (list (let ((inhibit-modification-hooks 7)) (setq inhibit-modification-hooks 11) (let ((la (local-variable-p 'inhibit-modification-hooks a))) (set-buffer b) (setq inhibit-modification-hooks 19) (list la (local-variable-p 'inhibit-modification-hooks b) inhibit-modification-hooks (default-value 'inhibit-modification-hooks)))) (buffer-local-value 'inhibit-modification-hooks a) (buffer-local-value 'inhibit-modification-hooks b) (default-value 'inhibit-modification-hooks)))",
        "((nil t t t) nil t nil)",
    );
}

#[test]
fn forwarded_let_preceding_automatic_localization_keeps_its_default() {
    check(
        "(progn (set-default 'auto-window-vscroll nil) (list (let ((auto-window-vscroll t)) (make-variable-buffer-local 'auto-window-vscroll) (setq auto-window-vscroll nil) (list auto-window-vscroll (local-variable-p 'auto-window-vscroll) (default-value 'auto-window-vscroll))) auto-window-vscroll))",
        "((nil t t) nil)",
    );
    check(
        "(progn (make-variable-buffer-local 'inhibit-modification-hooks) (list (condition-case e (makunbound 'inhibit-modification-hooks) (error (car e))) (local-variable-p 'inhibit-modification-hooks)))",
        "(error nil)",
    );
}

#[test]
fn local_let_restores_its_owner_after_buffer_switch() {
    check(
        "(let ((a (get-buffer-create \"a\")) (b (get-buffer-create \"b\"))) (set-default 'inhibit-modification-hooks nil) (set-buffer a) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks nil) (list (let ((inhibit-modification-hooks 7)) (set-buffer b) (list inhibit-modification-hooks (buffer-local-value 'inhibit-modification-hooks a))) (buffer-local-value 'inhibit-modification-hooks a) inhibit-modification-hooks))",
        "((nil t) nil nil)",
    );
}

#[test]
fn default_let_survives_first_localization_and_nonlocal_exit() {
    check(
        "(progn (set-default 'inhibit-modification-hooks nil) (list (let ((inhibit-modification-hooks 7)) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks nil) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks))) inhibit-modification-hooks (default-value 'inhibit-modification-hooks) (local-variable-p 'inhibit-modification-hooks)))",
        "((nil t) nil nil t)",
    );
    check(
        "(progn (set-default 'inhibit-modification-hooks nil) (make-variable-buffer-local 'inhibit-modification-hooks) (list (catch 'done (let ((inhibit-modification-hooks 7)) (throw 'done inhibit-modification-hooks))) inhibit-modification-hooks (condition-case e (let ((inhibit-modification-hooks t)) (signal 'error '(\"expected\"))) (error (car e))) inhibit-modification-hooks))",
        "(t nil error nil)",
    );
}

#[test]
fn local_let_follows_kill_and_recreation() {
    check(
        "(with-temp-buffer (set-default 'inhibit-modification-hooks nil) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (list (let ((inhibit-modification-hooks nil)) (kill-local-variable 'inhibit-modification-hooks) inhibit-modification-hooks) inhibit-modification-hooks (local-variable-p 'inhibit-modification-hooks)))",
        "(nil nil nil)",
    );
    check(
        "(with-temp-buffer (set-default 'inhibit-modification-hooks nil) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (list (let ((inhibit-modification-hooks nil)) (kill-local-variable 'inhibit-modification-hooks) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks nil) inhibit-modification-hooks) inhibit-modification-hooks (local-variable-p 'inhibit-modification-hooks)))",
        "(nil t t)",
    );
}

#[test]
fn backtrace_reads_recreated_local_binding() {
    check(
        "(progn (fset 'bool-query (lambda () (backtrace--locals 1 'bool-query))) (fset 'bool-outer (lambda () (let ((inhibit-modification-hooks nil)) (kill-local-variable 'inhibit-modification-hooks) (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (bool-query)))) (with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (bool-outer)))",
        "((inhibit-modification-hooks . t))",
    );
}

#[test]
fn aliases_and_builtin_protection_follow_localized_identity() {
    check(
        "(progn (defvaralias 'bool-alias 'inhibit-modification-hooks) (with-temp-buffer (make-local-variable 'bool-alias) (setq bool-alias 7) (list bool-alias inhibit-modification-hooks (local-variable-p 'bool-alias) (default-value 'bool-alias) (condition-case e (makunbound 'inhibit-modification-hooks) (error e)) (condition-case e (defvaralias 'inhibit-modification-hooks 'bool-other) (error e)))))",
        "(t t t nil (error \"Built-in variable may not be unbound : inhibit-modification-hooks\") (error \"Don’t know how to make a buffer-local variable an alias: inhibit-modification-hooks\"))",
    );
    check(
        "(with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (let ((s (make-symbol \"inhibit-modification-hooks\"))) (set s 7) (make-local-variable s) (list (symbol-value s) inhibit-modification-hooks (default-value 'inhibit-modification-hooks))))",
        "(7 nil nil)",
    );
}

#[test]
fn native_printing_and_property_checks_observe_local_boolean() {
    check(
        "(with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (setq before-change-functions '(ignore)) (prin1 'value (current-buffer)) (put-text-property 1 2 'face 'bold) (list (equal (buffer-string) \"value\") (get-text-property 1 'face)))",
        "(t bold)",
    );
}

#[test]
fn noncanonical_localized_defaults_are_refused_without_mutation() {
    let mut automatic = Vm::new();
    assert_eq!(
        shown(
            &mut automatic,
            "(make-variable-buffer-local 'inhibit-modification-hooks) (let ((inhibit-modification-hooks t)) (condition-case e (set-default-toplevel-value 'inhibit-modification-hooks 13) (error (list (car e) inhibit-modification-hooks (default-toplevel-value 'inhibit-modification-hooks)))))"
        ),
        "(error t nil)"
    );
    assert_eq!(
        shown(
            &mut automatic,
            "(let ((inhibit-modification-hooks t)) (set-default-toplevel-value 'inhibit-modification-hooks t) inhibit-modification-hooks) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks))"
        ),
        "(t t)"
    );
    let mut vm = Vm::new();
    assert_eq!(
        shown(
            &mut vm,
            "(make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (condition-case e (set-default 'inhibit-modification-hooks 13) (error (list e inhibit-modification-hooks (default-value 'inhibit-modification-hooks))))"
        ),
        "((error \"VM2 localized Boolean default requires nil or t: inhibit-modification-hooks\") t nil)"
    );
    assert_eq!(
        shown(
            &mut vm,
            "(let ((inhibit-modification-hooks nil)) (condition-case e (set-default-toplevel-value 'inhibit-modification-hooks 13) (error (list (car e) inhibit-modification-hooks (default-toplevel-value 'inhibit-modification-hooks)))))"
        ),
        "(error nil nil)"
    );
    let mut vm = Vm::new();
    assert_eq!(
        shown(
            &mut vm,
            "(let ((inhibit-modification-hooks t)) (set-default-toplevel-value 'inhibit-modification-hooks 13) (condition-case e (make-local-variable 'inhibit-modification-hooks) (error (list (car e) inhibit-modification-hooks (local-variable-p 'inhibit-modification-hooks)))))"
        ),
        "(error t nil)"
    );
    assert_eq!(shown(&mut vm, "inhibit-modification-hooks"), "t");
    for name in ["undo-limit", "minibuffer-local-map"] {
        assert!(
            shown(
                &mut vm,
                &format!("(condition-case e (make-local-variable '{name}) (error e))")
            )
            .contains("VM2 cannot make built-in variable buffer-local")
        );
    }
}

#[test]
fn killed_buffer_ownership_is_reclaimed_and_defaults_survive_gc() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = "(with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks '(payload)) (garbage-collect) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks)))";
    assert_eq!(shown(&mut vm, source), "(t nil)");
    vm.collect_garbage();
    let baseline = vm.heap_stats();
    for _ in 0..16 {
        assert_eq!(shown(&mut vm, source), "(t nil)");
        vm.collect_garbage();
        let now = vm.heap_stats();
        assert_eq!(
            (now.live_objects, now.live_bytes),
            (baseline.live_objects, baseline.live_bytes)
        );
    }
    assert_eq!(
        shown(
            &mut vm,
            "(list inhibit-modification-hooks (default-value 'inhibit-modification-hooks))"
        ),
        "(nil nil)"
    );
}

#[test]
fn image_loaded_boolean_can_be_promoted_and_locals_cannot_be_published() {
    let store = std::env::temp_dir().join(format!("localized-bool-image-{}", std::process::id()));
    std::fs::create_dir(&store).unwrap();
    let mut source = Vm::new();
    source
        .eval_source("(put 'inhibit-modification-hooks 'image-note t)")
        .unwrap();
    let entry = source.compile_source("nil").unwrap();
    let key = source.publish_code_image(&store, entry, [83; 32]).unwrap();
    let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
    let (mut loaded, _) = Vm::from_code_image(image, [83; 32]).unwrap();
    assert_eq!(
        shown(
            &mut loaded,
            "(with-temp-buffer (make-local-variable 'inhibit-modification-hooks) (setq inhibit-modification-hooks t) (list inhibit-modification-hooks (default-value 'inhibit-modification-hooks) (get 'inhibit-modification-hooks 'image-note)))"
        ),
        "(t nil t)"
    );
    source
        .eval_source("(make-variable-buffer-local 'inhibit-modification-hooks)")
        .unwrap();
    assert!(source.publish_code_image(&store, entry, [83; 32]).is_err());
    std::fs::remove_dir_all(store).unwrap();
}
