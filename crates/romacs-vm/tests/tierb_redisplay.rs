//! Tier B: redisplay keeps point fully visible, as GNU's `redisplay_window`
//! does (`src/xdisp.c:20470`). With GNU's defaults -- `scroll-conservatively`
//! 0, `scroll-step` 0, `scroll-margin` 0 -- a point that is not on a fully
//! visible row makes redisplay RECENTER: `centering_position =
//! window_box_height (w) / 2` (`xdisp.c:21230`) and the start moves back with
//! `move_it_vertically_backward` (`xdisp.c:21245`). A forced start
//! (`set-window-start`) is honoured instead and point moves to the row at half
//! the window height (`xdisp.c:20742-20901`).
//!
//! Every expected start/point below is GNU 32.0.50's own answer, recorded on a
//! tty frame with `crates/romacs-vm-gui/evidence/tierb/gnu/scroll.el`
//! (transcript `scroll.txt` beside it). A tty line is one "pixel", so these
//! cases run at a 20px line with text heights that are exact multiples; the
//! pixel cases at the bottom follow the same `move_it_vertically_backward`
//! arithmetic and are pinned against GNU's PGTK frame in `gui-scroll.txt`.
use romacs_vm::Vm;

const LINE: u32 = 20;
const COLUMN: u32 = 10;

