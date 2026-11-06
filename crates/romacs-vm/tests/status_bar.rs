//! The root frame's status bar (VM2-STATUS-BAR) on a window-system frame
//! of 1216x876 pixels and 10x25 cells with GNU's Lisp: the band it
//! reserves above the mini-window, the window it shows, what it shows,
//! the pane labels of the windows' own bands and the height the host
//! measures for it.
use romacs_vm::Vm;
#[path = "support/gui_probe.rs"]
mod gui_probe;
use gui_probe::gnu_session;

fn eval(vm: &mut Vm, form: &str) -> String {
    vm.eval_source(form).unwrap_or_else(|e| panic!("{form}: {e}")).display().unwrap()
}

/// A session whose root frame shows the bar, its windows' mode line.
fn session() -> Vm {
    let mut vm = gnu_session();
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 1)))");
    vm
}

fn snapshot(vm: &mut Vm) -> romacs_layout_input::FrameSnapshot {
    vm.frame_snapshot(1216, 876).unwrap()
}

fn bar(vm: &mut Vm) -> romacs_layout_input::StatusBarSnapshot {
    snapshot(vm).status_bar.expect("a status bar")
}

/// The id the frame's snapshot gives the window FORM names, found by its
/// edges (no border on these frames).
fn window_id(vm: &mut Vm, form: &str) -> u64 {
    let edges = eval(vm, &format!("(list (window-pixel-left {form}) (window-pixel-top {form}))"));
    let frame = snapshot(vm);
    frame.windows.iter().find(|w| format!("({} {})", w.x - w.snapshot.chrome.insets().0, w.y) == edges)
        .unwrap_or_else(|| panic!("no window at {edges}")).snapshot.id
}

/// The band of the window FORM names in the last frame: its text, and
/// its face's background.
fn band(vm: &mut Vm, form: &str) -> Option<(String, [u8; 3])> {
    let id = window_id(vm, form);
    let frame = snapshot(vm);
    let w = &frame.windows.iter().find(|w| w.snapshot.id == id).unwrap().snapshot;
    w.mode_line.as_ref().map(|band| (band.utf8.clone(), w.faces[w.mode_line_face.0 as usize].background))
}

#[test]
fn the_bar_takes_a_band_above_the_mini_window() {
    let mut vm = gnu_session();
    let probe = "(list (window-pixel-height (frame-root-window)) (frame-height) (frame-text-height) (window-pixel-top (minibuffer-window)) (window-body-height))";
    assert_eq!(eval(&mut vm, probe), "(851 35 876 851 33)");
    assert!(snapshot(&mut vm).status_bar.is_none());
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 1)))");
    // The bar is the window's mode line: no band of its own, a body line more.
    assert_eq!(eval(&mut vm, probe), "(826 35 876 851 33)");
    assert_eq!(eval(&mut vm, "(window-mode-line-height)"), "0");
    let bar = bar(&mut vm);
    assert_eq!((bar.x, bar.y, bar.width, bar.height), (0, 826, 1216, 25));
    // Inside the internal border, as the windows are (frame.h:1796).
    eval(&mut vm, "(modify-frame-parameters nil '((internal-border-width . 6)))");
    let bar = self::bar(&mut vm);
    assert_eq!((bar.x, bar.y, bar.width, bar.height), (6, 6 + 864 - 25 - 25, 1204, 25));
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 0) (internal-border-width . 0)))");
    assert_eq!(eval(&mut vm, probe), "(851 35 876 851 33)");
}

/// `romacs-status-bar-position' `top' puts the bar under the tab bar and
/// the root window under it, the same sizes as at the bottom; switching
/// moves it at once. Layers anchored at the top start below it
/// (`romacs-chrome-top').
#[test]
fn the_bar_moves_to_the_top() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el"] { vm.load_file(&root.join(file)).unwrap(); }
    let probe = "(list (window-pixel-top (frame-root-window)) (window-pixel-height (frame-root-window)) (window-body-height) (window-pixel-top (minibuffer-window)) (romacs-chrome-top) (romacs-chrome-bottom))";
    assert_eq!(eval(&mut vm, probe), "(0 826 33 851 0 826)");
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-position . top)))");
    assert_eq!(eval(&mut vm, probe), "(25 826 33 851 25 851)");
    let shown = bar(&mut vm);
    assert_eq!((shown.x, shown.y, shown.width, shown.height), (0, 0, 1216, 25));
    // Under the tab bar.
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 1)))");
    assert_eq!(eval(&mut vm, probe), "(50 801 32 851 50 851)");
    assert_eq!(bar(&mut vm).y, 25);
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 0) (romacs-status-bar-position . bottom)))");
    assert_eq!(eval(&mut vm, probe), "(0 826 33 851 0 826)");
    assert_eq!(bar(&mut vm).y, 826);
}

#[test]
fn the_bar_drops_when_the_frame_is_too_short() {
    let mut vm = session();
    vm.set_frame_geometry(1216, 100, 25, 10).unwrap();
    // 100 pixels: a mini-window, the bar and two lines would need 100.
    assert_eq!(eval(&mut vm, "(window-pixel-height (frame-root-window))"), "50");
    vm.set_frame_geometry(1216, 99, 25, 10).unwrap();
    assert_eq!(eval(&mut vm, "(window-pixel-height (frame-root-window))"), "74");
    assert!(vm.frame_snapshot(1216, 99).unwrap().status_bar.is_none());
    // The tab bar's line counts too.
    vm.set_frame_geometry(1216, 100, 25, 10).unwrap();
    eval(&mut vm, "(modify-frame-parameters nil '((tab-bar-lines . 1)))");
    assert_eq!(eval(&mut vm, "(list (window-pixel-top (frame-root-window)) (window-pixel-height (frame-root-window)))"), "(25 50)");
    assert!(vm.frame_snapshot(1216, 100).unwrap().status_bar.is_none());
}

