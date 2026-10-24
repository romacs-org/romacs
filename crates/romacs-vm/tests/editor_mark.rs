use romacs_vm::Vm;

fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}

#[test]
fn ordinary_mark_is_buffer_owned_moving_and_gc_traced() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(printed(&mut vm, "(fboundp 'mark-marker)"), "t", "EDITOR_MARK_NATIVE_AUTHORITY");
    assert_eq!(printed(&mut vm, "(progn (setq a (get-buffer-create \"mark-a\")) (set-buffer a) (insert \"aλbcd\") (setq first-mark (mark-marker)) (set-marker first-mark 3) (goto-char 1) (insert \"界\") (list (eq first-mark (mark-marker)) (marker-position (mark-marker))))"), "(t 4)");
    vm.collect_garbage();
    assert_eq!(printed(&mut vm, "(progn (setq b (get-buffer-create \"mark-b\")) (set-buffer b) (list (marker-position (mark-marker)) mark-active))"), "(nil nil)", "EDITOR_MARK_AND_ACTIVATION_ARE_LOCAL");
    assert_eq!(printed(&mut vm, "(progn (set-buffer a) (setq mark-active t) (delete-region 1 3) (list (eq first-mark (mark-marker)) (marker-position first-mark) mark-active))"), "(t 2 t)");
    vm.collect_garbage();
    assert_eq!(printed(&mut vm, "(progn (set-buffer b) (list mark-active (marker-position first-mark) (eq (marker-buffer first-mark) a)))"), "(nil 2 t)");
    vm.eval_source("(kill-buffer a)").unwrap();
    vm.collect_garbage();
    assert_eq!(printed(&mut vm, "(list (marker-position first-mark) (marker-buffer first-mark))"), "(nil nil)", "KILLED_BUFFER_DETACHES_ORDINARY_MARK");
}

#[test]
fn region_endpoints_clip_to_narrowing_and_interactive_args_agree() {
    let mut vm = Vm::new();
    assert_eq!(printed(&mut vm, "(fboundp 'region-beginning)"), "t", "EDITOR_REGION_NATIVE_ENDPOINTS");
    assert_eq!(printed(&mut vm, "(progn (insert \"0123456789\") (set-marker (mark-marker) 2) (goto-char 9) (list (region-beginning) (region-end) (call-interactively (lambda (m) (interactive \"m\") m)) (call-interactively (lambda (a b) (interactive \"r\") (list a b)))))"), "(2 9 2 (2 9))", "INTERACTIVE_REGION_USES_ORDINARY_MARK");
    assert_eq!(printed(&mut vm, "(let ((current-prefix-arg (mark-marker))) (eq (call-interactively (lambda (x) (interactive \"P\") x)) (mark-marker)))"), "t", "PREFIX_MARKER_IS_NOT_A_POSITION_ARGUMENT");
    assert_eq!(printed(&mut vm, "(progn (narrow-to-region 4 8) (list (region-beginning) (region-end)))"), "(4 8)", "REGION_MARK_OUTSIDE_RESTRICTION_CLIPS");
    assert_eq!(printed(&mut vm, "(let ((transient-mark-mode t) (mark-even-if-inactive nil)) (condition-case e (region-end) (mark-inactive (car e))))"), "mark-inactive", "INACTIVE_REGION_IS_TYPED");
    assert_eq!(printed(&mut vm, "(let ((transient-mark-mode t) (mark-even-if-inactive nil) (mark-active t)) (region-end))"), "8");
    assert_eq!(printed(&mut vm, "(progn (defun use-region-p () t) (call-interactively (lambda (a b) (interactive \"R\") (list a b))))"), "(2 8)", "OPTIONAL_INTERACTIVE_REGION_PRESERVES_UNCLIPPED_MARK");
    assert_eq!(printed(&mut vm, "(progn (defun use-region-p () nil) (call-interactively (lambda (a b) (interactive \"R\") (list a b))))"), "(nil nil)");
    vm.eval_source("(set-marker (mark-marker) nil)").unwrap();
    assert_eq!(printed(&mut vm, "(condition-case e (region-end) (error (car e)))"), "error", "UNSET_MARK_HAS_NO_REGION");
}
