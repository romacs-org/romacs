use romacs_vm::Vm;

fn boot() -> Vm {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    for file in ["emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote", "subr"] {
        vm.load_file(&root.join(format!("gnu-lisp/{file}.el"))).unwrap();
    }
    vm
}
fn check(vm: &mut Vm, source: &str, expected: &str, message: &str) {
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "{message}: {actual:?}");
}

#[test]
fn completed_subr_temp_buffer() {
    check(&mut boot(), "(with-temp-buffer 42)", "42", "CUSTOMBOOT_GNU_WITH_TEMP_BUFFER_RETURNS_BODY");
}

#[test]
fn normal_and_inhibited_lifetime_hooks() {
    let mut vm = boot();
    check(&mut vm, r#"(let ((rows nil))
      (dolist (inhibit '(nil t))
        (let* ((events nil)
          (buffer-list-update-hook (list (lambda () (push 'list events))))
          (kill-buffer-hook (list (lambda () (push 'kill events))))
          (kill-buffer-query-functions (list (lambda () (push 'query events) t)))
          (b (get-buffer-create (if inhibit "customboot-inhibited" "customboot-normal") inhibit)))
          (let ((created (reverse events)))
            (kill-buffer b)
            (push (list :inhibit inhibit :created created :all-events (reverse events) :live (buffer-live-p b)) rows))))
      (reverse rows))"#,
      "((:inhibit nil :created (list) :all-events (list query kill list) :live nil) (:inhibit t :created nil :all-events nil :live nil))",
      "BUFFER_LIFECYCLE_HOOK_INHIBITION");
    check(&mut vm, r#"(let* ((events nil) (buffer-list-update-hook (list (lambda () (push 'list events)))) (b (generate-new-buffer "pass-inhibit" t))) (kill-buffer b) events)"#, "nil", "BUFFER_LIFECYCLE_GENERATE_PASSES_INHIBIT");
}

#[test]
fn query_veto_and_dead_identity() {
    let mut vm = boot();
    check(&mut vm, r#"(let* ((kill-buffer-query-functions (list (lambda () nil))) (b (get-buffer-create "customboot-veto"))) (list :veto (kill-buffer b) :live (buffer-live-p b)))"#, "(:veto nil :live t)", "BUFFER_LIFECYCLE_QUERY_VETO");
    check(&mut vm, r#"(let ((b (get-buffer-create "customboot-dead" t))) (kill-buffer b) (list :dead-returned (eq b (get-buffer-create b nil))))"#, "(:dead-returned t)", "BUFFER_LIFECYCLE_DEAD_IDENTITY");
}

#[test]
fn cleanup_and_hook_current_buffer() {
    let mut vm = boot();
    check(&mut vm, r#"(let* ((kill-buffer-query-functions nil) (b (get-buffer-create "cleanup")) (seen nil) (m nil) (old (current-buffer)))
      (with-current-buffer b
        (insert "abc") (setq m (copy-marker 2))
        (set (make-local-variable 'lifecycle-local) 9)
        (let ((kill-buffer-hook (list (lambda () (setq seen (list (eq (current-buffer) b) lifecycle-local (marker-position m)))))))
          (kill-buffer b)))
      (list seen (buffer-name b) (buffer-live-p b) (marker-buffer m) (marker-position m) (eq old (current-buffer))))"#,
      "((t 9 2) nil nil nil nil t)", "BUFFER_LIFECYCLE_DEAD_CLEANUP");
}

#[test]
fn hook_error_restores_current_buffer() {
    check(&mut boot(), r#"(let ((old (current-buffer)) (b (get-buffer-create "query-error")) (kill-buffer-query-functions (list (lambda () (error "query")))))
      (condition-case nil (kill-buffer b) (error nil)) (list (eq old (current-buffer)) (buffer-live-p b)))"#, "(t t)", "BUFFER_LIFECYCLE_QUERY_ERROR_UNWIND");
}

#[test]
fn windows_reanchor_after_kill() {
    check(&mut boot(), r#"(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2))
      (w (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))
      (b (get-buffer-create "shown-kill" t)))
      (set-window-buffer w b) (kill-buffer b)
      (let ((positions (list (integerp (window-point w)) (integerp (window-start w)))))
        (select-window w) (list positions (buffer-live-p (current-buffer)))))"#,
      "((t t) t)", "BUFFER_LIFECYCLE_WINDOW_REANCHOR");
}

#[test]
fn dead_buffer_cannot_gain_overlay() {
    check(&mut boot(), "(condition-case nil (progn (make-overlay 1 1 (with-temp-buffer (current-buffer))) nil) (error t))", "t", "BUFFER_LIFECYCLE_DEAD_OVERLAY_REJECTED");
}

#[test]
fn native_get_buffer_create_passes_inhibit() {
    check(&mut Vm::new(), r#"(let* ((events nil) (buffer-list-update-hook (list (lambda () (setq events t)))) (b (get-buffer-create "native-inhibit" t))) (kill-buffer b) events)"#, "nil", "BUFFER_LIFECYCLE_NATIVE_CREATE_INHIBIT");
}

#[test]
fn gnu_subr_defines_generate_new_buffer_and_forwards_inhibition() {
    let mut vm = boot();
    check(&mut vm, "(fboundp 'generate-new-buffer)", "t", "GENERATE_NEW_BUFFER_DEFINED_BY_GNU_SUBR_EL");
    check(&mut vm, r#"(let* ((events nil)
      (kill-buffer-query-functions nil)
      (buffer-list-update-hook (list (lambda () (push 'list events))))
      (first (generate-new-buffer "gnu-generated"))
      (second (funcall 'generate-new-buffer "gnu-generated" t)))
      (list (buffer-name first) (buffer-name second)
        (eq first (get-buffer "gnu-generated"))
        (eq second (get-buffer "gnu-generated<2>"))
        (eq first second) events
        (progn (kill-buffer second) events)
        (progn (kill-buffer first) events)))"#,
      "(\"gnu-generated\" \"gnu-generated<2>\" t t nil (list) (list) (list list))",
      "GENERATE_NEW_BUFFER_GNU_FRESH_NAMES_AND_INHIBIT_FORWARDING");
}

#[test]
fn inhibition_belongs_to_creation_not_lookup() {
    check(&mut boot(), r#"(let* ((events nil) (buffer-list-update-hook (list (lambda () (push 'list events))))
      (kill-buffer-query-functions nil) (kill-buffer-hook (list (lambda () (push (list 'kill (buffer-name)) events))))
      (normal (get-buffer-create "persistent-normal")) (inhibited (get-buffer-create "persistent-inhibit" t)))
      (get-buffer-create normal t) (get-buffer-create "persistent-normal" t)
      (get-buffer-create inhibited nil) (get-buffer-create "persistent-inhibit" nil)
      (kill-buffer normal) (kill-buffer inhibited) (reverse events))"#,
      "(list (kill \"persistent-normal\") list)", "BUFFER_LIFECYCLE_INHIBIT_PERSISTENT");
}

#[test]
fn overlay_recenter_is_gnus_checked_noop() {
    check(&mut boot(), "(list (overlay-recenter -100) (overlay-recenter (copy-marker (point))) (overlay-recenter #x100000000000000000000) (condition-case e (overlay-recenter nil) (wrong-type-argument (car e))) (condition-case e (overlay-recenter (make-marker)) (error (car e))))", "(nil nil nil wrong-type-argument error)", "BUFFER_LIFECYCLE_OVERLAY_RECENTER");
}

#[test]
fn binary_scratch_replacement_does_not_panic() {
    let result = std::panic::catch_unwind(|| {
        check(&mut Vm::new(), r#"(progn (set-buffer (get-buffer-create "*scratch*" t)) (buffer-encode-utf8) (let ((b (get-buffer-create "kill-beside-bytes" t))) (set-buffer b) (list (kill-buffer b) (buffer-live-p (current-buffer)))))"#,
          "(t t)", "BUFFER_LIFECYCLE_BINARY_REPLACEMENT");
    });
    assert!(result.is_ok(), "BUFFER_LIFECYCLE_BINARY_REPLACEMENT: public buffer representation change must not panic");
}

/// buffer.c Fother_buffer against pinned GNU: the frame's
/// `buffer-predicate', hidden, dead and visible buffers, VISIBLE-OK and
/// FRAME. The predicate keeps the buffers each engine starts with out.
#[test]
fn other_buffer_matches_pinned_gnu() {
    let source = r#"(append (prin1-to-string
      (let* ((names (lambda (v) (if (bufferp v) (buffer-name v) v)))
             (a (get-buffer-create "ob-a")) (b (get-buffer-create "ob-b")))
        (get-buffer-create " ob-hidden")
        (modify-frame-parameters nil (list (cons 'buffer-predicate (lambda (x) (string-match "ob-" (buffer-name x))))))
        (list (funcall names (other-buffer a))
              (funcall names (other-buffer))
              (funcall names (other-buffer 'not-a-buffer))
              (progn (set-window-buffer (selected-window) a) (funcall names (other-buffer b)))
              (funcall names (other-buffer b t))
              (let ((c (get-buffer-create "ob-c")))
                (list (funcall names (other-buffer b)) (progn (kill-buffer c) (funcall names (other-buffer b)))))
              (funcall names (other-buffer nil nil (selected-frame)))
              (condition-case e (other-buffer nil nil 'nope) (error e))
              (progn (modify-frame-parameters nil (list (cons 'buffer-predicate (lambda (x) (eq x b)))))
                     (funcall names (other-buffer a)))
              (subrp (symbol-function 'other-buffer))))) nil)"#;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    check(&mut vm, source, &String::from_utf8(output.stdout).unwrap(), "OTHER_BUFFER_GNU");
}