#[test]
fn the_bar_shows_the_selected_window_and_a_prompt_its_origin() {
    let mut vm = session();
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :background \"#101112\")");
    eval(&mut vm, "(set-face-attribute 'mode-line-inactive nil :background \"#202122\")");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-right))");
    let (w1, w2) = (window_id(&mut vm, "w1"), window_id(&mut vm, "w2"));
    let shown = bar(&mut vm);
    assert_eq!((shown.window, shown.focused), (w1, true));
    assert_eq!(shown.faces[shown.face.0 as usize].background, [0x10, 0x11, 0x12], "in mode-line-active");
    assert_eq!((band(&mut vm, "w1"), band(&mut vm, "w2")), (None, None), "no window draws a band");
    // A minibuffer entered from W1: W1 keeps the bar.
    eval(&mut vm, "(progn (setq romacs--minibuf-level 1 romacs--minibuf-selected-window w1) (select-window (minibuffer-window)))");
    assert_eq!(eval(&mut vm, "(eq (romacs-status-bar-window) w1)"), "t");
    let shown = bar(&mut vm);
    assert_eq!((shown.window, shown.focused), (w1, true));
    // Another window selected during the prompt owns it.
    eval(&mut vm, "(select-window w2)");
    assert_eq!(bar(&mut vm).window, w2);
    eval(&mut vm, "(setq romacs--minibuf-level 0)");
    // A child frame's window selected: the host frame's selected window
    // keeps the bar, unfocused.
    eval(&mut vm, "(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (width . 30) (height . 8) (left . 10) (top . 300))))");
    eval(&mut vm, "(select-frame c)");
    let shown = bar(&mut vm);
    assert_eq!((shown.window, shown.focused), (w2, false));
    assert_eq!(eval(&mut vm, "(eq (romacs-status-bar-window) w2)"), "t");
}

#[test]
fn the_bar_expands_the_mode_line_at_its_own_columns() {
    let mut vm = session();
    // `%-' shows nothing in the bar (its dashes would fill it and hide
    // the lines folded in); a string's `--', as GNU's. The painter clips
    // what the bar shows (`romacs-status-bar-string').
    let shows = |vm: &mut Vm| { bar(vm); eval(vm, "(romacs--status-bar-text)").trim_matches('"').to_owned() };
    eval(&mut vm, "(setq mode-line-format \"%b %-\")");
    assert_eq!(bar(&mut vm).text.utf8, "*scratch* ");
    assert_eq!(eval(&mut vm, "(let ((noninteractive nil)) (format-mode-line mode-line-format))"), "\"*scratch* --\"");
    // A longer format is clipped at them.
    eval(&mut vm, "(setq mode-line-format (make-string 300 ?x))");
    assert_eq!(shows(&mut vm), "x".repeat(119));
    // A window's `mode-line-format' parameter is its effective format.
    eval(&mut vm, "(set-window-parameter nil 'mode-line-format \"param %l\")");
    assert_eq!(bar(&mut vm).text.utf8, "param 1");
    // In `mode-line-active'.
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :background \"#123456\")");
    let shown = bar(&mut vm);
    assert_eq!(shown.faces[shown.face.0 as usize].background, [0x12, 0x34, 0x56]);
}

/// The bar is every window's mode line (VM2-STATUS-BAR): no window of
/// the root frame draws a band, whatever its format, so
/// `window-mode-line-height' is 0 and its body takes that line; header
/// lines stay. Without the bar (a batch frame) GNU's bands and geometry
/// return.
#[test]
fn no_window_draws_a_band_of_its_own() {
    let mut vm = session();
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below) w3 (split-window-right))");
    eval(&mut vm, "(set-window-parameter w2 'mode-line-format 'none)");
    eval(&mut vm, "(setq header-line-format \"head\")");
    for window in ["w1", "w2", "w3"] { assert_eq!(band(&mut vm, window), None, "{window}"); }
    let id = window_id(&mut vm, "w1");
    let frame = snapshot(&mut vm);
    let header = &frame.windows.iter().find(|w| w.snapshot.id == id).unwrap().snapshot.header_line;
    assert_eq!(header.as_ref().map(|h| h.utf8.as_str()), Some("head"));
    let heights = "(list (window-mode-line-height w1) (window-body-height w1 t))";
    assert_eq!(eval(&mut vm, heights), "(0 376)");
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 0)))");
    assert_eq!(eval(&mut vm, heights), "(25 363)");
    assert_eq!(band(&mut vm, "w1").map(|b| b.0.starts_with("-")), Some(true), "GNU's band again");
}

/// A session whose bar shows Romacs's mode line.
fn romacs_lines() -> Vm {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el", "romacs-icons.el", "romacs-mode-line.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    eval(&mut vm, "(progn (setq romacs-icons--font t) (romacs-mode-line-mode 1))");
    vm
}

