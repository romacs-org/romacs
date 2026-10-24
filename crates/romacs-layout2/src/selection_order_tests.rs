use super::*;

#[test]
fn selection_order_vm_snapshot_skipped_round_trip_reaches_retained_resolver() {
    let mut vm = romacs_vm::Vm::new();
    let mut engine = LayoutEngine::new().unwrap();
    vm.eval_source("(internal-set-font-selection-order '(:height :height :height :height))").unwrap();
    let before = vm.frame_snapshot(400, 300).unwrap();
    engine.layout_frame(&before, 1, None).unwrap();
    assert_eq!(engine.resolver.selection_state(), before.font_selection);

    // No intermediate snapshot or layout: replaying only the raw final order
    // would miss the priority change from the intervening weight-first order.
    vm.eval_source("(progn (internal-set-font-selection-order '(:weight :width :height :slant)) (internal-set-font-selection-order '(:height :height :height :height)))").unwrap();
    let after = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(before.font_selection.order(), after.font_selection.order());
    assert_ne!(before.font_selection.shifts(), after.font_selection.shifts());
    engine.layout_frame(&after, 2, None).unwrap();
    assert_eq!(engine.resolver.selection_state(), after.font_selection,
        "LAYOUT_ADOPTS_VM_HISTORY_NOT_ONLY_RAW_FINAL_ORDER");

    // A second round trip has identical final raw order AND effective shifts.
    // The revision alone must reach the resolver to invalidate warm answers.
    vm.eval_source("(progn (internal-set-font-selection-order '(:weight :width :height :slant)) (internal-set-font-selection-order '(:height :height :height :height)))").unwrap();
    let final_frame = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(after.font_selection.order(), final_frame.font_selection.order());
    assert_eq!(after.font_selection.shifts(), final_frame.font_selection.shifts());
    assert_ne!(after.font_selection.revision(), final_frame.font_selection.revision());
    engine.layout_frame(&final_frame, 3, None).unwrap();
    assert_eq!(engine.resolver.selection_state(), final_frame.font_selection,
        "LAYOUT_ADOPTS_VM_REVISION_ON_IDENTICAL_FINAL_PRIORITIES");
}
