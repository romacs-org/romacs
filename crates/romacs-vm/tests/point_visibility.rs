//! Point visibility on both axes, over the layout that paints: after
//! every step of point_visibility.el's randomized command sequences (long,
//! short, tabbed and wide lines; split, narrow and divided windows;
//! truncation, line numbers, `auto-hscroll-mode', `hscroll-margin',
//! `hscroll-step', `scroll-margin' and `scroll-conservatively' varied),
//! every window's cursor lies inside its text area, and each window's
//! hscroll, start, point and body width are pinned GNU X11's
//! (point_visibility.gnu, child_frames_oracle.py --probe). Romacs runs
//! GNU's Lisp on a window-system frame 100 columns by 24 lines of its
//! real font, laid out by romacs-layout2 as the GUI lays it out.
use romacs_vm::Vm;
use std::{cell::RefCell, path::Path, rc::Rc};
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;
#[path = "support/gui_probe.rs"]
mod gui_probe;

struct Engine(Rc<RefCell<romacs_layout2::LayoutEngine>>);
impl romacs_vm::LayoutHost for Engine {
    fn measure(&mut self, input: &romacs_layout_input::MeasureInput) -> Result<romacs_layout_input::Measured, String> {
        self.0.borrow_mut().measure(input)
    }
    fn lay_out(&mut self, frame: &romacs_layout_input::FrameSnapshot) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        self.0.borrow_mut().lay_out(frame)
    }
}

struct Session { vm: Vm, engine: Rc<RefCell<romacs_layout2::LayoutEngine>>, width: u32, height: u32, presentation: u64 }

fn session() -> Session {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    vm.eval_source("(setq noninteractive nil)").unwrap();
    let (line, column) = romacs_layout2::frame_metrics(&vm.display_default_face().unwrap()).unwrap();
    // 100 columns of text between fringes a column wide (the probe sets
    // them), 24 lines with the mini-window's.
    let (width, height) = (102 * column, 24 * line);
    vm.set_frame_geometry(width, height, line, column).unwrap();
    vm.set_display_host(Some(Box::new(gui_probe::Pgtk)));
    assert!(vm.open_display());
    let engine = Rc::new(RefCell::new(romacs_layout2::LayoutEngine::new().unwrap()));
    vm.set_layout_host(Some(Box::new(Engine(engine.clone()))));
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    Session { vm, engine, width, height, presentation: 0 }
}

impl Session {
    /// Paint the frame as the GUI does; the cursors outside their window's
    /// text area, and whether the selected window showed one.
    fn paint(&mut self) -> (Vec<String>, bool) {
        let snapshot = self.vm.frame_snapshot(self.width, self.height).unwrap();
        let selected = self.vm.selected_window_id().unwrap();
        self.presentation += 1;
        let color = romacs_display_protocol::Color::from_pixel(0xffffff);
        let (frame, ends) = self.engine.borrow_mut().layout_frame(&snapshot, self.presentation, Some((selected, color))).unwrap();
        self.vm.display_apply_layout(&ends).unwrap();
        let mut outside = vec![];
        for (placed, end) in snapshot.windows.iter().zip(&ends) {
            let s = &placed.snapshot;
            assert!(s.header_line.is_none(), "the probe shows no header lines");
            // The text area, placed at its origin, and the right fringe a
            // cursor after a full row shows in (`overflow-newline-into-fringe').
            let (x0, y0) = (placed.x as f32, placed.y as f32);
            let (x1, y1) = (x0 + (s.width + s.chrome.right_fringe) as f32, y0 + end.body_height);
            for c in frame.window_cursors.iter().filter(|c| c.window_id.get() == s.id as i64) {
                if c.x < x0 || c.y < y0 || c.x + c.width > x1 + 0.01 || c.y + c.height > y1 + 0.01 {
                    outside.push(format!("window {}: cursor ({}, {}) {}x{} outside ({x0}, {y0})-({x1}, {y1})", s.id, c.x, c.y, c.width, c.height));
                }
            }
        }
        let shown = frame.window_cursors.iter().any(|c| c.window_id.get() == selected as i64);
        (outside, shown)
    }
}

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

