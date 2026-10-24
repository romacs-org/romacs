use romacs_vm::Vm;

#[test]
fn local_clear_retention_stabilizes_across_32_collections() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_stats().live_objects, 0, "no startup heap allocation");
    let source = "(with-temp-buffer (setq buffer-file-name (copy-sequence \"visit\")) (set (make-local-variable 'bl-retain) (list (make-vector 17 nil) (copy-sequence \"payload\"))) (put 'bl-retain 'permanent-local t) (setq fill-column 33) (kill-local-variable 'fill-column) (kill-all-local-variables) (list (equal buffer-file-name \"visit\") (equal (car (cdr bl-retain)) \"payload\") (local-variable-p 'fill-column)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t nil)");
    vm.collect_garbage();
    let baseline = vm.heap_stats();
    for _ in 0..32 {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t nil)");
        vm.collect_garbage();
        let now = vm.heap_stats();
        assert!(now.collections > baseline.collections);
        assert_eq!((now.live_objects, now.live_bytes), (baseline.live_objects, baseline.live_bytes), "cleared local retention must stabilize");
    }
    eprintln!("buffer locals retention: {} live objects / {} live bytes across 32 cycles", baseline.live_objects, baseline.live_bytes);
}


fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let result = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(result.as_deref(), Ok(expected), "buffer-local contract: {source}");
}

#[test]
fn mode_fields_are_inherently_local_and_reset_unconditionally() {
    check("(with-temp-buffer (list (local-variable-p 'major-mode) (local-variable-p 'mode-name) major-mode (equal mode-name \"Fundamental\")))", "(t t fundamental-mode t)");
    check("(with-temp-buffer (setq major-mode 'other mode-name \"Other\") (put 'major-mode 'permanent-local t) (kill-local-variable 'major-mode) (kill-all-local-variables) (list major-mode (equal mode-name \"Fundamental\")))", "(fundamental-mode t)");
}

#[test]
fn clear_one_removes_value_and_flag_only() {
    check("(with-temp-buffer (setq fill-column 31 buffer-file-name \"visit\") (kill-local-variable 'fill-column) (list fill-column (local-variable-p 'fill-column) buffer-file-name (local-variable-p 'buffer-file-name)))", "(70 nil \"visit\" t)");
}
#[test]
fn global_is_not_a_local_and_automatic_is_not_yet_local() {
    check("(with-temp-buffer (setq bl-global 4) (make-variable-buffer-local 'bl-auto) (list (local-variable-p 'bl-global) (local-variable-p 'bl-auto) (local-variable-if-set-p 'bl-auto)))", "(nil nil t)");
}
#[test]
fn other_buffer_read_and_local_inventory() {
    check("(let ((b (get-buffer-create \"local-other\"))) (setq bl-x 7) (with-current-buffer b (set (make-local-variable 'bl-x) 12)) (list (buffer-local-value 'bl-x b) bl-x (local-variable-p 'bl-x b) (cdr (assq 'bl-x (buffer-local-variables b)))))", "(12 7 t 12)");
}
#[test]
fn kill_all_keeps_permanent_and_file_visit_state() {
    check("(with-temp-buffer (setq bl-global 7) (set (make-local-variable 'bl-plain) 3) (set (make-local-variable 'bl-permanent) 5) (put 'bl-permanent 'permanent-local t) (setq buffer-file-name \"visit\" fill-column 31) (kill-all-local-variables) (list (local-variable-p 'bl-plain) bl-permanent (local-variable-p 'bl-permanent) bl-global fill-column buffer-file-name))", "(nil 5 t 7 70 \"visit\")");
}
#[test]
fn kill_one_ignores_permanent_property() {
    check("(with-temp-buffer (setq bl-p 7) (set (make-local-variable 'bl-p) 8) (put 'bl-p 'permanent-local t) (kill-local-variable 'bl-p) (list bl-p (local-variable-p 'bl-p)))", "(7 nil)");
}
#[test]
fn kill_all_permanent_argument_and_hook_order() {
    check("(with-temp-buffer (set (make-local-variable 'bl-p) 8) (put 'bl-p 'permanent-local t) (setq bl-seen nil) (let ((change-major-mode-hook (list (lambda () (setq bl-seen (local-variable-p 'bl-p)))))) (kill-all-local-variables t)) (list bl-seen (local-variable-p 'bl-p)))", "(t nil)");
}
#[test]
fn kill_all_does_not_touch_other_buffer() {
    check("(let ((b (get-buffer-create \"untouched\"))) (with-current-buffer b (setq fill-column 43)) (with-temp-buffer (setq fill-column 32) (kill-all-local-variables)) (buffer-local-value 'fill-column b))", "43");
}
#[test]
fn permanent_hook_void_condition_matches_pinned_gnu() {
    check("(condition-case e (with-temp-buffer (make-local-variable 'bl-h) (makunbound 'bl-h) (put 'bl-h 'permanent-local 'permanent-local-hook) (kill-all-local-variables)) (error (list (car e) (car (cdr e)) (equal (symbol-name (car (cdr (cdr e)))) \"unbound\") (eq (car (cdr (cdr e))) 'unbound))))", "(wrong-type-argument arrayp t nil)");
}
#[test]
fn builtin_permanence_uses_symbol_identity() {
    check("(with-temp-buffer (let ((s (make-symbol \"truncate-lines\"))) (set s 1) (make-local-variable s) (set s 2) (kill-all-local-variables) (symbol-value s)))", "1");
    check("(with-temp-buffer (set (make-local-variable 'buffer-file-coding-system) 'raw-text) (kill-all-local-variables) (list buffer-file-coding-system (local-variable-p 'buffer-file-coding-system)))", "(raw-text t)");
}
#[test]
fn fundamental_mode_name_identity_is_shared() {
    check("(with-temp-buffer (let ((name mode-name)) (kill-all-local-variables) (list (eq name mode-name) (eq mode-name (default-value 'mode-name)))))", "(t nil)");
}

#[test]
fn buffer_local_value_checks_symbol_before_buffer() {
    check("(condition-case e (buffer-local-value 3 nil) (wrong-type-argument e))", "(wrong-type-argument symbolp 3)");
}

#[test]
fn local_wrong_types_and_void_value() {
    check("(list (condition-case e (local-variable-p 3) (wrong-type-argument (car e))) (condition-case e (buffer-local-value 'bl-void (current-buffer)) (void-variable e)) (condition-case e (kill-local-variable 3) (wrong-type-argument (car e))))", "(wrong-type-argument (void-variable bl-void) wrong-type-argument)");
}
