//! Fringes and the tab bar on a window-system frame, against the pinned
//! GNU pgtk's answers on a 1216x876 frame of 10x25 cells (no scroll bars,
//! as the parity harnesses run it): 8-pixel fringes, a split of 606 and 610
//! pixels whose bodies are 59 columns each, and a tab bar of one line that
//! puts the root window 25 pixels down. Window dividers and the internal
//! border (romacs-chrome.el's pane gap) against pinned GNU X11's answers
//! on the same frame (frame_chrome.el, frame_chrome.gnu).
use romacs_vm::Vm;
use std::path::Path;
#[path = "support/gui_probe.rs"]
mod gui_probe;
use gui_probe::{Pgtk, gnu_session};

fn probe(section: &str, after: impl Fn(&mut Vm, &str)) -> Vec<String> {
    gui_probe::probe(include_str!("frame_chrome.el"), include_str!("frame_chrome.gnu"), section, after)
}


thread_local! {
    /// Whether `session' gives a window-system frame under
    /// `romacs-classic-chrome' (classic_chrome_is_todays_geometry).
    static CLASSIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn session(display: bool) -> Vm {
    if display && CLASSIC.get() { return classic_session(); }
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

/// `gnu_session' with romacs-chrome.el's panes, then `romacs-classic-chrome',
/// without the status bar it keeps (tests/status_bar.rs has the bar's
/// geometry), so the window geometry is a GNU frame's.
fn classic_session() -> Vm {
    let mut vm = gnu_session();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    eval(&mut vm, "(defface romacs-layer-glow '((t :foreground \"#d48a20\")) \"\")");
    vm.load_file(&root.join("rlisp/romacs-chrome.el")).unwrap();
    eval(&mut vm, "(romacs-chrome-apply)");
    assert_eq!(eval(&mut vm, "(list (frame-internal-border-width) (frame-right-divider-width) (window-pixel-width))"), "(6 6 1204)");
    eval(&mut vm, "(romacs-classic-chrome 1)");
    assert_eq!(eval(&mut vm, "(list (plist-get (romacs-chrome-state) :classic) (frame-internal-border-width) (frame-right-divider-width) (frame-parameter nil 'romacs-pane-radius) (frame-parameter nil 'romacs-status-bar-lines))"), "(t 0 0 0 1)");
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 0)))");
    vm
}

