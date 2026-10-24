use romacs_vm::Vm;

#[test]
fn kill_last_buffer_replacement_keeps_registered_name_and_identity() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(printed(&mut vm, r#"(progn
        (current-buffer)
        (setq victim (current-buffer))
        (kill-buffer victim)
        (setq survivor (current-buffer))
        (list (buffer-name survivor)
              (eq survivor (get-buffer "*scratch*"))
              (eq survivor (set-buffer "*scratch*"))
              (buffer-live-p victim)))"#), "(\"*scratch*\" t t nil)");
}

#[test]
fn kill_survivor_excludes_hidden_buffers_and_minibuffer_storage() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    assert_eq!(printed(&mut vm, r#"(progn
        (setq victim (current-buffer))
        (get-buffer-create " hidden")
        (setq survivor (get-buffer-create "visible"))
        (kill-buffer victim)
        (list (eq (current-buffer) survivor)
              (eq (window-buffer (selected-window)) survivor)
              (eq (window-buffer (minibuffer-window)) survivor)))"#), "(t t nil)");
}

#[test]
fn kill_buffer_unwind_skips_dead_saved_buffer() {
    let mut vm = Vm::new();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq a (get-buffer-create \"unwind-a\") b (get-buffer-create \"unwind-b\")) (set-buffer a) (with-current-buffer b (kill-buffer a)) (list (buffer-live-p (current-buffer)) (eq (current-buffer) b)))"
        ),
        "(t t)",
        "KILL unwind must not restore a killed saved buffer"
    );
}

#[test]
fn kill_buffer_hook_variables_are_special_before_first_call() {
    let mut vm = Vm::new();
    assert_eq!(
        printed(
            &mut vm,
            "(list (boundp 'kill-buffer-hook) (boundp 'kill-buffer-query-functions))"
        ),
        "(t t)",
        "KILL built-in hook variables must be declared at boot"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(let ((b (get-buffer-create \"boot-veto\")) (kill-buffer-query-functions (list (lambda () nil)))) (list (kill-buffer b) (buffer-live-p b)))"
        ),
        "(nil t)",
        "KILL lexical let of native query variable must veto"
    );
}

#[test]
fn kill_buffer_single_function_hooks() {
    let mut vm = Vm::new();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq victim (get-buffer-create \"single-hook\") ran nil kill-buffer-query-functions (lambda () t) kill-buffer-hook (lambda () (setq ran t))) (list (kill-buffer victim) ran))"
        ),
        "(t t)",
        "KILL hooks accept a single function"
    );
}

#[test]
fn kill_buffer_local_hook_defaults_and_point_unwind() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq victim (get-buffer-create \"local-hooks\") ran nil) (set-buffer victim) (insert \"abc\") (goto-char 2) (setq kill-buffer-query-functions (list (lambda () (setq ran (cons 'global ran)) t))) (make-local-variable 'kill-buffer-query-functions) (setq kill-buffer-query-functions (list (lambda () (goto-char 1) (setq ran (cons 'local ran)) t) t (lambda () nil))) (list (kill-buffer victim) (point) ran))"
        ),
        "(nil 2 (global local))",
        "KILL local default hooks and saved point unwind"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq kill-buffer-query-functions nil kill-buffer-hook '(t)) (kill-buffer victim))"
        ),
        "t",
        "KILL global t hook marker must terminate"
    );
}

#[test]
fn kill_buffer_refuses_minibuffer_destruction() {
    let mut vm = Vm::new();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq mini (window-buffer (minibuffer-window))) (list (kill-buffer mini) (buffer-live-p mini) (eq mini (window-buffer (minibuffer-window)))))"
        ),
        "(nil t t)",
        "KILL must not retarget the echo buffer into user text"
    );
}

fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}

#[test]
fn kill_buffer_lifecycle_removes_registry_identity() {
    let mut vm = Vm::new();
    let result = printed(
        &mut vm,
        "(progn (setq victim (get-buffer-create \"kill-target\")) (if (fboundp 'kill-buffer) (kill-buffer victim)) (eq victim (get-buffer-create \"kill-target\")))",
    );
    assert_eq!(
        result, "nil",
        "KILL lifecycle: killed registry identity must not be reused"
    );
}