#[test]
fn point_stays_visible_and_scrolls_as_pinned_gnu() {
    replay(include_str!("point_visibility.el"), include_str!("point_visibility.gnu"), 500);
}

/// scroll_invisible.el: C-n past the window's end and C-p past its start
/// over invisible lines (text properties, overlays, an invisibility spec;
/// whole lines and a heading's folded subtree) scroll as GNU's
/// try_scrolling does, or recentre where it does, for `scroll-conservatively'
/// 0, 5 and 101 and `scroll-margin' 0 and 3.
#[test]
fn scrolling_past_invisible_text_as_pinned_gnu() {
    replay(include_str!("scroll_invisible.el"), include_str!("scroll_invisible.gnu"), 2000);
}

/// pre_redisplay.el: `pre-redisplay-function' and its hook run with the
/// windows GNU's prepare_menu_bars passes, under `inhibit-redisplay'.
#[test]
fn pre_redisplay_function_runs_as_pinned_gnu() {
    replay(include_str!("pre_redisplay.el"), include_str!("pre_redisplay.gnu"), 30);
}

/// Replay PROBE's forms, painting at each `pv-step', against GNU's
/// ANSWERS; at least LEAST steps run.
fn replay(probe: &str, answers: &str, least: usize) {
    assert_eq!(forms(probe).count(), forms(answers).count(), "the .gnu answers are stale: rerun child_frames_oracle.py --probe");
    let mut s = session();
    let (mut sequence, mut mismatches, mut invariant, mut steps) = ("", vec![], vec![], 0);
    let mut gnu = forms(answers);
    for line in probe.lines().filter(|line| !line.is_empty()) {
        if let Some(name) = line.strip_prefix(";; ").filter(|name| !name.contains(' ')) { sequence = name; continue; }
        if line.starts_with(';') { continue; }
        let expected = gnu.next().unwrap();
        let printed = gui_probe::printed(&mut s.vm, line);
        if line.starts_with("(defun pv-step ") {
            // Here a step runs its commands; the runner paints, then reads.
            gui_probe::printed(&mut s.vm, "(defun pv-step (cmds) (pv-run cmds) 'pv-paint)");
        }
        let actual = if printed == "pv-paint" {
            steps += 1;
            let (outside, shown) = s.paint();
            invariant.extend(outside.into_iter().map(|o| format!("{sequence} {line}: {o}")));
            if !shown && gui_probe::printed(&mut s.vm, "(window-minibuffer-p)") == "nil" {
                invariant.push(format!("{sequence} {line}: the selected window shows no cursor"));
            }
            gui_probe::printed(&mut s.vm, "(pv-state)")
        } else {
            printed
        };
        // Wide characters' widths are the fonts': no comparison there.
        if actual != expected && !sequence.starts_with("wide-") { mismatches.push(format!("{sequence} {line}\n  GNU    {expected}\n  Romacs {actual}")); }
    }
    assert!(steps > least, "{steps} steps");
    assert!(invariant.is_empty() && mismatches.is_empty(), "{} cursors outside their text area:\n{}\n{} of {steps} steps differ from GNU:\n{}",
        invariant.len(), invariant.join("\n"), mismatches.len(), mismatches.join("\n"));
}


fn eval(s: &mut Session, form: &str) -> String { gui_probe::printed(&mut s.vm, form) }

/// Under `auto-hscroll-mode' `current-line', an explicit scroll scrolls
/// every row; once point moves, rows but point's go back to
/// `min_hscroll' (0) even where point's own hscroll stands.
#[test]
fn moving_point_after_an_explicit_scroll_unscrolls_the_other_rows() {
    let mut s = session();
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "cl")) (dotimes (i 5) (insert (make-string 60 (+ ?a i)) "\n"))
                          (setq-local truncate-lines t) (setq auto-hscroll-mode 'current-line) (goto-char 31))"#);
    s.paint();
    eval(&mut s, "(set-window-hscroll nil 10)");
    s.paint();
    let column = s.vm.frame_snapshot(s.width, s.height).unwrap().windows[0].snapshot.hscroll.point_row / 10;
    let hscroll = |s: &mut Session| s.vm.frame_snapshot(s.width, s.height).unwrap().windows[0].snapshot.hscroll;
    assert_eq!(hscroll(&mut s), romacs_layout_input::Hscroll::uniform(10 * column), "held: every row scrolled");
    eval(&mut s, "(forward-char 1)");
    s.paint();
    assert_eq!(eval(&mut s, "(window-hscroll)"), "10");
    assert_eq!(hscroll(&mut s), romacs_layout_input::Hscroll { rows: 0, point_row: 10 * column });
}

