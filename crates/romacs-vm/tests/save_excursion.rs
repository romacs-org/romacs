use romacs_vm::Vm;

fn check(source: &str, expected: &str, key: &str) {
    let mut vm = Vm::new();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "{key}: {actual:?}");
}

#[test]
fn body_and_point() {
    check("(with-temp-buffer (insert \"aλbc\") (goto-char 3) (list (save-excursion) (save-excursion (goto-char 1) 42) (point)))", "(nil 42 3)", "SAVE_EXCURSION_BODY_POINT");
}
#[test]
fn moving_marker_and_left_bias() {
    check("(with-temp-buffer (insert \"aλbc\") (goto-char 3) (save-excursion (goto-char 1) (insert \"Ж\")) (let ((p (point))) (save-excursion (insert \"Q\")) (list p (point))))", "(4 4)", "SAVE_EXCURSION_MOVING_LEFT_MARKER");
}
#[test]
fn restores_buffer() {
    check("(with-temp-buffer (insert \"abc\") (goto-char 2) (let ((b (current-buffer))) (save-excursion (set-buffer (get-buffer-create \"other\" t)) (insert \"xyz\")) (list (eq b (current-buffer)) (point))))", "(t 2)", "SAVE_EXCURSION_BUFFER");
}
#[test]
fn abnormal_exits_and_nested() {
    check("(with-temp-buffer (insert \"abcdef\") (goto-char 2) (let ((r (catch 'x (save-excursion (goto-char 3) (save-excursion (goto-char 5)) (throw 'x (point)))))) (list r (point) (condition-case nil (save-excursion (goto-char 6) (error \"stop\")) (error (point))))))", "(3 2 2)", "SAVE_EXCURSION_UNWIND");
}
#[test]
fn dead_original_is_not_restored() {
    check("(let* ((b (get-buffer-create \"saved\" t)) (other (get-buffer-create \"survivor\" t))) (set-buffer b) (save-excursion (kill-buffer b) (set-buffer other)) (list (eq other (current-buffer)) (buffer-live-p b)))", "(t nil)", "SAVE_EXCURSION_DEAD_BUFFER");
}
#[test]
fn narrowing_clips_restored_point() {
    check("(with-temp-buffer (insert \"abcdef\") (goto-char 2) (save-excursion (narrow-to-region 4 6)) (point))", "4", "SAVE_EXCURSION_NARROWING");
}
#[test]
fn native_restore_ignores_redefinitions() {
    // Point MUST move inside the excursion, or restoring 2 is indistinguishable
    // from never restoring: a mutant routing restoration through the mutable
    // `goto-char' function cell survived this test until the move was added.
    // Pinned GNU returns 2 for this fixture.
    check("(with-temp-buffer (insert \"abc\") (goto-char 2) (save-excursion (goto-char 1) (fset 'goto-char (lambda (&rest args) 99))) (point))", "2", "SAVE_EXCURSION_NATIVE_RESTORE");
}
#[test]
fn selected_window_point() {
    check("(progn (set-buffer (get-buffer-create \"shown\" t)) (insert \"abcdef\") (goto-char 2) (set-window-buffer nil (current-buffer)) (let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (w (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) (save-excursion (goto-char 5) (select-window w)) (list (point) (window-point a) (eq w (selected-window)))))", "(2 2 t)", "SAVE_EXCURSION_WINDOW_POINT");
}
#[test]
fn gc_keeps_saved_state() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let actual = vm.eval_source("(with-temp-buffer (insert \"aλbc\") (goto-char 3) (save-excursion (let ((n 0)) (while (< n 3000) (cons n n) (setq n (1+ n)))) (goto-char 1) (insert \"Ж\")) (point))").and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("4"), "SAVE_EXCURSION_GC_ROOTS: {actual:?}");
    assert!(vm.heap_stats().collections > before, "SAVE_EXCURSION_GC_RAN");
}

#[test]
fn gnu_test_remove_overlays_after_subr() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("lisp/gnu"), root.join("lisp/gnu/emacs-lisp")]).unwrap();
    for file in ["emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote", "subr"] {
        vm.load_file(&root.join(format!("lisp/gnu/{file}.el"))).unwrap();
    }
    vm.load_file(&root.join("test/gnu/harness/romacs-mini-ert.el")).unwrap();
    let file = std::fs::read_to_string(root.join("test/gnu/test/src/buffer-tests.el")).unwrap();
    let start = file.find("(ert-deftest test-remove-overlays ()").unwrap();
    let end = start + file[start..].find("\n(defun test-kill-buffer-auto-save").unwrap();
    vm.eval_source(&file[start..end]).unwrap();
    let actual = vm.eval_source("(progn (romacs-gnu-tests--run 0) romacs-gnu-tests--last)").and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("(passed)"), "SAVE_EXCURSION_GNU_REMOVE_OVERLAYS: {actual:?}");
}

#[test]
fn window_not_originally_showing_buffer() {
    check("(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (w (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) (set-buffer (get-buffer-create \"not-shown\" t)) (insert \"abcdef\") (goto-char 2) (save-excursion (goto-char 5) (set-window-buffer a (current-buffer)) (select-window w)) (list (point) (window-point a) (eq w (selected-window))))", "(2 5 t)", "SAVE_EXCURSION_WINDOW_NOT_SHOWN");
}
#[test]
fn window_no_longer_showing_buffer() {
    check("(progn (set-buffer (get-buffer-create \"shown\" t)) (insert \"abcdef\") (goto-char 2) (set-window-buffer nil (current-buffer)) (let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (w (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))) (save-excursion (goto-char 5) (let ((b (get-buffer-create \"replacement\" t))) (with-current-buffer b (insert \"abcdef\")) (set-window-buffer a b) (set-window-point a 4)) (select-window w)) (list (point) (window-point a) (eq w (selected-window)))))", "(2 4 t)", "SAVE_EXCURSION_WINDOW_CHANGED");
}

#[test]
fn excursion_restores_point_in_unibyte_buffers() {
    // A unibyte buffer keeps its saved point in a marker, as GNU does.
    // Pinned GNU answers (42 2) for this fixture.
    check("(with-temp-buffer (set-buffer-multibyte nil) (insert \"abc\") (goto-char 2) (condition-case c (list (save-excursion (goto-char 1) 42) (point)) (error c)))", "(42 2)", "SAVE_EXCURSION_UNIBYTE");
}