/// A window whose buffer sets its own format (keymap-popup's line) folds
/// it into the bar while it is visible: after the selected window's line,
/// a `context' segment in `mode-line-inactive'. A nil format adds nothing.
/// A click there runs that window's line's binding: the bar maps it to
/// the window and its column (`romacs--status-bar-target').
#[test]
fn a_windows_own_line_folds_into_the_bar() {
    let mut vm = session();
    eval(&mut vm, "(set-face-attribute 'mode-line-inactive nil :background \"#202122\")");
    eval(&mut vm, "(setq mode-line-format '(\"main %b\"))");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below) w3 (split-window-right))");
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"*keymap-popup*\") (setq-local mode-line-format '(\"q  \" \"Gnosis Dashboard\")))");
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"bare\") (setq-local mode-line-format nil))");
    eval(&mut vm, "(set-window-buffer w2 \"*keymap-popup*\")");
    eval(&mut vm, "(set-window-buffer w3 \"bare\")");
    let shown = bar(&mut vm);
    assert_eq!(shown.text.utf8, "main *scratch* │ q  Gnosis Dashboard");
    let context = shown.marks.segments.iter().find(|s| s.name == "context").expect("a context segment");
    // With its separator, which goes with it when the bar is too narrow.
    assert_eq!(&shown.text.utf8[context.bytes.clone()], " │ q  Gnosis Dashboard");
    let run = shown.text.face_runs.iter().find(|r| r.bytes.start == context.bytes.start).expect("its face");
    assert_eq!(shown.faces[run.face.0 as usize].background, [0x20, 0x21, 0x22]);
    // Shown column 20 is the popup's `G', in the bar's text as a click
    // reads it; the separator is no window's.
    assert_eq!(eval(&mut vm, "(let ((target (romacs--status-bar-target 20))) (list (eq (nth 0 target) w2) (= (aref (nth 1 target) (nth 2 target)) ?G)))"), "(t t)");
    assert_eq!(eval(&mut vm, "(eq (car (romacs--status-bar-target 3)) w1)"), "t");
    assert_eq!(eval(&mut vm, "(romacs--status-bar-target 15)"), "nil");
    // Narrower than both lines, the popup goes with its separator.
    eval(&mut vm, "(setq mode-line-format (list (make-string 112 ?m)))");
    assert_eq!(bar(&mut vm).text.utf8, "m".repeat(112));
    eval(&mut vm, "(setq mode-line-format '(\"main %b\"))");
    // Gone with its window.
    eval(&mut vm, "(delete-window w2)");
    assert_eq!(bar(&mut vm).text.utf8, "main *scratch*");
}

/// A folded line's own `right-align' marker is plain text: the bar's
/// alignment is the selected line's, which here has none.
#[test]
fn a_context_marker_does_not_align_the_bar() {
    let mut vm = session();
    eval(&mut vm, "(setq-default mode-line-format '(\"MAIN\"))");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below))");
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"pop\") (setq-local mode-line-format '((:propertize \" \" romacs-segment right-align) \"POP\")))");
    eval(&mut vm, "(set-window-buffer w2 \"pop\")");
    let shown = bar(&mut vm);
    assert_eq!(shown.text.utf8, "MAIN │  POP");
    assert_eq!(shown.marks.right_align(), None);
    // A marker that is a string's own property, likewise.
    eval(&mut vm, "(with-current-buffer \"pop\" (setq-local mode-line-format (list (propertize \" \" 'romacs-segment 'right-align) \"POP\")))");
    let shown = bar(&mut vm);
    assert_eq!(shown.text.utf8, "MAIN │  POP");
    assert_eq!(shown.marks.right_align(), None);
}

/// A pill reads `romacs-pill-modified' only on the bar's own line, and
/// from the owner's buffer as it was before the line's Lisp ran, which may
/// collect: a modified owner with no line of its own leaves a context's
/// pill in its own colour; a collecting line still recolours the owner's.
#[test]
fn only_the_owners_pill_reads_its_modified_state() {
    let mut vm = session();
    vm.set_gc_stress(1);
    eval(&mut vm, "(set-face-attribute 'error nil :foreground \"#ff0000\")");
    let pill = "(:propertize \"ID\" romacs-segment id romacs-pill mode-line-highlight romacs-pill-modified error)";
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below))");
    eval(&mut vm, &format!("(with-current-buffer (get-buffer-create \"pop\") (setq-local mode-line-format '({pill})))"));
    eval(&mut vm, "(set-window-buffer w2 \"pop\")");
    eval(&mut vm, "(progn (setq-local mode-line-format nil) (insert \"x\"))");
    let shown = bar(&mut vm);
    let id = shown.marks.segments.iter().find(|s| s.name == "id").expect("the context's pill");
    assert_eq!(id.pill.and_then(|p| p.foreground), None, "a context keeps its own colour: {:?}", shown.text.utf8);
    // The owner's own line collects as it expands: its pill still recolours.
    eval(&mut vm, &format!("(setq-local mode-line-format '((:eval (progn (garbage-collect) \"\")) {pill}))"));
    let shown = bar(&mut vm);
    let id = shown.marks.segments.iter().find(|s| s.name == "id").expect("the owner's pill");
    assert_eq!(id.pill.and_then(|p| p.foreground), Some([0xff, 0, 0]));
}

/// A `romacs-icon' alist's first entry for a character decides, as
/// `assq' and the painter do: `none' drops it only when it comes first.
#[test]
fn the_first_icon_entry_decides_a_drop() {
    let mut vm = session();
    eval(&mut vm, "(setq-default mode-line-format '((:propertize \"a-b\" romacs-icon ((?- . saved) (?- . none)))))");
    assert_eq!(bar(&mut vm).text.utf8, "a-b");
    eval(&mut vm, "(setq-default mode-line-format '((:propertize \"a-b\" romacs-icon ((?- . none) (?- . saved)))))");
    assert_eq!(bar(&mut vm).text.utf8, "ab");
}