/// `window-line-height' reads point's row, not the empty rows below the
/// text (GNU's cursor row).
#[test]
fn window_line_height_reads_points_row_above_the_empty_ones() {
    let mut s = session();
    eval(&mut s, r#"(switch-to-buffer (get-buffer-create "empty"))"#);
    s.paint();
    let line = eval(&mut s, "(frame-char-height)");
    assert_eq!(eval(&mut s, "(cdr (window-line-height))"), "(0 0 0)");
    eval(&mut s, r#"(progn (insert "a\nb\nc") (goto-char (point-max)))"#);
    s.paint();
    assert_eq!(eval(&mut s, "(nth 1 (window-line-height))"), "2");
    assert_eq!(eval(&mut s, "(car (window-line-height))"), line);
}

/// `window-old-point' is a marker that edits move, and a window
/// configuration saves and restores it, as GNU's (pinned GNU X11: (51 51)
/// and (50 50)).
#[test]
fn window_old_point_moves_with_edits_and_window_configurations() {
    let mut s = session();
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "old")) (insert (make-string 100 ?x)) (goto-char 50))"#);
    s.paint();
    assert_eq!(eval(&mut s, "(progn (save-excursion (goto-char 1) (insert \"\\n\")) (list (window-point) (window-old-point)))"), "(51 51)");
    s.paint();
    eval(&mut s, "(goto-char 50)");
    s.paint();
    let restored = "(let ((c (current-window-configuration))) (switch-to-buffer (get-buffer-create \"other\")) (set-window-configuration c) (list (window-point) (window-old-point)))";
    assert_eq!(eval(&mut s, restored), "(50 50)");
}

/// A scroll moves a window's old point with its point, keeping an explicit
/// hscroll's hold; an old point inserts as `window-point-insertion-type'
/// says, its saved copy too (pinned GNU X11: (102 102 10) and (51 51)).
#[test]
fn scrolling_and_insertion_move_the_old_point_as_gnus() {
    let mut s = session();
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "sc")) (dotimes (_ 100) (insert (make-string 100 ?x) "\n"))
                          (setq-local truncate-lines t) (goto-char 1))"#);
    s.paint();
    eval(&mut s, "(progn (set-window-hscroll nil 10) (goto-char 1))");
    s.paint();
    assert_eq!(eval(&mut s, "(progn (scroll-up 1) (list (window-point) (window-old-point) (window-hscroll)))"), "(102 102 10)");
    s.paint();
    assert_eq!(eval(&mut s, "(window-hscroll)"), "10", "the hold stands");
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "ins")) (setq window-point-insertion-type t) (insert (make-string 100 ?x)) (goto-char 50))"#);
    s.paint();
    let restored = "(let ((c (current-window-configuration))) (insert \"\\n\") (switch-to-buffer (get-buffer-create \"other\")) (set-window-configuration c) (list (window-point) (window-old-point)))";
    assert_eq!(eval(&mut s, restored), "(51 51)");
    eval(&mut s, "(setq window-point-insertion-type nil)");
}

/// A window's first display scrolls to show point past its right edge,
/// and raising `hscroll-margin' around a point that does not move scrolls
/// again.
#[test]
fn first_display_and_margin_changes_scroll_without_point_moving() {
    let mut s = session();
    eval(&mut s, r#"(progn (with-current-buffer (get-buffer-create "far") (insert (make-string 300 ?x)) (setq truncate-lines t) (goto-char 151))
                          (switch-to-buffer "far"))"#);
    s.paint();
    let hscroll = eval(&mut s, "(window-hscroll)").parse::<i64>().unwrap();
    assert!(hscroll > 50, "the first display scrolled: {hscroll}");
    eval(&mut s, "(progn (set-window-hscroll nil 0) (goto-char 86) (setq hscroll-margin 5))");
    s.paint();
    eval(&mut s, "(setq hscroll-margin 5)");
    s.paint();
    assert_eq!(eval(&mut s, "(window-hscroll)"), "0");
    eval(&mut s, "(setq hscroll-margin 20)");
    s.paint();
    // hscroll_window_tree: (point's x - half the text area) in columns.
    let expected = eval(&mut s, "(/ (- (* 85 (frame-char-width)) (/ (window-body-width nil t) 2)) (frame-char-width))");
    assert_eq!(eval(&mut s, "(window-hscroll)"), expected);
    eval(&mut s, "(setq hscroll-margin 5)");
}

