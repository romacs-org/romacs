//! Child frames against pinned GNU: the frame and window queries packages
//! such as posframe, keymap-popup and which-key observe (child_frames.el),
//! evaluated form by form in one session and compared, as printed, with
//! pinned GNU X11's answers (child_frames.gnu, child_frames_oracle.py).
use romacs_vm::Vm;

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

#[test]
fn child_frame_queries_match_pinned_gnu() {
    let probe = include_str!("child_frames.el");
    let expected: Vec<&str> = forms(include_str!("child_frames.gnu")).collect();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    let mut failures = Vec::new();
    let actual: Vec<String> = forms(probe).map(|form| {
        if form.starts_with("(setq ") {
            vm.eval_source(form).map(|_| "-".to_owned()).unwrap_or_else(|e| format!("setup failed: {e}"))
        } else {
            let source = format!("(prin1-to-string (condition-case e {form} (error (list 'signal e))))");
            // `display' prints the printed string as Lisp reads it.
            vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
                printed[1..printed.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            }).unwrap_or_else(|e| format!("eval failed: {e}"))
        }
    }).collect();
    assert_eq!(actual.len(), expected.len(), "child_frames.gnu is stale: rerun child_frames_oracle.py");
    for ((form, gnu), romacs) in forms(probe).zip(&expected).zip(&actual) {
        if gnu != romacs { failures.push(format!("{form}\n  GNU    {gnu}\n  Romacs {romacs}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Showing, moving and hiding a child frame never changes the windows of
/// the frame it is painted over.
#[test]
fn a_child_frame_leaves_its_parents_windows_alone() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let edges = "(mapcar (lambda (w) (list (window-pixel-left w) (window-pixel-top w) (window-pixel-width w) (window-pixel-height w))) (window-list nil t))";
    vm.eval_source("(let* ((a (selected-window)) (h (window-pixel-height a))) (set-window-new-pixel a (- h 300)) (split-window-internal a 300 nil 0.5))").unwrap();
    let before = vm.eval_source(edges).unwrap().display().unwrap();
    vm.eval_source("(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 30) (height . 8) (left . 10) (top . 500))))").unwrap();
    vm.eval_source("(set-frame-position c 40 20) (set-frame-size c 60 12) (make-frame-invisible c) (make-frame-visible c)").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
    vm.eval_source("(delete-frame c)").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
}

/// Positions as frame.c reads them for a child frame: a negative integer
/// or `(- N)' from the parent's far edge, a float a fraction of the room.
#[test]
fn child_frame_positions_count_from_either_edge() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let place = |vm: &mut Vm, left: &str, top: &str| vm.eval_source(&format!(
        "(let ((c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 2) (child-frame-border-width . 1) (left . {left}) (top . {top}))))) (prog1 (frame-position c) (delete-frame c)))"))
        .unwrap().display().unwrap();
    // The frame is 102 by 42 pixels outside.
    assert_eq!(place(&mut vm, "-10", "(- 5)"), "(688 . 553)");
    assert_eq!(place(&mut vm, "(+ -4)", "0"), "(-4 . 0)");
    assert_eq!(place(&mut vm, "0.5", "1.0"), "(349 . 558)");
}

/// A child frame's window redisplay runs Lisp that may collect and move
/// frames: the frames after it still snapshot with their own parameters.
#[test]
fn child_frame_snapshots_survive_a_collecting_mode_line() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_collection_floor(1);
    vm.eval_source("(progn \
        (set-buffer (get-buffer-create \"gc\")) (insert \"gc\") \
        (setq mode-line-format '((:eval (progn (garbage-collect) (make-string 3 ?m))))) \
        (set-buffer (get-buffer-create \"plain\")) (insert \"plain\") (setq mode-line-format nil) \
        (dotimes (i 3) \
          (let ((c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 10) (height . 3) (left . ,(* 100 i)) \
                                           (background-color . ,(format \"#10203%d\" i)))))) \
            (set-window-buffer (frame-root-window c) (if (= i 0) \"gc\" \"plain\")))))").unwrap();
    for _ in 0..3 {
        let children = vm.frame_snapshot(800, 600).unwrap().children;
        let seen: Vec<_> = children.iter().map(|c| (c.x, c.background)).collect();
        assert_eq!(seen, vec![(0, [0x10, 0x20, 0x30]), (100, [0x10, 0x20, 0x31]), (200, [0x10, 0x20, 0x32])]);
    }
}

/// A child frame hosting the minibuffer window (`romacs--host-minibuffer'):
/// the frame's windows take its line, the minibuffer window is laid out at
/// the host's edge and drawn there, it grows the host at the far side, and
/// giving it back, or deleting the host, restores the line.
#[test]
fn a_hosted_minibuffer_takes_no_line_of_the_frame() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let edges = |vm: &mut Vm, window: &str| vm.eval_source(&format!(
        "(let ((w {window})) (list (window-pixel-left w) (window-pixel-top w) (window-pixel-width w) (window-pixel-height w)))"))
        .unwrap().display().unwrap();
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 580)");
    vm.eval_source("(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (minibuffer . ,(minibuffer-window)) \
                    (width . (text-pixels . 798)) (height . (text-pixels . 100)) (child-frame-border-width . 1) (left . 0) (top . 498))))").unwrap();
    assert_eq!(vm.eval_source("(eq (romacs--host-minibuffer c 'bottom) c)").unwrap().display().unwrap(), "t");
    assert_eq!(vm.eval_source("(eq (romacs--minibuffer-host) c)").unwrap().display().unwrap(), "t");
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 600)");
    assert_eq!(edges(&mut vm, "(minibuffer-window)"), "(1 579 798 20)");
    assert_eq!(edges(&mut vm, "(frame-root-window c)"), "(1 1 798 80)");
    // The minibuffer window is the host frame's still, drawn in the child.
    assert_eq!(vm.eval_source("(list (eq (window-frame (minibuffer-window)) (selected-frame)) (eq (minibuffer-window c) (minibuffer-window)))")
        .unwrap().display().unwrap(), "(t t)");
    let snapshot = vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(snapshot.windows.len(), 1, "the root frame draws its own windows only");
    let child = &snapshot.children[0];
    let placed: Vec<_> = child.windows.iter().map(|w| (w.x, w.y, w.snapshot.height)).collect();
    assert_eq!(placed, vec![(1, 1, 80), (1, 81, 20)]);
    // Three lines of input grow the strip upward; its bottom stays.
    vm.eval_source("(setq resize-mini-windows t)").unwrap();
    vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (insert \"a\\nb\\nc\"))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(edges(&mut vm, "(minibuffer-window)"), "(1 539 798 60)");
    assert_eq!(vm.eval_source("(list (frame-position c) (frame-text-height c))").unwrap().display().unwrap(), "((0 . 458) 140)");
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 600)");
    // A top edge: a box keeps its top and its root window follows the minibuffer.
    vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (erase-buffer))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source("(romacs--host-minibuffer c 'top) (set-frame-position c 100 50)").unwrap();
    assert_eq!(edges(&mut vm, "(minibuffer-window)"), "(101 51 798 20)");
    assert_eq!(edges(&mut vm, "(frame-root-window c)"), "(1 21 798 80)");
    // Only the minibuffer: the root window keeps no line and is not drawn.
    vm.eval_source("(set-frame-size c 798 20 t)").unwrap();
    assert_eq!(edges(&mut vm, "(frame-root-window c)"), "(1 21 798 0)");
    let child = &vm.frame_snapshot(800, 600).unwrap().children[0];
    assert_eq!(child.windows.len(), 1);
    vm.eval_source("(romacs--host-minibuffer nil)").unwrap();
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 580)");
    assert_eq!(edges(&mut vm, "(minibuffer-window)"), "(0 580 800 20)");
    vm.eval_source("(set-frame-size c 798 100 t) (romacs--host-minibuffer c)").unwrap();
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 600)");
    vm.eval_source("(delete-frame c)").unwrap();
    assert_eq!(vm.eval_source("(romacs--minibuffer-host)").unwrap().display().unwrap(), "nil");
    assert_eq!(edges(&mut vm, "(frame-root-window)"), "(0 0 800 580)");
    assert_eq!(edges(&mut vm, "(minibuffer-window)"), "(0 580 800 20)");
}