/// A projected property's value is the data its format spells, as
/// `quote' gives it: an uninterned symbol stays one, a nested string keeps
/// its multibyteness, and pieces of one `:propertize' share one value.
#[test]
fn projected_values_are_the_formats_data() {
    let mut vm = session();
    let line = |vm: &mut Vm, form: &str| eval(vm, &format!("(let* ((noninteractive nil) (s (format-mode-line {form}))) (list {}))",
        "(let ((v (get-text-property 0 'help-echo s))) (and (symbolp v) (not (eq v (intern-soft (symbol-name v)))) (equal (symbol-name v) \"tag\")))"));
    assert_eq!(line(&mut vm, "(list :propertize \"x\" 'help-echo (make-symbol \"tag\"))"), "(t)");
    assert_eq!(eval(&mut vm, "(let* ((noninteractive nil) (s (format-mode-line (list :propertize \"x\" 'help-echo (vector (string-to-multibyte \"ascii\")))))) (multibyte-string-p (aref (get-text-property 0 'help-echo s) 0)))"), "t");
    assert_eq!(eval(&mut vm, "(let* ((noninteractive nil) (s (format-mode-line '(:propertize (\"a\" \"b\") local-map (keymap (1 . ignore)))))) (eq (get-text-property 0 'local-map s) (get-text-property 1 'local-map s)))"), "t");
    assert_eq!(eval(&mut vm, "(let* ((noninteractive nil) (s (format-mode-line (list :propertize (list (propertize \"a\" 'help-echo \"x\") (propertize \"b\" 'help-echo \"y\")) 'local-map '(keymap (1 . ignore)))))) (list (and (get-text-property 0 'local-map s) (eq (get-text-property 0 'local-map s) (get-text-property 1 'local-map s))) (equal (get-text-property 0 'help-echo s) \"x\") (equal (get-text-property 1 'help-echo s) \"y\")))"), "(t t t)");
}

/// A line's `:eval' that collects moves windows: the bar resolves them
/// again by id, with split windows and popups.
#[test]
fn a_collecting_line_keeps_its_windows() {
    let mut vm = session();
    vm.set_gc_stress(1);
    eval(&mut vm, "(put 'gc-probe 'risky-local-variable t)");
    eval(&mut vm, "(setq-default mode-line-format '(:eval (progn (garbage-collect) \"MAIN\")))");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below) w3 (split-window-right))");
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"pop\") (setq-local mode-line-format '(:eval (progn (garbage-collect) \"POP\"))))");
    eval(&mut vm, "(set-window-buffer w2 \"pop\")");
    assert_eq!(bar(&mut vm).text.utf8, "MAIN │ POP");
    eval(&mut vm, "(force-mode-line-update)");
    assert_eq!(bar(&mut vm).text.utf8, "MAIN │ POP");
}

/// The bar shows what GNU's redisplay updates a mode line for, without a
/// `force-mode-line-update': a renamed buffer.
#[test]
fn a_rename_shows_at_once() {
    let mut vm = session();
    eval(&mut vm, "(setq-default mode-line-format '(\"%b\"))");
    assert_eq!(bar(&mut vm).text.utf8, "*scratch*");
    eval(&mut vm, "(rename-buffer \"renamed\")");
    assert_eq!(bar(&mut vm).text.utf8, "renamed");
}