#[test]
fn dividers_and_border_follow_gnu_on_a_three_way_split() {
    let failures = probe("dividers_and_border_follow_gnu_on_a_three_way_split", |_, _| {});
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_mini_window_has_no_bottom_divider_and_ordinary_windows_do() {
    let failures = probe("the_mini_window_has_no_bottom_divider_and_ordinary_windows_do", |_, _| {});
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// window.c's prepared_check: along a combination's axis its children sum
/// to it, across it each is as large; each body is its window less its
/// divider, fringes, margins, header and mode line; the root window fills
/// the text area above the mini-window.
const INVARIANTS: &str = r#"(progn
  (defun probe-invariants (w)
    (let ((child (window-child w)) (bad nil))
      (if (null child)
          (let ((margins (window-margins w)) (fringes (window-fringes w)))
            (unless (= (window-body-width w t) (- (window-pixel-width w) (window-right-divider-width w) (car fringes) (cadr fringes) (* (frame-char-width) (+ (or (car margins) 0) (or (cdr margins) 0)))))
              (push (list 'body-width w) bad))
            (unless (= (window-body-height w t) (- (window-pixel-height w) (window-bottom-divider-width w) (window-header-line-height w) (window-mode-line-height w)))
              (push (list 'body-height w) bad))
            (unless (and (>= (window-body-width w t) 0) (>= (window-body-height w t) 0)) (push (list 'negative w) bad)))
        (let ((horizontal (window-combined-p child t)) (sum 0))
          (while child
            (setq sum (+ sum (if horizontal (window-pixel-width child) (window-pixel-height child))))
            (unless (= (if horizontal (window-pixel-height child) (window-pixel-width child)) (if horizontal (window-pixel-height w) (window-pixel-width w)))
              (push (list 'across child) bad))
            (setq bad (append (probe-invariants child) bad) child (window-next-sibling child)))
          (unless (= sum (if horizontal (window-pixel-width w) (window-pixel-height w))) (push (list 'sum w) bad))))
      bad))
  (let* ((b (frame-internal-border-width)) (r (frame-root-window)))
    (append (probe-invariants r)
            (unless (equal (list (window-pixel-left r) (window-pixel-top r) (window-pixel-width r) (window-pixel-height r))
                           (list 0 0 (- (frame-native-width) b b) (- (frame-native-height) b b (window-pixel-height (minibuffer-window)))))
              (list 'root)))))"#;

#[test]
fn uneven_resize_cycles_keep_sibling_sums_and_bodies() {
    let failures = probe("uneven_resize_cycles_keep_sibling_sums_and_bodies", |vm, line| {
        assert_eq!(eval(vm, INVARIANTS), "nil", "after {line}");
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn balance_windows_with_dividers_matches_gnu() {
    let failures = probe("balance_windows_with_dividers_matches_gnu", |vm, line| {
        assert_eq!(eval(vm, INVARIANTS), "nil", "after {line}");
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Under `romacs-classic-chrome' every value is 0 again, and every
/// assertion above about a window-system frame's geometry holds as it does
/// without the chrome. (The bitmap numbers of
/// `defined_fringe_bitmaps_take_numbers_above_the_standard_ones' count
/// the bitmaps GNU's fringe.el defines in this session, so it is not run.)
#[test]
fn classic_chrome_is_todays_geometry() {
    CLASSIC.set(true);
    a_window_system_frame_gives_windows_eight_pixel_fringes();
    fringe_parameters_and_set_window_fringes_resize_the_body();
    a_buffer_shown_in_a_window_gives_it_its_fringe_widths();
    the_tab_bar_takes_the_top_line_of_the_frame();
    snapshots_publish_the_fringes_and_their_indicators();
    a_child_frame_keeps_its_columns_beside_its_fringes();
    reused_snapshots_follow_bitmaps_and_margins();
    the_total_lines_count_the_tab_bar();
    CLASSIC.set(false);
}

/// A divider width change keeps every allocation and changes bodies, so
/// the next redisplay runs `window-size-change-functions', and the one
/// after it none.
#[test]
fn a_divider_change_runs_the_size_change_functions() {
    let mut vm = gnu_session();
    vm.eval_source(include_str!("../../../rlisp/vm/window-change.el")).unwrap();
    eval(&mut vm, "(progn (split-window-right) (setq probe-log nil))");
    vm.frame_snapshot(1216, 876).unwrap();
    eval(&mut vm, "(add-hook 'window-size-change-functions (lambda (_) (push 'size probe-log)))");
    for (parameter, mark) in [("right-divider-width", "right"), ("bottom-divider-width", "bottom")] {
        eval(&mut vm, &format!("(modify-frame-parameters nil '(({parameter} . 6)))"));
        vm.frame_snapshot(1216, 876).unwrap();
        eval(&mut vm, &format!("(push '{mark} probe-log)"));
    }
    vm.frame_snapshot(1216, 876).unwrap();
    assert_eq!(eval(&mut vm, "(reverse probe-log)"), "(size right size bottom)");
}

/// A border the split windows cannot fit inside is refused and the frame
/// keeps its layout, a grown mini-window's lines included; it redisplays
/// as before.
#[test]
fn a_border_the_windows_cannot_fit_is_refused() {
    let mut vm = gnu_session();
    eval(&mut vm, "(progn (split-window-below) (modify-frame-parameters nil '((internal-border-width . 6))))");
    eval(&mut vm, "(progn (setq resize-mini-windows t) (with-current-buffer (window-buffer (minibuffer-window)) (insert \"1\\n2\\n3\")))");
    vm.frame_snapshot(1216, 876).unwrap();
    let edges = "(list (frame-internal-border-width) (mapcar (lambda (w) (window-edges w nil nil t)) (window-list nil t)))";
    assert_eq!(eval(&mut vm, "(window-pixel-height (minibuffer-window))"), "75");
    let before = eval(&mut vm, edges);
    assert!(vm.eval_source("(modify-frame-parameters nil '((internal-border-width . 401)))").is_err());
    assert_eq!(eval(&mut vm, edges), before);
    vm.frame_snapshot(1216, 876).unwrap();
    assert_eq!(eval(&mut vm, edges), before);
}

/// WCAG 2's relative luminance of an sRGB colour.
fn luminance(rgb: [f64; 3]) -> f64 {
    let linear = rgb.map(|c| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

/// An inactive pane's default text, blended toward the frame background
/// (the default background) by the largest `romacs-pane-dim', 0.3, keeps
/// WCAG AA contrast (4.5) on the shipped palette, whether the blend is in
/// sRGB values or in linear light (an sRGB surface's).
#[test]
fn a_dimmed_pane_keeps_aa_contrast() {
    let face = Vm::new().display_default_face().unwrap();
    let [fg, bg] = [face.foreground, face.background].map(|c| c.map(|v| f64::from(v) / 255.0));
    let to_linear = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let to_srgb = |c: f64| if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    let dim = 0.3;
    let srgb: [f64; 3] = std::array::from_fn(|i| fg[i] * (1.0 - dim) + bg[i] * dim);
    let light: [f64; 3] = std::array::from_fn(|i| to_srgb(to_linear(fg[i]) * (1.0 - dim) + to_linear(bg[i]) * dim));
    for dimmed in [srgb, light] {
        let (a, b) = (luminance(dimmed), luminance(bg));
        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(contrast >= 4.5, "{:?} on {:?}: {contrast:.2}", face.foreground, face.background);
    }
}

/// `gnu_session' with romacs-chrome.el's panes and Romacs's themes on the
/// theme path.
fn themed_session() -> Vm {
    let mut vm = gnu_session();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    eval(&mut vm, &format!("(progn (push {:?} load-path) (push {:?} custom-theme-load-path) (require 'romacs-chrome) (romacs-chrome-refresh))",
                           root.join("rlisp"), root.join("etc/themes")));
    vm
}

fn chrome(vm: &mut Vm) -> romacs_layout_input::FrameChrome { vm.frame_snapshot(1216, 876).unwrap().chrome }

/// The edge and hairline are the `romacs-pane-focus' and
/// `romacs-pane-border' faces as the display realises them: a theme, a
/// customised face and an inherited colour show on the next snapshot with
/// no parameter set, and `romacs-chrome-state' reports them.
#[test]
fn pane_chrome_colours_are_the_faces() {
    let mut vm = themed_session();
    // No theme: the focus face inherits the glow, the built-in accent.
    assert_eq!(chrome(&mut vm).focus, Some([0xd4, 0x8a, 0x20]), "an inherited foreground is realised");
    eval(&mut vm, "(load-theme 'omarchy-tokyo-night t)");
    let after = chrome(&mut vm);
    assert_eq!(after.focus, Some([0x7a, 0xa2, 0xf7]), "a theme's focus edge without `romacs-chrome-apply'");
    assert_eq!(eval(&mut vm, "(plist-get (romacs-chrome-state) :focus)"), "\"#7aa2f7\"");
    let border = eval(&mut vm, "(face-foreground 'romacs-pane-border nil t)");
    assert_eq!(after.border.map(|[r, g, b]| format!("\"#{r:02x}{g:02x}{b:02x}\"")), Some(border));
    eval(&mut vm, "(custom-set-faces '(romacs-pane-focus ((t :foreground \"#ff0000\"))))");
    assert_eq!(chrome(&mut vm).focus, Some([0xff, 0, 0]), "a customised face");
    eval(&mut vm, "(romacs-classic-chrome 1)");
    eval(&mut vm, "(load-theme 'omarchy-catppuccin-latte t)");
    assert!(!chrome(&mut vm).drawn(), "classic chrome draws none after a theme switch");
    assert_eq!(eval(&mut vm, "(plist-get (romacs-chrome-state) :focus)"), "nil");
}

/// A launch that never loaded romacs-chrome.el draws no pane chrome, even
/// with the faces defined.
#[test]
fn without_romacs_chrome_no_pane_chrome_is_drawn() {
    let mut vm = gnu_session();
    eval(&mut vm, "(progn (defface romacs-pane-focus '((t :foreground \"#ff0000\")) \"\") (defface romacs-pane-border '((t :foreground \"#00ff00\")) \"\"))");
    assert!(!chrome(&mut vm).drawn());
}

/// Under pane chrome a window's fringes are part of its pane: painted in
/// its default background whatever the theme gives `fringe' (here a dimmer
/// one); classic chrome paints the `fringe' face, as GNU.
#[test]
fn pane_chrome_paints_fringes_in_the_window_background() {
    let mut vm = themed_session();
    eval(&mut vm, r##"(progn (deftheme fc-strip)
                         (custom-theme-set-faces 'fc-strip '(default ((t :background "#000000" :foreground "#ffffff")))
                                                 '(fringe ((t :background "#1e1e1e"))))
                         (enable-theme 'fc-strip))"##);
    let fringe_and_default = |vm: &mut Vm| {
        let frame = vm.frame_snapshot(1216, 876).unwrap();
        let window = &frame.windows[0].snapshot;
        assert!(window.chrome.left_fringe > 0 && window.chrome.right_fringe > 0, "fringes keep their width");
        (window.faces[window.chrome.fringe_face.0 as usize].background, window.faces[0].background)
    };
    let (fringe, default) = fringe_and_default(&mut vm);
    assert_eq!(fringe, default, "pane chrome: no fringe strip");
    eval(&mut vm, "(romacs-classic-chrome 1)");
    let (fringe, default) = fringe_and_default(&mut vm);
    assert_ne!(fringe, default, "classic chrome paints the theme's fringe face");
    // The parameter alone, after a classic snapshot: the window is shown again.
    eval(&mut vm, "(set-frame-parameter nil 'romacs-pane-chrome t)");
    let (fringe, default) = fringe_and_default(&mut vm);
    assert_eq!(fringe, default, "a pane-chrome switch alone shows no stale strip");
}
