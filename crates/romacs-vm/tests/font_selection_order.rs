use romacs_vm::Vm;

#[test]
fn invalid_font_selection_order_signals_exact_data() {
    let mut vm = Vm::new();
    let actual = vm.eval_source("(condition-case e (internal-set-font-selection-order '(:width)) (error e))").unwrap().display().unwrap();
    assert_eq!(actual, "(error \"Invalid font sort order\" :width)", "INVALID_ORDER_EXACT_DATA");
}

#[test]
fn font_selection_order_accepts_duplicates_and_preserves_lisp_variable() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source("(progn (setq face-font-selection-order 'sentinel) (list (internal-set-font-selection-order '(:width :width :width :width)) face-font-selection-order (boundp 'face-font-sort-order) (fboundp 'face-font-sort-order)))").unwrap().display().unwrap(), "(nil sentinel nil nil)");
    use romacs_layout_input::FontSelectionAttribute::Width;
    assert_eq!(vm.font_selection_order(), [Width; 4], "PRIMITIVE_STORES_ORDER_FOR_SELECTION");
    let published = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(published.font_selection.order(), [Width; 4], "SNAPSHOT_CARRIES_SELECTION_ORDER");
}

#[test]
fn font_selection_order_snapshot_retains_history_across_unpublished_round_trip() {
    let mut vm = Vm::new();
    vm.eval_source("(internal-set-font-selection-order '(:height :height :height :height))").unwrap();
    let before = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(before.font_selection.shifts(), [23, 2, 9, 2]);
    vm.eval_source("(progn (internal-set-font-selection-order '(:weight :width :height :slant)) (internal-set-font-selection-order '(:height :height :height :height)))").unwrap();
    let after = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(before.font_selection.order(), after.font_selection.order());
    assert_eq!(after.font_selection.shifts(), [16, 2, 23, 2], "VM_SNAPSHOT_PRESERVES_SKIPPED_HISTORY");
    assert_eq!(after.font_selection.revision(), before.font_selection.revision() + 2);
    let unchanged = vm.font_selection_state();
    vm.eval_source("(internal-set-font-selection-order '(:height :height :height :height))").unwrap();
    assert_eq!(vm.font_selection_state(), unchanged, "REAPPLYING_ORDER_RETAINS_REVISION");
    vm.eval_source("(condition-case e (internal-set-font-selection-order '(:width :unknown :weight :slant)) (error nil))").unwrap();
    assert_eq!(vm.font_selection_state(), unchanged, "INVALID_ORDER_IS_ATOMIC");
}

#[test]
fn font_selection_order_validation_matrix() {
    let mut vm = Vm::new();
    for (input, expected) in [
        ("'(:width :height :weight :slant)", "nil"),
        ("nil", "(error \"Invalid font sort order\")"),
        ("'(:width :height :weight :slant :width)", "(error \"Invalid font sort order\" :width :height :weight :slant :width)"),
        ("'(:width :unknown :weight :slant)", "(error \"Invalid font sort order\" :width :unknown :weight :slant)"),
        ("42", "(wrong-type-argument listp 42)"),
        ("'(:width :height :weight . :slant)", "(error \"Invalid font sort order\" (:width :height :weight . :slant))"),
    ] {
        assert_eq!(vm.eval_source(&format!("(condition-case e (internal-set-font-selection-order {input}) (error e))")).unwrap().display().unwrap(), expected, "FONT_ORDER_CASE {input}");
    }
}
