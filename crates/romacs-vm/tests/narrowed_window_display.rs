//! A window shows its buffer's accessible region (xdisp.c: the iterator
//! stops at ZV, and CLIP_TEXT_POS_FROM_MARKER keeps the start inside
//! BEGV..ZV): the published text ends at ZV, a start before BEGV shows
//! from BEGV, and the window end the layout records is at most ZV, as GNU
//! reports for a narrowed buffer.
use romacs_vm::Vm;

#[test]
fn a_narrowed_window_shows_and_ends_in_its_accessible_region() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(column * 80, line * 24, line, column).unwrap();
    vm.eval_source(r#"(progn (set-buffer (get-buffer-create "narrowed")) (set-window-buffer nil (current-buffer))
                             (dotimes (i 100) (insert (format "line %d\n" i)))
                             (set-window-start nil 1) (narrow-to-region 20 60) (goto-char (point-min)))"#).unwrap();
    let snapshot = vm.frame_snapshot(column * 80, line * 24).unwrap();
    let window = &snapshot.windows[0].snapshot;
    // ASCII text: BEGV 20 and ZV 60 are bytes 19 and 59; the published
    // text starts at the buffer byte `published_base' names.
    let base = vm.published_base(window.id);
    assert_eq!((base + window.start_byte, base + window.text.utf8.len()), (19, 59), "start clipped to BEGV, text ends at ZV");
    let (_, ends) = romacs_layout2::layout_frame(&snapshot, 1).unwrap();
    vm.display_apply_layout(&ends).unwrap();
    assert_eq!(vm.eval_source("(list (window-end) (point-max))").unwrap().display().unwrap(), "(60 60)");
}