/// A click after a `%-' reaches what the bar shows there: the bar's own
/// expansion, where `%-' shows nothing, not `format-mode-line''s `--'.
#[test]
fn a_click_after_dashes_reaches_the_shown_element() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    eval(&mut vm, "(progn (setq noninteractive nil clicked nil)
        (defvar x-map (let ((m (make-sparse-keymap))) (define-key m [mode-line mouse-1] (lambda () (interactive) (setq clicked 'x))) m))
        (setq-default mode-line-format (list \"%-\" (propertize \"X\" 'local-map x-map))))");
    assert_eq!(bar(&mut vm).text.utf8, "X");
    eval(&mut vm, "(romacs--status-bar-click 0 'mouse-1)");
    assert_eq!(eval(&mut vm, "clicked"), "x");
}

/// A bar wider than the frame drops segments by priority, the lowest
/// first, until it fits; the buffer stays. A click's column, counted in
/// what shows, reaches the expanded text's.
#[test]
fn a_full_bar_drops_segments_by_priority() {
    let mut vm = romacs_lines();
    // A name the bar does not cut in its middle.
    eval(&mut vm, "(setq romacs-mode-line-buffer-name-width 200)");
    eval(&mut vm, &format!("(rename-buffer \"{}\")", "n".repeat(100)));
    let shown = bar(&mut vm);
    let names: Vec<&str> = shown.marks.segments.iter().map(|s| s.name.as_str()).collect();
    // The mode name has no priority: the position goes, it stays.
    assert!(names.contains(&"buffer") && names.contains(&"major-mode") && !names.contains(&"position"), "{names:?}");
    assert!(shown.text.utf8.chars().count() <= 119, "{:?}", shown.text.utf8);
    // A click reads the bar's text as it shows, segments dropped.
    let length = shown.text.utf8.chars().count();
    assert_eq!(eval(&mut vm, &format!("(length (nth 1 (romacs--status-bar-target {})))", length - 1)), length.to_string());
}

/// A layout host whose rows are 25 pixels, or 5/3 of a face's pixel size
/// past 20.
struct Rows;
impl romacs_vm::LayoutHost for Rows {
    fn measure(&mut self, input: &romacs_layout_input::MeasureInput) -> Result<romacs_layout_input::Measured, String> {
        let heights: Vec<u32> = input.faces.iter().map(|f| if f.pixel_size <= 20.0 { 25 } else { (f.pixel_size * 5.0 / 3.0).round() as u32 }).collect();
        Ok(romacs_layout_input::Measured { glyphs: input.text.utf8.chars().map(|_| (10, heights[0])).collect(), ascents: input.text.utf8.chars().map(|_| heights[0] * 4 / 5).collect(), face_ascents: heights.iter().map(|h| h * 4 / 5).collect(), face_heights: heights })
    }
    fn lay_out(&mut self, frame: &romacs_layout_input::FrameSnapshot) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        Ok(frame.windows.iter().map(|w| {
            let len = w.snapshot.text.utf8.len();
            romacs_vm::WindowEnd { id: w.snapshot.id, end: len, visible_end: len, body_height: w.snapshot.height as f32, rows: vec![], point: None }
        }).collect())
    }
}

#[test]
fn a_taller_bar_face_lays_the_frame_out_again_once() {
    let mut vm = session();
    vm.eval_source(include_str!("../../../rlisp/vm/window-change.el")).unwrap();
    vm.set_layout_host(Some(Box::new(Rows)));
    snapshot(&mut vm);
    vm.display_apply_layout(&[]).unwrap();
    eval(&mut vm, "(setq changes 0)");
    eval(&mut vm, "(add-hook 'window-size-change-functions (lambda (_) (setq changes (1+ changes))))");
    assert_eq!(bar(&mut vm).height, 25);
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :height 300)");
    let shown = bar(&mut vm);
    assert!(shown.height > 25, "{}", shown.height);
    assert_eq!(shown.y, 851 - shown.height);
    assert_eq!(eval(&mut vm, "(window-pixel-height (frame-root-window))"), (851 - shown.height).to_string());
    snapshot(&mut vm);
    snapshot(&mut vm);
    assert_eq!(eval(&mut vm, "changes"), "1");
    // Too tall for a short frame it drops, and back to a line it returns.
    vm.set_frame_geometry(1216, 125, 25, 10).unwrap();
    assert!(vm.frame_snapshot(1216, 125).unwrap().status_bar.is_none());
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :height 'unspecified)");
    vm.frame_snapshot(1216, 125).unwrap();
    let shown = vm.frame_snapshot(1216, 125).unwrap().status_bar.expect("the bar again");
    assert_eq!((shown.y, shown.height), (125 - 25 - 25, 25));
}

/// `romacs-chrome-bottom' (rlisp/romacs-chrome.el): where a layer anchored
/// at the frame's bottom ends, in native pixels: the bar's top, else the
/// mini-window's (grown too), inside the internal border; with a hosted
/// minibuffer the frame's bottom, or the top of its layer while that is
/// shown and reaches below, unless it is the layer being placed.
#[test]
fn layers_end_above_the_bar_and_the_minibuffer() {
    let mut vm = gnu_session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    let bottom = |vm: &mut Vm, form: &str| eval(vm, form);
    eval(&mut vm, "(modify-frame-parameters nil '((internal-border-width . 6)))");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864 - 25).to_string());
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 1)))");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864 - 25 - 25).to_string());
    // A mini-window grown to three lines.
    eval(&mut vm, "(setq resize-mini-windows t)");
    eval(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (insert \"a\\nb\\nc\"))");
    vm.frame_snapshot(1216, 876).unwrap();
    assert_eq!(eval(&mut vm, "(window-pixel-height (minibuffer-window))"), "75");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864 - 75 - 25).to_string());
    eval(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (erase-buffer))");
    vm.frame_snapshot(1216, 876).unwrap();
    // A layer hosts the minibuffer: the bar sits at the frame's bottom.
    eval(&mut vm, "(setq c (romacs--create-frame `((parent-frame . ,(selected-frame)) (minibuffer . ,(minibuffer-window)) \
                   (width . (text-pixels . 600)) (height . (text-pixels . 100)) (child-frame-border-width . 1) (left . 100) (top . 780))))");
    eval(&mut vm, "(romacs--host-minibuffer c)");
    eval(&mut vm, "(make-frame-invisible c)");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864 - 25).to_string());
    eval(&mut vm, "(make-frame-visible c)");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), "780");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom nil c)"), (6 + 864 - 25).to_string());
    // Above the bar's top it ends nothing.
    eval(&mut vm, "(set-frame-position c 100 100)");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864 - 25).to_string());
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 0)))");
    assert_eq!(bottom(&mut vm, "(romacs-chrome-bottom)"), (6 + 864).to_string());
}

