//! Fringes and the tab bar on a window-system frame, against the pinned
//! GNU pgtk's answers on a 1216x876 frame of 10x25 cells (no scroll bars,
//! as the parity harnesses run it): 8-pixel fringes, a split of 606 and 610
//! pixels whose bodies are 59 columns each, and a tab bar of one line that
//! puts the root window 25 pixels down.
use romacs_vm::Vm;

/// A window system that answers nothing about fonts.
struct Pgtk;
impl romacs_vm::DisplayHost for Pgtk {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &str, _: f32, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
}

fn session(display: bool) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(1216, 876, 25, 10).unwrap();
    if display {
        vm.set_display_host(Some(Box::new(Pgtk)));
        assert!(vm.open_display());
    }
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    vm
}

fn eval(vm: &mut Vm, form: &str) -> String {
    vm.eval_source(form).unwrap_or_else(|e| panic!("{form}: {e}")).display().unwrap()
}

#[test]
fn a_window_system_frame_gives_windows_eight_pixel_fringes() {
    let mut vm = session(true);
    assert_eq!(eval(&mut vm, "(list (frame-parameter nil 'left-fringe) (frame-parameter nil 'right-fringe) (frame-fringe-width))"), "(8 8 16)");
    assert_eq!(eval(&mut vm, "(window-fringes)"), "(8 8 nil nil)");
    assert_eq!(eval(&mut vm, "(list (window-body-width) (window-body-width nil t) (frame-text-cols))"), "(120 1200 120)");
    // GNU's split-window-right of this frame: 606 and 610 pixels, no
    // divider column, each body 59 columns.
    eval(&mut vm, "(let ((a (selected-window))) (set-window-new-pixel a 606) (split-window-internal a 610 t 0.5))");
    assert_eq!(eval(&mut vm, "(mapcar (lambda (w) (list (window-pixel-width w) (window-body-width w) (window-body-width w t))) (window-list))"),
               "((606 59 590) (610 59 594))");
}

#[test]
fn a_frame_without_a_window_system_has_no_fringes_and_a_divider_column() {
    let mut vm = session(false);
    assert_eq!(eval(&mut vm, "(window-fringes)"), "(0 0 nil nil)");
    assert_eq!(eval(&mut vm, "(frame-fringe-width)"), "0");
    eval(&mut vm, "(let ((a (selected-window))) (set-window-new-pixel a 606) (split-window-internal a 610 t 0.5))");
    assert_eq!(eval(&mut vm, "(mapcar (lambda (w) (window-body-width w t)) (window-list))"), "(596 610)");
}

#[test]
fn fringe_parameters_and_set_window_fringes_resize_the_body() {
    let mut vm = session(true);
    eval(&mut vm, "(modify-frame-parameters nil '((left-fringe . 10) (right-fringe . -10)))");
    assert_eq!(eval(&mut vm, "(list (window-fringes) (frame-parameter nil 'left-fringe) (window-body-width) (window-body-width nil t))"),
               "((10 10 nil nil) 10 119 1196)");
    // nil is the default, 8 pixels.
    eval(&mut vm, "(modify-frame-parameters nil '((left-fringe . nil) (right-fringe . nil)))");
    assert_eq!(eval(&mut vm, "(set-window-fringes nil 20 0)"), "t");
    assert_eq!(eval(&mut vm, "(list (window-fringes) (window-body-width nil t))"), "((20 0 nil nil) 1196)");
    assert_eq!(eval(&mut vm, "(set-window-fringes nil 20 0)"), "nil");
    // Fringes that leave no room are refused (MIN_SAFE_WINDOW_PIXEL_WIDTH).
    assert_eq!(eval(&mut vm, "(list (set-window-fringes nil 1200 10) (window-fringes))"), "(nil (20 0 nil nil))");
    assert_eq!(eval(&mut vm, "(list (set-window-fringes nil 20 0 t t) (window-fringes))"), "(t (20 0 t t))");
    assert_eq!(eval(&mut vm, "(progn (set-window-fringes nil nil nil) (window-fringes))"), "(8 8 nil nil)");
}

