//! A window's recorded end distance must not survive a buffer change.
//!
//! GNU sets `w->window_end_pos = 0` when it assigns a window's buffer
//! (`src/window.c:4360`), before the KEEP-MARGINS and same-buffer guard, so the
//! reset is unconditional. VM2 cleared an `end_valid` flag but left the distance
//! itself, so after swapping to a short buffer `(window-end)` reported a
//! position derived from the PREVIOUS buffer's length.
//!
//! This exercises the real producer chain rather than a hand-built snapshot:
//! the VM publishes a frame, `romacs-layout2` lays it out and reports where each
//! window's text ends, that result is applied back through the journal, and only
//! then is `(window-end)` asked. An equivalent in-crate regression exists, but
//! this is the one that reproduces the reviewer's original observation through
//! every layer, which is why it belongs in the repository and not in a scratch
//! directory.
use romacs_vm::Vm;

#[test]
fn buffer_swap_resets_the_window_end_after_real_layout_and_journal() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(column * 80, line * 24, line, column).unwrap();
    vm.eval_source(&format!(
        r#"(progn (set-buffer (get-buffer-create "long-end")) (insert {:?})
                 (set-window-buffer nil (current-buffer)) (setq mode-line-format nil)
                 ;; Point at the top, so redisplay keeps window start 1 the
                 ;; way GNU's would; left at the end it recenters there and
                 ;; the whole tail becomes visible.
                 (goto-char (point-min)))"#,
        "x\n".repeat(1000)
    ))
    .unwrap();

    let snapshot = vm.frame_snapshot(column * 80, line * 24).unwrap();
    let (_, ends) = romacs_layout2::layout_frame(&snapshot, 1).unwrap();
    let end = ends
        .iter()
        .find(|end| end.id == snapshot.windows[0].snapshot.id)
        .unwrap()
        .end;
    assert!(end > 0 && end < 2000, "real layout must leave undisplayed text: {end}");
    vm.display_apply_layout(&ends).unwrap();

    let before = vm.eval_source("(window-end)").unwrap();
    assert_eq!(before.display().unwrap().parse::<usize>().unwrap(), end + 1);

    vm.eval_source(r#"(set-window-buffer nil (get-buffer-create "empty-end"))"#).unwrap();
    let after = vm.eval_source("(window-end)").unwrap();
    assert_eq!(
        after.display().unwrap(),
        "1",
        "a buffer change must discard the previous buffer's end distance"
    );
}