/// A click on the bar (`romacs--status-bar-click', the column vm-gui maps
/// the pointer to) runs the binding at that column of the shown window's
/// mode line: `%b''s `mode-line-buffer-identification-keymap', a format
/// element's own `local-map', and after another window is selected, that
/// window's.
#[test]
fn a_click_on_the_bar_runs_the_shown_mode_lines_binding() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    eval(&mut vm, "(setq noninteractive nil)");
    eval(&mut vm, "(progn (switch-to-buffer \"other\") (switch-to-buffer \"*scratch*\"))");
    let column = eval(&mut vm, "(string-search \"*scratch*\" (format-mode-line mode-line-format))");
    bar(&mut vm);
    eval(&mut vm, &format!("(romacs--status-bar-click {column} 'mouse-1)"));
    assert_eq!(eval(&mut vm, "(buffer-name (window-buffer))"), "\"other\"");
    eval(&mut vm, "(defvar clicked nil)");
    eval(&mut vm, "(defun bar-map (tag) (let ((m (make-sparse-keymap))) (define-key m [mode-line mouse-1] (lambda () (interactive) (push (list tag (selected-window)) clicked))) m))");
    eval(&mut vm, "(setq-default mode-line-format nil)");
    eval(&mut vm, "(with-current-buffer \"other\" (setq mode-line-format (list \"x\" (propertize \"ab\" 'local-map (bar-map 'one)))))");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-right))");
    eval(&mut vm, "(set-window-buffer w2 (get-buffer-create \"third\"))");
    eval(&mut vm, "(with-current-buffer \"third\" (setq mode-line-format (list (propertize \"cd\" 'keymap (bar-map 'two)))))");
    bar(&mut vm);
    eval(&mut vm, "(romacs--status-bar-click 2 'mouse-1)");
    assert_eq!(eval(&mut vm, "(equal clicked (list (list 'one w1)))"), "t");
    // Column 0 is no element with a map: nothing of ours runs.
    eval(&mut vm, "(romacs--status-bar-click 0 'mouse-1)");
    assert_eq!(eval(&mut vm, "(length clicked)"), "1");
    eval(&mut vm, "(select-window w2)");
    bar(&mut vm);
    eval(&mut vm, "(romacs--status-bar-click 0 'mouse-1)");
    assert_eq!(eval(&mut vm, "(equal (car clicked) (list 'two w2))"), "t");
}