/// `line 1` .. `line 200`, one per line, exactly as the GNU probe inserts.
fn long_buffer(text_rows: u32) -> Vm {
    let mut vm = Vm::new();
    // Frame = text rows + one mode-line row + the one-line minibuffer.
    let height = (text_rows + 2) * LINE;
    vm.set_frame_geometry(80 * COLUMN, height, LINE, COLUMN).unwrap();
    vm.eval_source(
        r#"(progn (set-buffer (get-buffer-create "long"))
             (let ((i 1)) (while (<= i 200) (insert (format "line %d\n" i)) (setq i (1+ i))))
             (setq mode-line-format '("%b"))
             (set-window-buffer nil (current-buffer))
             ;; GNU's probe starts every case from window start 1 with point
             ;; displayed; insertion leaves point at the end.
             (goto-char (point-min)))"#,
    )
    .unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

fn goto_line(vm: &mut Vm, line: usize) {
    eval(vm, &format!("(progn (goto-char (point-min)) (forward-line {}) (point))", line - 1));
}

/// Redisplay once, then report (start line, point line) as GNU's probe does.
fn redisplay(vm: &mut Vm, text_rows: u32) -> (usize, usize) {
    let height = (text_rows + 2) * LINE;
    vm.frame_snapshot(80 * COLUMN, height).unwrap();
    let start = eval(vm, "(let ((s (window-start)) (p (point)) (n 1)) (goto-char 1) (while (< (point) s) (forward-line 1) (setq n (1+ n))) (goto-char p) n)");
    let point = eval(vm, "(let ((p (point)) (n 1)) (goto-char 1) (while (and (< (point) p) (= (forward-line 1) 0) (<= (point) p)) (setq n (1+ n))) (goto-char p) n)");
    (start.parse().unwrap(), point.parse().unwrap())
}

#[test]
fn conservative_reveal_preserves_point_and_minimizes_scrolling() {
    let mut vm = long_buffer(12);
    vm.set_collection_floor(1);
    eval(&mut vm, "(setq scroll-conservatively 101)");
    assert_eq!(redisplay(&mut vm, 12), (1, 1));
    for target in 2..=201 {
        goto_line(&mut vm, target);
        assert_eq!(redisplay(&mut vm, 12), (target.saturating_sub(11).max(1), target));
        assert_eq!(redisplay(&mut vm, 12), (target.saturating_sub(11).max(1), target), "paint is stable");
    }
    for target in (1..=200).rev() {
        goto_line(&mut vm, target);
        assert_eq!(redisplay(&mut vm, 12), (target.min(190), target));
    }
}

#[test]
fn conservative_threshold_and_partial_rows_remain_explicit() {
    let mut vm = long_buffer(12);
    eval(&mut vm, "(setq scroll-conservatively 1)");
    goto_line(&mut vm, 13);
    assert_eq!(redisplay(&mut vm, 12), (2, 13));
    goto_line(&mut vm, 40);
    assert_eq!(redisplay(&mut vm, 12), (34, 40), "large jumps retain configured centering");
    eval(&mut vm, "(setq scroll-conservatively 101)");
    goto_line(&mut vm, 1);
    assert_eq!(redisplay(&mut vm, 12), (1, 1));
    goto_line(&mut vm, 13);
    vm.frame_snapshot(800, 290).unwrap();
    assert_eq!(redisplay(&mut vm, 12), (2, 13), "a partial row cannot hold the caret");
    eval(&mut vm, "(narrow-to-region 65 (point-max)) (goto-char (point-min))");
    let point = eval(&mut vm, "(point)");
    vm.frame_snapshot(800, 280).unwrap();
    assert_eq!(eval(&mut vm, "(point)"), point);
    assert_eq!(eval(&mut vm, "(>= (window-start) (point-min))"), "t");
}

#[test]
fn point_below_the_window_recenters_like_gnu() {
    // GNU tty, body 12: targets 22 23 24 30 100 200 201 -> starts 16 17 18 24 94 194 195.
    for (target, start) in [(22, 16), (23, 17), (24, 18), (30, 24), (100, 94), (200, 194), (201, 195)] {
        let mut vm = long_buffer(12);
        redisplay(&mut vm, 12);
        goto_line(&mut vm, target);
        assert_eq!(redisplay(&mut vm, 12), (start, target), "body 12, point on line {target}");
    }
}

#[test]
fn point_above_the_window_recenters_like_gnu() {
    // GNU: start forced to line 100, point moved to 106 (visible), then point
    // to line 99 above the window: start 93, point row 6.
    let mut vm = long_buffer(12);
    eval(&mut vm, "(progn (goto-char (point-min)) (forward-line 99) (set-window-start nil (point)))");
    goto_line(&mut vm, 106);
    assert_eq!(redisplay(&mut vm, 12), (100, 106), "visible point keeps a forced start");
    goto_line(&mut vm, 99);
    assert_eq!(redisplay(&mut vm, 12), (93, 99));
}

#[test]
fn visible_point_never_moves_the_start() {
    // body 11: line 11 is on the last full row (start stays 1); line 12 is
    // one row past it and recenters to start 7 (GNU "half" cases 11 and 12).
    let mut vm = long_buffer(11);
    goto_line(&mut vm, 11);
    assert_eq!(redisplay(&mut vm, 11), (1, 11));
    goto_line(&mut vm, 12);
    assert_eq!(redisplay(&mut vm, 11), (7, 12));
    goto_line(&mut vm, 13);
    assert_eq!(redisplay(&mut vm, 11), (7, 13), "now visible again: no second scroll");
    // body 10 (GNU "lower" window): line 11 -> start 6, line 40 -> start 35.
    let mut vm = long_buffer(10);
    goto_line(&mut vm, 11);
    assert_eq!(redisplay(&mut vm, 10), (6, 11));
    goto_line(&mut vm, 40);
    assert_eq!(redisplay(&mut vm, 10), (35, 40));
}

#[test]
fn a_forced_start_moves_point_to_the_middle_row_instead() {
    // GNU: (set-window-start nil <line 50>) with point at 1 -> start 50, point 55.
    let mut vm = long_buffer(12);
    eval(&mut vm, "(progn (goto-char (point-min)) (forward-line 49) (set-window-start nil (point)))");
    eval(&mut vm, "(goto-char (point-min))");
    assert_eq!(redisplay(&mut vm, 12), (50, 55));
    // The force is consumed by that redisplay (GNU clears w->force_start), so
    // the next out-of-view point recenters rather than moving point again.
    goto_line(&mut vm, 120);
    assert_eq!(redisplay(&mut vm, 12), (114, 120));
}

#[test]
fn every_window_scrolls_independently_and_the_snapshot_paints_the_new_start() {
    let mut vm = long_buffer(22);
    // Two windows on the same buffer, 11 text rows each after the split.
    eval(&mut vm, "(progn (set-window-new-pixel (selected-window) 240) (setq lower (split-window-internal (selected-window) 220 nil 0.5)))");
    goto_line(&mut vm, 40);
    let snapshot = vm.frame_snapshot(800, 24 * LINE).unwrap();
    let upper = &snapshot.windows[0].snapshot;
    let lower = &snapshot.windows[1].snapshot;
    // Each window publishes the lines from its start's.
    let line_start = |n: usize| upper.text.utf8.match_indices('\n').nth(n - 36).unwrap().0 + 1;
    assert_eq!(upper.start_byte, 0);
    assert!(upper.text.utf8.starts_with("line 35\n"), "selected 11-row window recentred on line 40");
    assert_eq!((lower.start_byte, &lower.text.utf8[..7]), (0, "line 1\n"), "the other window's point (1) was visible");
    assert_eq!(upper.point_byte, line_start(40));
}

#[test]
fn pixel_heights_use_gnus_vertical_backward_arithmetic() {
    // Text height 250px at a 20px line: 12 fully visible rows. dy = 125,
    // 6 lines back leave 5px (<= 2*20/3 = 13), so point lands on row 6.
    // Text height 270px: dy = 135, 6 lines leave 15px (> 13): one more line,
    // point on row 7. A partially visible 13th/14th row never holds point.
    // 250px: line 13 is the first row not fully visible. 270px shows 13 full
    // rows, so line 14 is the first one out of view.
    for (text_height, target, row) in [(250u32, 13usize, 6usize), (270, 14, 7)] {
        let mut vm = Vm::new();
        let height = text_height + 2 * LINE;
        vm.set_frame_geometry(800, height, LINE, COLUMN).unwrap();
        eval(&mut vm, r#"(progn (set-buffer (get-buffer-create "long"))
             (let ((i 1)) (while (<= i 200) (insert (format "line %d\n" i)) (setq i (1+ i))))
             (setq mode-line-format '("%b")) (set-window-buffer nil (current-buffer)))"#);
        goto_line(&mut vm, target);
        vm.frame_snapshot(800, height).unwrap();
        let start = eval(&mut vm, "(let ((s (window-start)) (p (point)) (n 1)) (goto-char 1) (while (< (point) s) (forward-line 1) (setq n (1+ n))) (goto-char p) n)");
        assert_eq!(start.parse::<usize>().unwrap(), target - row, "text height {text_height}px");
    }
}

/// GNU `redisplay_window` (`xdisp.c:21249`): when moving back half the window
/// height lands a full window above point -- a one-row window -- the start is
/// the beginning of point's own line instead.
#[test]
fn a_one_row_window_starts_at_points_line() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 60, 20, 10).unwrap();
    vm.eval_source(r#"(progn (set-buffer (get-buffer-create "one")) (insert "line 1\nline 2\nline 3\nline 4\n")
                     (setq mode-line-format '("%b")) (set-window-buffer nil (current-buffer)) (goto-char 17) t)"#).unwrap();
    let snapshot = vm.frame_snapshot(800, 60).unwrap();
    let window = &snapshot.windows[0].snapshot;
    let start = vm.eval_source("(window-start)").unwrap();
    assert_eq!(start.display().unwrap(), "15", "point 17 is on line 3, which starts at 15");
    assert_eq!(window.start_byte, 0);
    assert!(window.text.utf8.starts_with("line 3\n"), "a window publishes from its start's line");
}
