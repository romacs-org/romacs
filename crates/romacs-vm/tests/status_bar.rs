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

/// A session whose root frame shows the bar, its windows' bands LABEL.
fn session(label: &str) -> Vm {
    let mut vm = gnu_session();
    eval(&mut vm, &format!("(modify-frame-parameters nil '((romacs-status-bar-lines . 1) (romacs-pane-label . {label})))"));
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
    assert_eq!(eval(&mut vm, probe), "(826 35 876 851 32)");
    let bar = bar(&mut vm);
    assert_eq!((bar.x, bar.y, bar.width, bar.height), (0, 826, 1216, 25));
    // Inside the internal border, as the windows are (frame.h:1796).
    eval(&mut vm, "(modify-frame-parameters nil '((internal-border-width . 6)))");
    let bar = self::bar(&mut vm);
    assert_eq!((bar.x, bar.y, bar.width, bar.height), (6, 6 + 864 - 25 - 25, 1204, 25));
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-status-bar-lines . 0) (internal-border-width . 0)))");
    assert_eq!(eval(&mut vm, probe), "(851 35 876 851 33)");
}

#[test]
fn the_bar_drops_when_the_frame_is_too_short() {
    let mut vm = session("full");
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
    let mut vm = session("full");
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :background \"#101112\")");
    eval(&mut vm, "(set-face-attribute 'mode-line-inactive nil :background \"#202122\")");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-right))");
    let (w1, w2) = (window_id(&mut vm, "w1"), window_id(&mut vm, "w2"));
    let shown = bar(&mut vm);
    assert_eq!((shown.window, shown.focused), (w1, true));
    let active = band(&mut vm, "w1").unwrap().1;
    let inactive = band(&mut vm, "w2").unwrap().1;
    assert_ne!(active, inactive);
    // A minibuffer entered from W1: W1 keeps the active band and the bar.
    eval(&mut vm, "(progn (setq romacs--minibuf-level 1 romacs--minibuf-selected-window w1) (select-window (minibuffer-window)))");
    assert_eq!(eval(&mut vm, "(eq (romacs-status-bar-window) w1)"), "t");
    let shown = bar(&mut vm);
    assert_eq!((shown.window, shown.focused), (w1, true));
    assert_eq!(band(&mut vm, "w1").unwrap().1, active);
    assert_eq!(band(&mut vm, "w2").unwrap().1, inactive);
    // Another window selected during the prompt owns both.
    eval(&mut vm, "(select-window w2)");
    assert_eq!(bar(&mut vm).window, w2);
    assert_eq!(band(&mut vm, "w2").unwrap().1, active);
    assert_eq!(band(&mut vm, "w1").unwrap().1, inactive);
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
    let mut vm = session("full");
    // `%-' fills the bar's 121 columns; a string's `--'. The painter
    // clips what the bar shows there (`romacs-status-bar-string').
    let shows = |vm: &mut Vm| { bar(vm); eval(vm, "(romacs--status-bar-text)").trim_matches('"').to_owned() };
    eval(&mut vm, "(setq mode-line-format \"%b %-\")");
    assert!(bar(&mut vm).text.utf8.starts_with(&format!("*scratch* {}", "-".repeat(121))));
    let text = shows(&mut vm);
    assert_eq!(text.chars().count(), 121, "{text:?}");
    assert!(text.starts_with("*scratch* ---"), "{text}");
    assert_eq!(eval(&mut vm, "(let ((noninteractive nil)) (format-mode-line mode-line-format))"), "\"*scratch* --\"");
    // A longer format is clipped at them.
    eval(&mut vm, "(setq mode-line-format (make-string 300 ?x))");
    assert_eq!(shows(&mut vm), "x".repeat(121));
    // A window's `mode-line-format' parameter is its effective format.
    eval(&mut vm, "(set-window-parameter nil 'mode-line-format \"param %l\")");
    assert_eq!(bar(&mut vm).text.utf8, "param 1");
    // In `mode-line-active'.
    eval(&mut vm, "(set-face-attribute 'mode-line-active nil :background \"#123456\")");
    let shown = bar(&mut vm);
    assert_eq!(shown.faces[shown.face.0 as usize].background, [0x12, 0x34, 0x56]);
}