#[test]
fn kill_buffer_queries_hooks_gc_veto_and_unwind() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    printed(
        &mut vm,
        "(progn (setq origin (get-buffer-create \"origin\") victim (get-buffer-create \"queries\") events nil) (set-buffer origin))",
    );
    let before = vm.heap_stats().collections;
    let result = printed(
        &mut vm,
        r#"(progn
      (defun churn-query () (let ((i 0)) (while (< i 5000) (cons i nil) (setq i (1+ i)))) (setq events (cons (eq (current-buffer) victim) events)) t)
      (defun veto-query () (setq events (cons 'veto events)) nil)
      (defun killed-hook () (setq events (cons (list 'hook (eq (current-buffer) victim)) events)))
      (setq kill-buffer-query-functions (list 'churn-query 'veto-query) kill-buffer-hook (list 'killed-hook))
      (list (kill-buffer victim) (buffer-live-p victim) (eq origin (current-buffer)) events))"#,
    );
    assert_eq!(
        result, "(nil t t (veto t))",
        "KILL query veto retains identity and unwinds current buffer"
    );
    assert!(
        vm.heap_stats().collections > before,
        "KILL callback must really collect"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq kill-buffer-query-functions (list 'churn-query)) (list (funcall 'kill-buffer victim) (bufferp victim) (buffer-live-p victim) (buffer-name victim) (get-buffer \"queries\") (eq origin (current-buffer)) events))"
        ),
        "(t t nil nil nil t ((hook t) t veto t))",
        "KILL hook order and dead identity after moving GC"
    );
    assert_eq!(
        printed(&mut vm, "(kill-buffer victim)"),
        "nil",
        "KILL dead buffer is idempotent"
    );
    printed(&mut vm, "(setq victim (get-buffer-create \"query-error\"))");
    assert_eq!(
        printed(
            &mut vm,
            "(progn (defun fail-query () (signal 'error '(\"query failed\"))) (setq kill-buffer-query-functions (list 'fail-query)) (condition-case err (kill-buffer victim) (error (list (car err) (buffer-live-p victim) (eq origin (current-buffer))))))"
        ),
        "(error t t)",
        "KILL query error must unwind without destruction"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq kill-buffer-query-functions nil kill-buffer-hook (list 'fail-query)) (condition-case err (kill-buffer victim) (error (list (car err) (buffer-live-p victim) (eq origin (current-buffer))))))"
        ),
        "(error t t)",
        "KILL hook error must unwind without destruction"
    );
}

#[test]
fn kill_buffer_cleans_windows_markers_overlays_and_locals() {
    let mut vm = Vm::new();
    let result = printed(
        &mut vm,
        r#"(progn
      (setq survivor (get-buffer-create "survivor") victim (get-buffer-create "cleanup"))
      (set-buffer victim) (insert "abc")
      (make-local-variable 'owned) (setq owned (vector "private"))
      (setq mark (copy-marker 2) overlay (make-overlay 1 3))
      (set-window-buffer (selected-window) victim)
      (kill-buffer victim))"#,
    );
    assert_eq!(result, "t", "KILL cleanup completes");
    assert_eq!(
        printed(&mut vm, "(marker-buffer mark)"),
        "nil",
        "KILL cleanup detaches marker owner"
    );
    let result = printed(
        &mut vm,
        r#"(list (buffer-live-p victim) (eq (window-buffer (selected-window)) victim)
            (marker-buffer mark) (marker-position mark) (overlay-start overlay)
            (eq victim (get-buffer-create "cleanup"))
            (condition-case err (set-buffer victim) (error 'dead))
            (condition-case err (switch-to-buffer victim) (error 'dead)))"#,
    );
    assert_eq!(
        result, "(nil nil nil nil nil nil dead dead)",
        "KILL cleanup: windows markers overlays registry and selection"
    );
    vm.collect_garbage();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (set-buffer \"cleanup\") (list (boundp 'owned) (buffer-string) (buffer-live-p victim)))"
        ),
        "(nil \"\" nil)",
        "KILL cleanup releases buffer locals across GC"
    );
    vm.frame_snapshot(640, 480).unwrap();
}

#[test]
fn kill_buffer_current_and_unibyte_survive_collection() {
    let mut vm = Vm::new();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq b (get-buffer-create \"binary-kill\")) (set-buffer b) (set-buffer-multibyte nil) (insert 255) (list (kill-buffer) (buffer-live-p b) (eq (current-buffer) b) (bufferp b)))"
        ),
        "(t nil nil t)",
        "KILL unibyte current buffer must select a live replacement"
    );
    vm.collect_garbage();
    assert_eq!(
        printed(
            &mut vm,
            "(list (buffer-live-p (current-buffer)) (buffer-name b))"
        ),
        "(t nil)",
        "KILL dead identity survives moving collection"
    );
}