/// Input wider than the minibuffer window wraps: the window takes a line
/// per row (a row holds one column less than the window, for the `\'
/// continuation glyph), so a narrow layer grows to show all of it.
#[test]
fn a_hosted_minibuffer_grows_for_its_wrapped_rows() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.eval_source("(setq resize-mini-windows t truncate-partial-width-windows nil)").unwrap();
    vm.eval_source("(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (minibuffer . ,(minibuffer-window)) \
                    (width . (text-pixels . 400)) (height . (text-pixels . 20)) (left . 100) (top . 100))))").unwrap();
    vm.eval_source("(romacs--host-minibuffer c 'top)").unwrap();
    // 39 columns a row: 100 characters take 3 rows.
    vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (insert (make-string 100 ?x)))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("(list (window-pixel-height (minibuffer-window)) (frame-text-height c) (frame-position c))")
        .unwrap().display().unwrap(), "(60 60 (100 . 100))");
    // Truncated lines take one row each.
    vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (set (make-local-variable (quote truncate-lines)) t))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("(window-pixel-height (minibuffer-window))").unwrap().display().unwrap(), "20");
}

/// A wide glyph that does not fit at a row's end starts the next row:
/// 39 two-column glyphs in 39-column rows take 19, 19 and 1.
#[test]
fn a_hosted_minibuffer_wraps_wide_glyphs_whole() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.eval_source("(setq resize-mini-windows t truncate-partial-width-windows nil)").unwrap();
    vm.eval_source("(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (minibuffer . ,(minibuffer-window)) \
                    (width . (text-pixels . 400)) (height . (text-pixels . 20)) (left . 100) (top . 100))))").unwrap();
    vm.eval_source("(romacs--host-minibuffer c 'top)").unwrap();
    vm.eval_source("(with-current-buffer (window-buffer (minibuffer-window)) (insert (make-string 39 ?中)))").unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("(window-pixel-height (minibuffer-window))").unwrap().display().unwrap(), "60");
}