#[test]
fn pane_labels_replace_only_the_bands_gnu_draws() {
    let mut vm = session("name");
    eval(&mut vm, "(set-face-attribute 'mode-line-inactive nil :background \"#202122\")");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-below) w3 (split-window-right))");
    eval(&mut vm, "(set-window-parameter w2 'mode-line-format 'none)");
    eval(&mut vm, "(with-current-buffer (get-buffer-create \"bare\") (setq mode-line-format nil))");
    eval(&mut vm, "(set-window-buffer w3 \"bare\")");
    eval(&mut vm, "(setq header-line-format \"head\")");
    // The selected window's label too is inactive: the bar is the active line.
    assert_eq!(band(&mut vm, "w1"), Some(("*scratch*".into(), [0x20, 0x21, 0x22])));
    for label in ["name", "full", "nil"] {
        eval(&mut vm, &format!("(modify-frame-parameters nil '((romacs-pane-label . {label})))"));
        assert_eq!(band(&mut vm, "w2"), None, "{label}");
        assert_eq!(band(&mut vm, "w3"), None, "{label}");
        let id = window_id(&mut vm, "w1");
        let frame = snapshot(&mut vm);
        let header = &frame.windows.iter().find(|w| w.snapshot.id == id).unwrap().snapshot.header_line;
        assert_eq!(header.as_ref().map(|h| h.utf8.as_str()), Some("head"), "{label}");
    }
    let heights = "(list (window-mode-line-height w1) (window-body-height w1 t))";
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-pane-label . full)))");
    assert_eq!(eval(&mut vm, heights), "(25 351)");
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-pane-label . nil)))");
    assert_eq!(eval(&mut vm, heights), "(0 376)");
    assert_eq!(band(&mut vm, "w1"), None);
    // A label is a format of its own.
    eval(&mut vm, "(modify-frame-parameters nil '((romacs-pane-label . name)))");
    eval(&mut vm, "(setq romacs-pane-label-format '(\"<\" \"%b\" \">\"))");
    assert_eq!(band(&mut vm, "w1").unwrap().0, "<*scratch*>");
}