/// The bar's marks under Romacs's mode line (VM2-STATUS-BAR): its
/// segments, the `centre-align' and `right-align' markers, the buffer's
/// pill on `romacs-mode-line-pill''s background, its text in
/// `romacs-mode-line-modified' while the buffer is modified, `%*' drawn
/// as an icon while Romacs Icons is there; `romacs-status-bar-string' is
/// what the bar shows, the right side flush with its right edge.
/// The buffer's pill names the pill's foreground for its text, and the mark
/// stays in its own face for the layout to keep.
#[test]
fn the_buffer_pill_names_its_text_and_leaves_the_mark_its_face() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el", "romacs-icons.el", "romacs-mode-line.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    eval(&mut vm, "(progn (setq romacs-icons--font t) (romacs-mode-line-mode 1)
                     (set-face-attribute 'romacs-mode-line-pill nil :background \"#5a1414\" :foreground \"#d4c5a0\")
                     (set-face-attribute 'romacs-mode-line-mark nil :foreground \"#e06c75\")
                     (fundamental-mode) (romacs-mode-line-refresh-mode-icon))");
    let shown = bar(&mut vm);
    let foreground = |byte: usize| {
        let run = shown.text.face_runs.iter().find(|run| run.bytes.contains(&byte)).expect("a face");
        shown.faces[run.face.0 as usize].foreground
    };
    let buffer = shown.marks.segments.iter().find(|s| s.name == "buffer").unwrap();
    assert_eq!(buffer.pill.and_then(|p| p.foreground), Some([0xd4, 0xc5, 0xa0]), "the text");
    let (mark, _) = shown.text.utf8.char_indices().find(|(_, c)| ('\u{FE000}'..='\u{FE0FF}').contains(c)).expect("the mark");
    assert_eq!(foreground(mark), [0xe0, 0x6c, 0x75], "the mark");
}

#[test]
fn the_bar_marks_its_segments_pills_and_icons() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el", "romacs-icons.el", "romacs-mode-line.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    eval(&mut vm, "(progn (setq romacs-icons--font t) (romacs-mode-line-mode 1))");
    eval(&mut vm, "(set-face-attribute 'romacs-mode-line-pill nil :background \"#5a1414\" :foreground \"#d4c5a0\")");
    let shown = bar(&mut vm);
    let names: Vec<&str> = shown.marks.segments.iter().map(|s| s.name.as_str()).collect();
    for name in ["buffer", "position", "centre-align", "right-align", "major-mode"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    // The default coding says nothing.
    assert!(!names.contains(&"encoding"), "{names:?}");
    // The buffer's identity on its pill, in the pill's text while unmodified.
    eval(&mut vm, "(set-face-attribute 'romacs-mode-line-modified nil :foreground \"#e0a030\")");
    let buffer = shown.marks.segments.iter().find(|s| s.name == "buffer").unwrap();
    assert_eq!(buffer.pill.map(|p| (p.background, p.foreground)), Some(([0x5a, 0x14, 0x14], Some([0xd4, 0xc5, 0xa0]))));
    assert!(shown.text.utf8[buffer.bytes.clone()].contains("*scratch*"));
    // `%*' of the unmodified *scratch*: nothing, its `-' dropped.
    assert!(shown.marks.icons.is_empty(), "{:?}", shown.marks.icons);
    assert!(!shown.text.utf8[buffer.bytes.clone()].contains('-'), "{:?}", shown.text.utf8);
    eval(&mut vm, "(insert \"x\")");
    let shown = bar(&mut vm);
    let buffer = shown.marks.segments.iter().find(|s| s.name == "buffer").unwrap();
    assert_eq!(buffer.pill.and_then(|p| p.foreground), Some([0xe0, 0xa0, 0x30]), "modified");
    let (byte, icon) = shown.marks.icons[0];
    assert_eq!((&shown.text.utf8[byte..byte + 1], icon), ("*", '\u{FE001}'));
    // The drawn text: the bar's 119 columns, each marker's room in whole
    // cells, the right side at the end.
    let drawn = eval(&mut vm, "(romacs-status-bar-string)");
    let drawn = drawn.trim_matches('"');
    assert_eq!(drawn.chars().count(), 118, "{drawn}");
    let marker = shown.marks.right_align().unwrap();
    let right = shown.text.utf8[marker.end..].to_owned();
    assert!(drawn.ends_with(&right), "{drawn:?} ends with {right:?}");
    // A click right of the marker reaches that element's map: the click's
    // column is the expanded text's index, as the bar's extents give it.
    eval(&mut vm, "(progn (setq noninteractive nil clicked nil)
                     (defvar right-map (let ((m (make-sparse-keymap))) (define-key m [mode-line mouse-1] (lambda () (interactive) (setq clicked 'right))) m))
                     (defvar right-probe (propertize \"RIGHT\" 'local-map right-map))
                     (put 'right-probe 'risky-local-variable t)
                     (setopt romacs-mode-line-right (append romacs-mode-line-right '(right-probe))))");
    // The column the GUI maps the pointer to counts what the bar shows.
    let shown = bar(&mut vm);
    let at = shown.text.utf8.find("RIGHT").expect("shown");
    let column = shown.text.utf8[..at].chars().count();
    assert!(shown.marks.right_align().is_some_and(|m| m.end <= at));
    eval(&mut vm, &format!("(romacs--status-bar-click {column} 'mouse-1)"));
    assert_eq!(eval(&mut vm, "clicked"), "right");
    eval(&mut vm, "(setq noninteractive t)");
    // format-mode-line carries the segments and the icon alist.
    assert_eq!(eval(&mut vm, "(let ((noninteractive nil)) (seq-uniq (delq nil (mapcar (lambda (i) (get-text-property i 'romacs-segment (format-mode-line mode-line-format))) (number-sequence 0 20)))))").contains("buffer"), true);
    // Icons only while wanted: not with `romacs-icons-ascii' or the `text'
    // source, though the font is there.
    // An option's change shows when the mode line is updated, as GNU's.
    eval(&mut vm, "(setq romacs-icons-ascii t) (force-mode-line-update)");
    assert!(bar(&mut vm).marks.icons.is_empty());
    eval(&mut vm, "(setq romacs-icons-ascii nil romacs-icons-source 'text) (force-mode-line-update)");
    assert!(bar(&mut vm).marks.icons.is_empty());
    eval(&mut vm, "(setq romacs-icons-source 'romacs) (force-mode-line-update)");
    assert_eq!(bar(&mut vm).marks.icons.len(), 1);
    // Flymake's counts, each after its glyph (error 1, warning 1, note 0),
    // refreshed when flymake reports.
    eval(&mut vm, "(progn (require 'flymake) (setq trusted-content :all)
        (defun mle-backend (report &rest _)
          (funcall report (list (flymake-make-diagnostic (current-buffer) 1 2 :error \"e\")
                                (flymake-make-diagnostic (current-buffer) 1 2 :warning \"w\"))))
        (setq-local flymake-diagnostic-functions '(mle-backend)) (flymake-mode 1) (flymake-start))");
    let shown = bar(&mut vm);
    let diagnostics = shown.marks.segments.iter().find(|s| s.name == "diagnostics").expect("a diagnostics segment");
    let counters = &shown.text.utf8[diagnostics.bytes.clone()];
    assert!(counters.contains('1') && counters.matches('1').count() >= 2, "{counters:?}");
    eval(&mut vm, "(flymake-mode -1)");
    // The state reads in the accent on a band; its pill inverts it.
    eval(&mut vm, "(set-face-attribute 'romacs-pane-focus nil :foreground \"#d48a20\")");
    eval(&mut vm, "(progn (put 'romacs-mode-line-state 'theme-face nil) (romacs-mode-line-pill-faces))");
    assert_eq!(eval(&mut vm, "(face-foreground 'romacs-mode-line-state)"), "\"#d48a20\"");
    eval(&mut vm, "(start-kbd-macro nil)");
    let state = bar(&mut vm).marks.segments.iter().find(|s| s.name == "state").cloned().expect("a state segment");
    eval(&mut vm, "(end-kbd-macro)");
    assert!(state.pill.is_some_and(|p| p.foreground.is_some()), "{state:?}");
}

/// A string's own marks reach the bar as a `:propertize''s do: a raw
/// string element's `romacs-icon' and `romacs-segment', and a string's
/// inside an outer face, which only sets the face (VM2-STATUS-BAR).
#[test]
fn a_strings_own_marks_reach_the_bar() {
    let mut vm = session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el", "romacs-icons.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    eval(&mut vm, "(progn (setq romacs-icons--font t)
        (defvar mle-raw (propertize \"*\" 'romacs-icon '((?* . modified)) 'romacs-segment 'raw))
        (defvar mle-under (propertize \"ab\" 'romacs-segment 'under 'romacs-pill 'mode-line-highlight))
        (put 'mle-raw 'risky-local-variable t) (put 'mle-under 'risky-local-variable t)
        (setq-default mode-line-format '(\"x\" mle-raw \" \" (:propertize mle-under face bold))))");
    let shown = bar(&mut vm);
    assert!(shown.text.utf8.starts_with("x* ab"), "{:?}", shown.text.utf8);
    assert_eq!(shown.marks.icons, vec![(1, '\u{FE001}')]);
    let segments: Vec<(&str, std::ops::Range<usize>, bool)> = shown.marks.segments.iter()
        .map(|s| (s.name.as_str(), s.bytes.clone(), s.pill.is_some())).collect();
    assert_eq!(segments, vec![("raw", 1..2, false), ("under", 3..5, true)]);
}

/// `romacs-status-bar-string' is what the painter places: the host's
/// advances inside the bar's padding, a long text clipped where the bar
/// ends, a right side wider than the bar from its start; a centred zone
/// on the bar's middle, after the left zone where the room is short.
#[test]
fn the_bar_string_follows_the_painter() {
    let mut vm = session();
    vm.set_layout_host(Some(Box::new(Rows)));
    vm.set_frame_geometry(120, 876, 25, 10).unwrap();
    vm.frame_snapshot(120, 876).unwrap();
    vm.display_apply_layout(&[]).unwrap();
    let shows = |vm: &mut Vm| { vm.frame_snapshot(120, 876).unwrap(); eval(vm, "(romacs-status-bar-string)") };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    // 120 pixels less 12 of padding at either end: 9 cells of 10.
    eval(&mut vm, "(setq mode-line-format (make-string 300 ?x))");
    assert_eq!(shows(&mut vm), format!("{:?}", "x".repeat(9)));
    eval(&mut vm, "(setq mode-line-format (list \"L\" '(:propertize \" \" romacs-segment right-align) (make-string 30 ?R)))");
    assert_eq!(shows(&mut vm), format!("{:?}", "R".repeat(9)));
    eval(&mut vm, "(setq mode-line-format (list \"LL\" '(:propertize \" \" romacs-segment right-align) \"RRR\"))");
    assert_eq!(shows(&mut vm), format!("{:?}", format!("LL{}RRR", " ".repeat(4))));
    // A left side longer than the expansion keeps of it leaves the right
    // side whole.
    eval(&mut vm, "(setq mode-line-format (list (make-string 60 ?L) '(:propertize \" \" romacs-segment right-align) \"RRR\"))");
    assert_eq!(shows(&mut vm), format!("{:?}", format!("{}RRR", "L".repeat(6))));
    // The centre on the bar's middle; with no room for the gaps either
    // side of it, after the left.
    let zoned = |l: &str, c: &str, r: &str| format!("(setq mode-line-format (list {l:?} '(:propertize \" \" romacs-segment centre-align) {c:?} '(:propertize \" \" romacs-segment right-align) {r:?}))");
    eval(&mut vm, &zoned("L", "C", "R"));
    assert_eq!(shows(&mut vm), format!("{:?}", "L   C   R"));
    eval(&mut vm, &zoned("LLL", "CC", "RRR"));
    assert_eq!(shows(&mut vm), format!("{:?}", "LLLCC RRR"));
    // An empty centre: its room is counted once.
    eval(&mut vm, &zoned("LL", "", "RR"));
    assert_eq!(shows(&mut vm), format!("{:?}", "LL     RR"));
    // A left side clipped: the centre shows nothing, no room past the bar.
    eval(&mut vm, &zoned(&"L".repeat(60), "C", "R"));
    assert_eq!(shows(&mut vm), format!("{:?}", format!("{}R", "L".repeat(8))));
}

/// A buffer's `face-remapping-alist' remaps its bands' faces: the bar's
/// base face while the bar shows that buffer's line (its last expansion
/// is not reused), and its window's header line.
#[test]
fn a_buffers_face_remapping_reaches_its_bands() {
    let mut vm = session();
    let base = |b: &romacs_layout_input::StatusBarSnapshot| b.faces[b.face.0 as usize].background;
    let plain = base(&bar(&mut vm));
    eval(&mut vm, "(progn (setq header-line-format \"head\")
                     (setq-local face-remapping-alist
                       (list '(mode-line-active (:background \"#123456\"))
                             (list (if (facep 'header-line-active) 'header-line-active 'header-line) '(:background \"#654321\")))))");
    let remapped = base(&bar(&mut vm));
    assert_ne!(plain, [0x12, 0x34, 0x56]);
    assert_eq!(remapped, [0x12, 0x34, 0x56], "the bar's face follows the buffer's remapping");
    let id = window_id(&mut vm, "(selected-window)");
    let frame = snapshot(&mut vm);
    let w = &frame.windows.iter().find(|w| w.snapshot.id == id).unwrap().snapshot;
    assert_eq!(w.header_line.as_ref().map(|h| h.utf8.as_str()), Some("head"));
    assert_eq!(w.faces[w.header_line_face.0 as usize].background, [0x65, 0x43, 0x21], "the header line's face follows it too");
    // Undone, the bar's face is the theme's again.
    eval(&mut vm, "(kill-local-variable 'face-remapping-alist)");
    assert_eq!(base(&bar(&mut vm)), plain);
    // A remapping of the parent faces alone reaches the faces that inherit
    // them, as pinned GNU's (merge_named_face reads the remapping: there,
    // `mode-line' and `header-line' at :height 2.0 grow both bands from 20
    // to 38 pixels, as their `-active' faces would). The default theme's
    // `-active' faces only inherit, so a package's remapping of `mode-line'
    // (`face-remap-add-relative') reaches the bar.
    eval(&mut vm, "(progn (face-remap-add-relative 'mode-line :background \"#123456\")
                     (face-remap-add-relative 'header-line :background \"#654321\"))");
    let faces = eval(&mut vm, "(list (face-attribute 'mode-line-active :inherit) (face-attribute 'mode-line-active :background))");
    assert_eq!(base(&bar(&mut vm)), [0x12, 0x34, 0x56], "`mode-line' reaches `mode-line-active' {faces}");
    let frame = snapshot(&mut vm);
    let w = &frame.windows.iter().find(|w| w.snapshot.id == id).unwrap().snapshot;
    assert_eq!(w.faces[w.header_line_face.0 as usize].background, [0x65, 0x43, 0x21], "`header-line' reaches `header-line-active'");
}