/// `window-line-height' answers nil while the window's display is out of
/// date, as GNU's does, so callers ask `pos-visible-in-window-p'.
#[test]
fn window_line_height_is_nil_until_redisplay() {
    let mut s = session();
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "stale")) (insert "a\nb\nc\n"))"#);
    s.paint();
    assert_ne!(eval(&mut s, "(window-line-height)"), "nil");
    assert_eq!(eval(&mut s, "(progn (insert \"d\\n\") (window-line-height))"), "nil");
    s.paint();
    assert_ne!(eval(&mut s, "(window-line-height)"), "nil");
    assert_eq!(eval(&mut s, "(progn (set-window-start nil 3) (window-line-height -1))"), "nil");
}

/// Scaling probe (run with --ignored --test-threads 1): a cursor motion,
/// alternately forward and back inside the first row, and a paint per key
/// in a truncated window, which places point's line for its hscroll,
/// against one that wraps, with FACES faces defined; on a line whose RUNS
/// characters each show a face of their own, or one face throughout. The
/// line is cut from the published snapshot with only the faces it uses,
/// so a truncated window costs as a wrapped one does.
#[test]
#[ignore]
fn cursor_motion_per_key_with_many_faces() {
    for (faces, runs) in [(50usize, 1usize), (2000, 1), (2000, 2000)] {
        for truncate in ["nil", "t"] {
            let mut s = session();
            eval(&mut s, &format!("(dotimes (i {faces}) (eval `(defface ,(intern (format \"pv-face-%d\" i)) '((t :weight bold)) \"x\")))"));
            eval(&mut s, &format!(r#"(progn (switch-to-buffer (get-buffer-create "sc"))
                (dotimes (i 2000) (insert (propertize "x" 'face (intern (format "pv-face-%d" (% i {runs}))))))
                (insert "\n") (dotimes (_ 40) (insert (make-string 70 ?y) "\n"))
                (setq-local truncate-lines {truncate}) (goto-char 10))"#));
            s.paint(); s.paint();
            let start = std::time::Instant::now();
            for key in 0..200 { eval(&mut s, if key % 2 == 0 { "(forward-char 1)" } else { "(backward-char 1)" }); s.paint(); }
            let per = start.elapsed().as_secs_f64() / 200.0 * 1e6;
            eprintln!("PVSCALE faces={faces} runs={runs} truncate={truncate} us/key={per:.0}");
        }
    }
}

/// An old point inserts as `window-point-insertion-type' in the window's
/// own buffer says, set again by every `set-window-buffer', KEEP-MARGINS
/// or not (window.c:4395; pinned GNU X11: 50 becomes 51).
#[test]
fn the_old_points_insertion_type_is_the_windows_buffers() {
    let mut s = session();
    eval(&mut s, r#"(progn (with-current-buffer (get-buffer-create "lt") (insert (make-string 100 ?x)) (setq-local window-point-insertion-type t) (goto-char 50))
                          (set-buffer "*scratch*") (set-window-buffer nil "lt"))"#);
    s.paint();
    assert_eq!(eval(&mut s, r#"(with-current-buffer "lt" (save-excursion (goto-char 50) (insert "y")) (window-old-point))"#), "51");
    eval(&mut s, r#"(progn (switch-to-buffer (get-buffer-create "kept")) (insert (make-string 100 ?x)) (goto-char 50))"#);
    s.paint();
    assert_eq!(eval(&mut s, r#"(progn (setq-local window-point-insertion-type t) (set-window-buffer nil (current-buffer) t) (save-excursion (goto-char 50) (insert "y")) (window-old-point))"#), "51");
}