#[test]
fn a_buffer_shown_in_a_window_gives_it_its_fringe_widths() {
    let mut vm = session(true);
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"narrow\") (setq left-fringe-width 0 right-fringe-width 4))");
    eval(&mut vm, "(set-window-buffer nil \"narrow\")");
    assert_eq!(eval(&mut vm, "(window-fringes)"), "(0 4 nil nil)");
    // Persistent fringes stay with the window.
    eval(&mut vm, "(set-window-fringes nil 2 2 nil t)");
    eval(&mut vm, "(set-window-buffer nil \"*scratch*\")");
    assert_eq!(eval(&mut vm, "(window-fringes)"), "(2 2 nil t)");
}

#[test]
fn the_tab_bar_takes_the_top_line_of_the_frame() {
    let mut vm = session(true);
    assert_eq!(eval(&mut vm, "(list (tab-bar-height) (window-pixel-top) (window-body-height) (frame-text-lines))"), "(0 0 33 35)");
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 1)))");
    assert_eq!(eval(&mut vm, "(list (frame-parameter nil 'tab-bar-lines) (tab-bar-height) (tab-bar-height nil t))"), "(1 1 25)");
    assert_eq!(eval(&mut vm, "(list (window-pixel-top) (window-body-height) (window-pixel-height) (frame-text-lines) (frame-text-height))"),
               "(25 32 826 34 851)");
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 0)))");
    assert_eq!(eval(&mut vm, "(list (window-pixel-top) (window-body-height))"), "(0 33)");
}

#[test]
fn snapshots_publish_the_fringes_and_their_indicators() {
    let mut vm = session(true);
    eval(&mut vm, "(set-default 'fringe-indicator-alist '((truncation left-arrow right-arrow) (continuation left-curly-arrow right-curly-arrow) (empty-line . empty-line)))");
    eval(&mut vm, "(progn (put 'left-arrow 'fringe 3) (put 'right-arrow 'fringe 4) (put 'left-curly-arrow 'fringe 7) (put 'right-curly-arrow 'fringe 8) (put 'empty-line 'fringe 24))");
    eval(&mut vm, "(setq indicate-empty-lines t)");
    let snapshot = vm.frame_snapshot(1216, 876).unwrap();
    let window = &snapshot.windows[0];
    let chrome = &window.snapshot.chrome;
    assert_eq!((window.x, window.snapshot.width, chrome.left_fringe, chrome.right_fringe), (8, 1200, 8, 8));
    let numbers = |side: &[Option<romacs_layout_input::FringeBitmap>; 2]| side.iter().map(|b| b.as_ref().map(|b| b.number)).collect::<Vec<_>>();
    assert_eq!(numbers(&chrome.indicators.continuation), [Some(7), Some(8)]);
    assert_eq!(numbers(&chrome.indicators.truncation), [Some(3), Some(4)]);
    assert_eq!(numbers(&chrome.indicators.empty_line), [Some(24), None]);
    assert_eq!(chrome.indicators.continuation[1].as_ref().unwrap().bits[2], 0x03 << 8);
}

#[test]
fn defined_fringe_bitmaps_take_numbers_above_the_standard_ones() {
    let mut vm = session(true);
    assert_eq!(eval(&mut vm, "(list (define-fringe-bitmap 'dot [24 60 24] nil 6 'top) (get 'dot 'fringe) (car fringe-bitmaps))"), "(dot 25 dot)");
    eval(&mut vm, "(set-default 'fringe-indicator-alist '((continuation dot dot)))");
    let snapshot = vm.frame_snapshot(1216, 876).unwrap();
    let dot = snapshot.windows[0].snapshot.chrome.indicators.continuation[0].clone().unwrap();
    assert_eq!((dot.number, dot.width, dot.bits), (25, 6, vec![24 << 10, 60 << 10, 24 << 10]));
    assert_eq!(eval(&mut vm, "(list (condition-case e (define-fringe-bitmap 'bad [1] nil 17) (error (car e))) (destroy-fringe-bitmap 'dot) (get 'dot 'fringe) (memq 'dot fringe-bitmaps))"),
               "(args-out-of-range nil nil nil)");
}

