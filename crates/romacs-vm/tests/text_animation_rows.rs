//! A text animation's row is counted from the window start the frame's own
//! redisplay chose, not from a start Lisp read earlier.
//!
//! A command that moves point out of view (`end-of-buffer`, then
//! `beginning-of-buffer`) leaves `window-start` where it was until
//! redisplay scrolls, after `post-command-hook` has run.  The *Romacs*
//! landing used to compute its block's row there, so after M-> M-< the
//! display placed the block a screen above its text: the braille cells
//! showed and the scene's wordmark sat at the top edge.  The element now
//! names the buffer position the block starts at and the frame carries
//! the row for the start it was frozen with; a block out of view holds
//! its motion.
use romacs_vm::Vm;

/// (TOP MOVING) of the one animation of the next frame.
fn row(vm: &mut Vm, width: u32, height: u32) -> (i32, bool) {
    vm.frame_snapshot(width, height).unwrap();
    let animations = vm.text_animations().unwrap();
    assert_eq!(animations.len(), 1, "one animation in the frame");
    (animations[0].1.top, animations[0].1.moving)
}

#[test]
fn the_block_s_row_follows_the_start_redisplay_chooses() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&face).unwrap();
    let (width, height) = (column * 80, line * 24);
    vm.set_frame_geometry(width, height, line, column).unwrap();
    // A 10-row block starting the buffer, then 200 lines below it.
    vm.eval_source(&format!(
        r#"(progn (set-buffer (get-buffer-create "landing")) (insert {:?} {:?})
                 (set-window-buffer nil (current-buffer)) (setq mode-line-format nil)
                 (goto-char (point-min))
                 (setq romacs-animate--elements
                       (list (list 1 (selected-window) (current-buffer) (point-min) 20 10 25 t '(logo 0.5 0)))))"#,
        "logo\n".repeat(10),
        "text\n".repeat(200)
    ))
    .unwrap();
    assert_eq!(row(&mut vm, width, height), (0, true), "at the top");

    vm.eval_source("(goto-char (point-max))").unwrap();
    let (top, moving) = row(&mut vm, width, height);
    assert!(top <= -10 && !moving, "M->: the block is above the window and holds still: {top} {moving}");

    vm.eval_source("(goto-char (point-min))").unwrap();
    assert_eq!(row(&mut vm, width, height), (0, true), "M-<: back at the top, moving");

    // Part way: the window starts at the block's fourth row.
    vm.eval_source("(progn (goto-char (point-min)) (forward-line 5) (set-window-start nil (line-beginning-position -1)))")
        .unwrap();
    assert_eq!(row(&mut vm, width, height), (-3, true), "its last rows show");

    // A block below the window's start shows below its top row.
    vm.eval_source(
        r#"(progn (goto-char (point-min)) (forward-line 2)
                 (setq romacs-animate--elements
                       (list (list 1 (selected-window) (current-buffer) (point) 20 10 25 t '(logo 0.5 0))))
                 (goto-char (point-min)) (set-window-start nil (point-min)))"#,
    )
    .unwrap();
    assert_eq!(row(&mut vm, width, height), (2, true), "two rows down");
}