/// A layout host whose rows are 25 pixels, or 5/3 of a face's pixel size
/// past 20.
struct Rows;
impl romacs_vm::LayoutHost for Rows {
    fn measure(&mut self, input: &romacs_layout_input::MeasureInput) -> Result<romacs_layout_input::Measured, String> {
        let heights: Vec<u32> = input.faces.iter().map(|f| if f.pixel_size <= 20.0 { 25 } else { (f.pixel_size * 5.0 / 3.0).round() as u32 }).collect();
        Ok(romacs_layout_input::Measured { glyphs: input.text.utf8.chars().map(|_| (10, heights[0])).collect(), face_heights: heights })
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
    let mut vm = session("name");
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
    let mut vm = session("name");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    eval(&mut vm, "(setq noninteractive nil)");
    eval(&mut vm, "(progn (switch-to-buffer \"other\") (switch-to-buffer \"*scratch*\"))");
    let column = eval(&mut vm, "(string-search \"*scratch*\" (format-mode-line mode-line-format))");
    eval(&mut vm, &format!("(romacs--status-bar-click {column} 'mouse-1)"));
    assert_eq!(eval(&mut vm, "(buffer-name (window-buffer))"), "\"other\"");
    eval(&mut vm, "(defvar clicked nil)");
    eval(&mut vm, "(defun bar-map (tag) (let ((m (make-sparse-keymap))) (define-key m [mode-line mouse-1] (lambda () (interactive) (push (list tag (selected-window)) clicked))) m))");
    eval(&mut vm, "(setq-default mode-line-format nil)");
    eval(&mut vm, "(with-current-buffer \"other\" (setq mode-line-format (list \"x\" (propertize \"ab\" 'local-map (bar-map 'one)))))");
    eval(&mut vm, "(setq w1 (selected-window) w2 (split-window-right))");
    eval(&mut vm, "(set-window-buffer w2 (get-buffer-create \"third\"))");
    eval(&mut vm, "(with-current-buffer \"third\" (setq mode-line-format (list (propertize \"cd\" 'keymap (bar-map 'two)))))");
    eval(&mut vm, "(romacs--status-bar-click 2 'mouse-1)");
    assert_eq!(eval(&mut vm, "(equal clicked (list (list 'one w1)))"), "t");
    // Column 0 is no element with a map: nothing of ours runs.
    eval(&mut vm, "(romacs--status-bar-click 0 'mouse-1)");
    assert_eq!(eval(&mut vm, "(length clicked)"), "1");
    eval(&mut vm, "(select-window w2)");
    eval(&mut vm, "(romacs--status-bar-click 0 'mouse-1)");
    assert_eq!(eval(&mut vm, "(equal (car clicked) (list 'two w2))"), "t");
}

/// The bar's marks under Romacs's mode line (VM2-STATUS-BAR): its
/// segments, the `right-align' marker, the buffer's pill on
/// `romacs-mode-line-pill''s background, `%*' drawn as an icon while
/// Romacs Icons is there; `romacs-status-bar-string' is what the bar
/// shows, the right side flush with its right edge.
#[test]
fn the_bar_marks_its_segments_pills_and_icons() {
    let mut vm = session("name");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp");
    for file in ["romacs-layer.el", "romacs-chrome.el", "romacs-icons.el", "romacs-mode-line.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    eval(&mut vm, "(progn (setq romacs-icons--font t) (romacs-mode-line-mode 1))");
    eval(&mut vm, "(set-face-attribute 'romacs-mode-line-pill nil :background \"#5a1414\")");
    let shown = bar(&mut vm);
    let names: Vec<&str> = shown.marks.segments.iter().map(|s| s.name.as_str()).collect();
    for name in ["front", "buffer", "position", "right-align", "major-mode", "encoding"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    let buffer = shown.marks.segments.iter().find(|s| s.name == "buffer").unwrap();
    assert_eq!(buffer.pill.map(|p| p.background), Some([0x5a, 0x14, 0x14]));
    assert!(shown.text.utf8[buffer.bytes.clone()].contains("*scratch*"));
    // `%*' of the unmodified *scratch*: `-', drawn as the `saved' icon.
    let (byte, icon) = shown.marks.icons[0];
    assert_eq!((&shown.text.utf8[byte..byte + 1], icon), ("-", '\u{FE002}'));
    eval(&mut vm, "(insert \"x\")");
    let shown = bar(&mut vm);
    let (byte, icon) = shown.marks.icons[0];
    assert_eq!((&shown.text.utf8[byte..byte + 1], icon), ("*", '\u{FE001}'));
    // The drawn text: the bar's 121 columns, the right side at the end.
    let drawn = eval(&mut vm, "(romacs-status-bar-string)");
    let drawn = drawn.trim_matches('"');
    assert_eq!(drawn.chars().count(), 121, "{drawn}");
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
    let column = eval(&mut vm, "(string-search \"RIGHT\" (format-mode-line mode-line-format))");
    let marker = eval(&mut vm, "(string-search \" \" (format-mode-line mode-line-format) (next-single-property-change 0 'romacs-segment (format-mode-line mode-line-format) ))");
    assert!(column.parse::<usize>().unwrap() > marker.parse::<usize>().unwrap_or(0));
    eval(&mut vm, &format!("(romacs--status-bar-click {column} 'mouse-1)"));
    assert_eq!(eval(&mut vm, "clicked"), "right");
    eval(&mut vm, "(setq noninteractive t)");
    // format-mode-line carries the segments and the icon alist.
    assert_eq!(eval(&mut vm, "(let ((noninteractive nil)) (seq-uniq (delq nil (mapcar (lambda (i) (get-text-property i 'romacs-segment (format-mode-line mode-line-format))) (number-sequence 0 20)))))").contains("buffer"), true);
    // Icons only while wanted: not with `romacs-icons-ascii' or the `text'
    // source, though the font is there.
    eval(&mut vm, "(setq romacs-icons-ascii t)");
    assert!(bar(&mut vm).marks.icons.is_empty());
    eval(&mut vm, "(setq romacs-icons-ascii nil romacs-icons-source 'text)");
    assert!(bar(&mut vm).marks.icons.is_empty());
    eval(&mut vm, "(setq romacs-icons-source 'romacs)");
    assert_eq!(bar(&mut vm).marks.icons.len(), 1);
    // Flymake's counters are its own `:propertize' forms: they stay inside
    // the diagnostics segment's pill.
    eval(&mut vm, "(progn (require 'flymake) (setq trusted-content :all)
        (defun mle-backend (report &rest _)
          (funcall report (list (flymake-make-diagnostic (current-buffer) 1 2 :error \"e\")
                                (flymake-make-diagnostic (current-buffer) 1 2 :warning \"w\"))))
        (setq-local flymake-diagnostic-functions '(mle-backend)) (flymake-mode 1) (flymake-start))");
    let shown = bar(&mut vm);
    let diagnostics = shown.marks.segments.iter().find(|s| s.name == "diagnostics").expect("a diagnostics segment");
    assert!(diagnostics.pill.is_some());
    let counters = &shown.text.utf8[diagnostics.bytes.clone()];
    assert!(counters.contains('1') && counters.matches('1').count() >= 2, "{counters:?}");
    eval(&mut vm, "(flymake-mode -1)");
    // The state reads in the accent on a band; its pill inverts it.
    eval(&mut vm, "(set-face-attribute 'romacs-pane-focus nil :foreground \"#d48a20\")");
    eval(&mut vm, "(setq romacs-mode-line--pill-colors nil) (set-face-attribute 'romacs-mode-line-state nil :foreground 'unspecified) (romacs-mode-line-pill-faces)");
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
    let mut vm = session("name");
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
/// advances right of the accent block, a long text clipped where the bar
/// ends, a right side wider than the bar from its start.
#[test]
fn the_bar_string_follows_the_painter() {
    let mut vm = session("full");
    vm.set_layout_host(Some(Box::new(Rows)));
    vm.set_frame_geometry(120, 876, 25, 10).unwrap();
    vm.frame_snapshot(120, 876).unwrap();
    vm.display_apply_layout(&[]).unwrap();
    let shows = |vm: &mut Vm| { vm.frame_snapshot(120, 876).unwrap(); eval(vm, "(romacs-status-bar-string)") };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rlisp/romacs-chrome.el");
    vm.load_file(&root).unwrap();
    // 120 pixels less the 3-pixel accent: 11 cells of 10.
    eval(&mut vm, "(setq mode-line-format (make-string 300 ?x))");
    assert_eq!(shows(&mut vm), format!("{:?}", "x".repeat(11)));
    eval(&mut vm, "(setq mode-line-format (list \"L\" '(:propertize \" \" romacs-segment right-align) (make-string 30 ?R)))");
    assert_eq!(shows(&mut vm), format!("{:?}", "R".repeat(11)));
    eval(&mut vm, "(setq mode-line-format (list \"LL\" '(:propertize \" \" romacs-segment right-align) \"RRR\"))");
    assert_eq!(shows(&mut vm), format!("{:?}", format!("LL{}RRR", " ".repeat(6))));
    // A left side longer than the expansion keeps of it leaves the right
    // side whole.
    eval(&mut vm, "(setq mode-line-format (list (make-string 60 ?L) '(:propertize \" \" romacs-segment right-align) \"RRR\"))");
    assert_eq!(shows(&mut vm), format!("{:?}", format!("{}RRR", "L".repeat(8))));
}