/// tab-bar.el's width cache: a hash table on a test of GNU's own
/// `equal-including-properties' and its hash.
#[test]
fn a_hash_table_test_of_equal_including_properties_works() {
    let mut vm = session(false);
    assert_eq!(eval(&mut vm, "(progn (define-hash-table-test 'props #'equal-including-properties #'sxhash-equal-including-properties)
        (let ((h (make-hash-table :test 'props)))
          (puthash (propertize \"a\" 'face 'bold) 1 h)
          (list (gethash \"a\" h) (gethash (propertize \"a\" 'face 'bold) h) (hash-table-test h) (hash-table-count h))))"),
               "(nil 1 props 1)");
    assert_eq!(eval(&mut vm, "(progn (define-hash-table-test 'mine #'string= #'sxhash-equal) (condition-case e (make-hash-table :test 'mine) (error (car (cdr e)))))"),
               "\"VM2 does not support user-defined hash table tests\"");
}

/// A child frame's size parameters give its text area; its fringes widen
/// it, as GNU pgtk's (30 columns: native width 316, or 300 without them).
#[test]
fn a_child_frame_keeps_its_columns_beside_its_fringes() {
    let mut vm = session(true);
    let sizes = "(list (frame-text-cols c) (frame-native-width c) (window-body-width (frame-root-window c)) (window-fringes (frame-root-window c)))";
    eval(&mut vm, "(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 30) (height . 8)))) nil");
    assert_eq!(eval(&mut vm, sizes), "(30 316 30 (8 8 nil nil))");
    eval(&mut vm, "(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 30) (height . 8) (left-fringe . 0) (right-fringe . 0)))) nil");
    assert_eq!(eval(&mut vm, sizes), "(30 300 30 (0 0 nil nil))");
    eval(&mut vm, "(modify-frame-parameters c '((left-fringe . 4)))");
    assert_eq!(eval(&mut vm, sizes), "(30 304 30 (4 0 nil nil))");
}

/// A redisplay that reuses a window's snapshot still shows a bitmap
/// redefined, refaced or destroyed since, and margins moved from one side
/// to the other: each change alone between two snapshots.
#[test]
fn reused_snapshots_follow_bitmaps_and_margins() {
    let mut vm = session(true);
    eval(&mut vm, "(define-fringe-bitmap 'dot [1] nil 8) (set-default 'fringe-indicator-alist '((continuation dot dot)))");
    eval(&mut vm, "(set-window-margins nil 1 2)");
    let mut snap = |vm: &mut Vm, form: &str| {
        eval(vm, form);
        vm.frame_snapshot(1216, 876).unwrap().windows.remove(0)
    };
    snap(&mut vm, "nil");
    let bits = |w: &romacs_layout_input::PlacedWindowSnapshot| w.snapshot.chrome.indicators.continuation[0].as_ref().map(|b| (b.bits.clone(), b.face));
    let window = snap(&mut vm, "(define-fringe-bitmap 'dot [3] nil 8)");
    assert_eq!(bits(&window).unwrap().0, vec![3 << 8]);
    let fringe = bits(&window).unwrap().1;
    let window = snap(&mut vm, "(set-fringe-bitmap-face 'dot 'mode-line)");
    assert_ne!(bits(&window).unwrap().1, fringe);
    let window = snap(&mut vm, "(destroy-fringe-bitmap 'dot)");
    assert_eq!(bits(&window), None);
    let window = snap(&mut vm, "(set-window-margins nil 2 1)");
    assert_eq!((window.snapshot.chrome.left_margin, window.snapshot.chrome.right_margin, window.x), (20, 10, 28));
}

/// frame.c:1101: the total lines are the native height's, tab bar included.
#[test]
fn the_total_lines_count_the_tab_bar() {
    let mut vm = session(true);
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 1)))");
    assert_eq!(eval(&mut vm, "(list (frame-total-lines) (frame-text-lines) (frame-total-cols) (frame-text-cols))"), "(35 34 121 120)");
}
