//! Tier B: GNU's C mode-line engine (`src/xdisp.c:display_mode_element`,
//! `:28458`, and `decode_mode_spec`, `:29429`) over VM2's own state.
//!
//! Every expected string is GNU 32.0.50's `format-mode-line` answer for the
//! same buffer, window geometry, start and point, recorded on a tty frame with
//! `crates/romacs-vm-gui/evidence/tierb/gnu/mode-line.el` (transcript
//! `mode-line.txt` beside it). Where VM2 lacks the C state GNU reads (coding
//! systems, `mode-name`'s buffer.c default, a save point), the test pins what
//! VM2 really shows and DIVERGENCES.md VM2-G1B says so.
use romacs_vm::Vm;

const LINE: u32 = 20;
const COLUMN: u32 = 10;
/// GNU's probe window: 23 lines total, 22 of text, 80 columns.
const WIDTH: u32 = 80 * COLUMN;
const HEIGHT: u32 = (23 + 1) * LINE;

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

/// GNU's probe buffer: `line N` for 200 lines, line 3 is `κόσμε<TAB>x`.
fn probe() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(WIDTH, HEIGHT, LINE, COLUMN).unwrap();
    eval(&mut vm, r#"(progn (set-buffer (get-buffer-create "probe"))
        (let ((i 0)) (while (< i 200)
          (insert (if (= i 2) "κόσμε	x
" (format "line %d
" (1+ i))))
          (setq i (1+ i))))
        (set-window-buffer nil (current-buffer)) t)"#);
    vm
}

/// GNU `ml-case`: force the start to START-LINE, put point on POINT-LINE.
fn case(vm: &mut Vm, start_line: usize, point_line: usize, eol: bool) {
    eval(vm, &format!(
        "(progn (goto-char (point-min)) (forward-line {}) (set-window-start nil (point)) \
                (goto-char (point-min)) (forward-line {}) {} t)",
        start_line - 1, point_line - 1, if eol { "(end-of-line)" } else { "" }));
}

fn band(vm: &mut Vm, format: &str) -> String {
    eval(vm, &format!("(setq mode-line-format '{format})"));
    // A refused frame is reported as text so it fails at the comparison
    // that names the construct, not at an unwrap.
    current_band(vm)
}

fn current_band(vm: &mut Vm) -> String {
    match vm.frame_snapshot(WIDTH, HEIGHT) {
        Ok(snapshot) => snapshot.windows[0].snapshot.mode_line.as_ref().map_or("<no mode line>".into(), |b| b.utf8.clone()),
        Err(error) => format!("<frame refused: {error}>"),
    }
}

/// (spec, GNU A, GNU B, GNU C, GNU E-narrow) from `mode-line.txt`.
const SPECS: &[(&str, &str, &str, &str, &str)] = &[
    ("%b", "probe", "probe", "probe", "probe"),
    ("%f", "", "", "", ""),
    ("%l", "3", "100", "201", "2"),
    ("%c", "9", "0", "0", "0"),
    ("%C", "10", "1", "1", "1"),
    ("%p", "Top", "44%", "Bottom", "All"),
    ("%P", "Top10%", "55%", "Bottom", "All"),
    ("%o", "Top", "50%", "Bottom", "All"),
    ("%q", "0-10%", "44-55%", "95-100%", "All   "),
    ("%i", "1693", "1693", "1693", "40"),
    ("%I", "1.7k", "1.7k", "1.7k", "40"),
    ("%n", "", "", "", " Narrow"),
    ("%@", "-", "-", "-", "-"),
    ("%e", "", "", "", ""),
    ("%[", "", "", "", ""),
    ("%]", "", "", "", ""),
    ("%%", "%", "%", "%", "%"),
    ("%s", "no process", "no process", "no process", "no process"),
    ("%M", "", "", "", ""),
    ("%5l|", "    3|", "  100|", "  201|", "    2|"),
    ("%3c|", "  9|", "  0|", "  0|", "  0|"),
    ("%8p|", "Top     |", "44%     |", "Bottom  |", "All     |"),
    ("%2b|", "probe|", "probe|", "probe|", "probe|"),
    ("%12b|", "probe       |", "probe       |", "probe       |", "probe       |"),
    ("%6i|", "  1693|", "  1693|", "  1693|", "    40|"),
    ("%6I|", "   1.7k|", "   1.7k|", "   1.7k|", "    40|"),
    ("x%7%y", "x%      y", "x%      y", "x%      y", "x%      y"),
];

#[test]
fn percent_constructs_match_gnu_in_every_recorded_window_state() {
    for (column, (start, point, eol)) in [(1, 3, true), (94, 100, false), (190, 201, false)].into_iter().enumerate() {
        let mut vm = probe();
        case(&mut vm, start, point, eol);
        let label = ["A", "B", "C"][column];
        for &(spec, a, b, c, _) in SPECS {
            let expected = [a, b, c][column];
            assert_eq!(band(&mut vm, &format!("({spec:?})")), expected, "case {label} {spec}");
        }
    }
    // GNU E-narrow: (narrow-to-region 20 60), start forced to 1 (clamped to
    // BEGV 20), point on the second accessible line.
    let mut vm = probe();
    eval(&mut vm, "(progn (narrow-to-region 20 60) t)");
    case(&mut vm, 1, 2, false);
    for &(spec, _, _, _, e) in SPECS {
        assert_eq!(band(&mut vm, &format!("({spec:?})")), e, "case E-narrow {spec}");
    }
    // GNU F-narrow-scrolled: (narrow-to-region 200 1500), start on accessible
    // line 30, point on 40: BEGV, ZV, start and window end all differ, so
    // every percentage's numerator and denominator are exercised.
    let mut vm = probe();
    eval(&mut vm, "(progn (narrow-to-region 200 1500) t)");
    case(&mut vm, 30, 40, false);
    for (spec, expected) in [("%p", "18%"), ("%P", "31%"), ("%o", "21%"), ("%q", "18-31%"), ("%l", "40"),
                             ("%i", "1300"), ("%I", "1.3k"), ("%n", " Narrow"), ("%c", "0")] {
        assert_eq!(band(&mut vm, &format!("({spec:?})")), expected, "case F-narrow-scrolled {spec}");
    }
}

#[test]
fn native_modified_transitions_and_properties_share_display_state_after_gc() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(WIDTH, HEIGHT, LINE, COLUMN).unwrap();
    eval(&mut vm, r#"(progn (set-buffer (get-buffer-create "shared-state")) (set-window-buffer nil (current-buffer)) (insert "abc") (set-buffer-modified-p nil))"#);
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "---");
    eval(&mut vm, "(put-text-property 1 2 'shared-property t)");
    vm.collect_garbage();
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "***");
    eval(&mut vm, "(set-buffer-modified-p nil)");
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "---");
    eval(&mut vm, "(restore-buffer-modified-p t)");
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "***");
}

#[test]
fn modified_and_read_only_flags_follow_the_buffer_counters() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(WIDTH, HEIGHT, LINE, COLUMN).unwrap();
    eval(&mut vm, r#"(progn (set-buffer (get-buffer-create "fresh")) (set-window-buffer nil (current-buffer)) t)"#);
    // A buffer nobody changed: MODIFF == SAVE_MODIFF.
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "---");
    eval(&mut vm, "(defvar buffer-read-only nil)");
    eval(&mut vm, "(setq buffer-read-only t)");
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "%%-", "GNU RO-clean");
    eval(&mut vm, "(setq buffer-read-only nil)");
    eval(&mut vm, r#"(insert "z")"#);
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "***", "GNU D-modified");
    eval(&mut vm, "(setq buffer-read-only t)");
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "%**", "GNU RO-modified");
    // A long insertion takes the rope's replace path, not the append fast
    // path; GNU's insdel.c counts every real change the same way.
    eval(&mut vm, "(setq buffer-read-only nil)");
    eval(&mut vm, r#"(progn (set-buffer (get-buffer-create "long")) (set-window-buffer nil (current-buffer)) t)"#);
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "---", "fresh long buffer");
    eval(&mut vm, r#"(insert (make-string 600 ?x))"#);
    assert_eq!(band(&mut vm, r#"("%*%+%&")"#), "***", "GNU D-modified by a long insertion");
}

#[test]
fn constructs_match_gnu_display_mode_element() {
    let mut vm = probe();
    case(&mut vm, 1, 3, true);
    eval(&mut vm, r#"(progn
        (setq ml-risky-free (list "R" (list :eval "E") (list :propertize "P" 'face 'bold)))
        (setq ml-risky-marked (list "R" (list :eval "E") (list :propertize "P" 'face 'bold)))
        (put 'ml-risky-marked 'risky-local-variable t)
        (setq ml-str "%b") t)"#);
    for (format, gnu) in [
        (r#"("a" "b" "c")"#, "abc"),
        (r#"(ml-unbound-var "yes" "no")"#, "no"),
        (r#"(ml-unbound-var "yes")"#, ""),
        (r#"(t "T" "F")"#, "T"),
        (r#"(nil "T" "F")"#, "F"),
        (r#"(-5 "abcdefgh")"#, "abcde"),
        (r#"(5 "ab")"#, "ab   "),
        (r#"(-3 "%b")"#, "pro"),
        (r#"(12 "%b")"#, "probe       "),
        (r#"(3 . "ab")"#, "ab "),
        (r#"("" (-4 "%b") "|" (6 "x") "|")"#, "prob|x     |"),
        (r#"("" (8 (-3 "abcdef")) "|")"#, "abc     |"),
        (r#"("" (-6 (10 "ab")) "|")"#, "ab    |"),
        (r#"(:eval (concat "E" "V"))"#, "EV"),
        (r#"(:propertize "P" face bold)"#, "P"),
        (r#"("" ml-str)"#, "%b"),
        (r#"("" ml-risky-free)"#, "R"),
        (r#"("" ml-risky-marked)"#, "REP"),
        ("42", "*invalid*"),
        (r#"("" 42)"#, "*invalid*"),
        ("(foo . bar)", "*invalid*"),
        (r#"("" (:eval 7))"#, "*invalid*"),
    ] {
        assert_eq!(band(&mut vm, format), gnu, "{format}");
    }
}

#[test]
fn dashes_fill_the_displayed_mode_line() {
    // Display target: `%-` is FRAME_MESSAGE_BUF_SIZE - 1 = 4 * cols - 1
    // dashes (xdisp.c:29511), clipped by the window edge.
    let mut vm = probe();
    let text = band(&mut vm, r#"("%-" "|")"#);
    assert_eq!(text, format!("{}|", "-".repeat(4 * 80 - 1)));
}

#[test]
fn each_window_formats_its_own_buffer_local_mode_line() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(WIDTH, HEIGHT, LINE, COLUMN).unwrap();
    eval(&mut vm, r#"(progn
        (set-default 'mode-line-format '("global %b"))
        (set-buffer (get-buffer-create "left")) (make-local-variable 'mode-line-format)
        (setq mode-line-format '("left-local %b"))
        (set-window-buffer nil (current-buffer))
        (set-window-new-pixel (selected-window) 400)
        (setq other (split-window-internal (selected-window) 400 t 0.5))
        (set-window-buffer other (get-buffer-create "right"))
        (set-buffer (get-buffer-create "unrelated")) t)"#);
    let snapshot = vm.frame_snapshot(WIDTH, HEIGHT).unwrap();
    let bands: Vec<_> = snapshot.windows.iter().filter_map(|w| w.snapshot.mode_line.as_ref()).map(|b| b.utf8.as_str()).collect();
    assert_eq!(bands, ["left-local left", "global right"]);
}

#[test]
fn selected_window_uses_the_active_face_and_others_the_inactive_face() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(WIDTH, HEIGHT, LINE, COLUMN).unwrap();
    eval(&mut vm, r##"(progn
        (defface mode-line '((t (:foreground "#ffffff" :background "#303030"))) "m")
        (defface mode-line-inactive '((t (:foreground "#808080" :background "#101010"))) "i")
        (setq mode-line-format '("%b"))
        (set-window-new-pixel (selected-window) 240)
        (setq lower (split-window-internal (selected-window) 220 nil 0.5)) t)"##);
    let backgrounds = |vm: &mut Vm| {
        let snapshot = vm.frame_snapshot(WIDTH, HEIGHT).unwrap();
        snapshot.windows.iter().filter(|w| w.snapshot.mode_line.is_some())
            .map(|w| w.snapshot.faces[w.snapshot.mode_line_face.0 as usize].background).collect::<Vec<_>>()
    };
    // No `mode-line-active` face yet: the selected window falls back to `mode-line`.
    assert_eq!(backgrounds(&mut vm), [[0x30; 3], [0x10; 3]]);
    eval(&mut vm, "(progn (select-window lower) t)");
    assert_eq!(backgrounds(&mut vm), [[0x10; 3], [0x30; 3]], "selection moves the active face");
    eval(&mut vm, r##"(progn (defface mode-line-active '((t (:background "#505050"))) "a") t)"##);
    assert_eq!(backgrounds(&mut vm), [[0x10; 3], [0x50; 3]]);
    eval(&mut vm, "(progn (setq mode-line-in-non-selected-windows nil) t)");
    assert_eq!(backgrounds(&mut vm), [[0x50; 3], [0x50; 3]], "GNU: every window uses the active face");
}

#[test]
fn propertize_faces_reach_the_painted_mode_line() {
    let mut vm = probe();
    eval(&mut vm, r##"(progn (defface success '((t (:foreground "#00ff00"))) "s") t)"##);
    assert_eq!(band(&mut vm, r#"("a " (:propertize "%b" face success) " c")"#), "a probe c");
    let snapshot = vm.frame_snapshot(WIDTH, HEIGHT).unwrap();
    let window = &snapshot.windows[0].snapshot;
    let band = window.mode_line.as_ref().unwrap();
    assert_eq!(band.utf8, "a probe c");
    let green: Vec<_> = band.face_runs.iter()
        .filter(|run| window.faces[run.face.0 as usize].foreground == [0, 0xff, 0])
        .map(|run| &band.utf8[run.bytes.clone()]).collect();
    assert_eq!(green, ["probe"], "only the propertized element carries the face");
}

/// The engine holds no heap `Value` across a collecting `:eval`: the format is
/// an owned `Form`, window and buffer are re-read from rooted slots. With a
/// collection floor of one object, every allocation in `:eval` collects; the
/// band must still be exact, the buffer-local format must survive, and two
/// identical redisplays must leave the same live objects AND bytes.
#[test]
fn eval_that_collects_during_expansion_retains_nothing() {
    let mut vm = probe();
    case(&mut vm, 94, 100, false);
    eval(&mut vm, r#"(progn (make-local-variable 'mode-line-format)
        (setq mode-line-format
          '("%b " (:eval (let ((l nil) (i 0))
                           (while (< i 300) (setq l (cons (make-string 16 ?a) l) i (1+ i)))
                           (format "%d" (length l))))
            " " (:propertize "%p" face mode-line-inactive) " L%l"))
        t)"#);
    vm.set_collection_floor(1);
    let run = |vm: &mut Vm| {
        let before = vm.heap_stats().collections;
        let snapshot = vm.frame_snapshot(WIDTH, HEIGHT).unwrap();
        let text = snapshot.windows[0].snapshot.mode_line.as_ref().unwrap().utf8.clone();
        let during = vm.heap_stats().collections - before;
        vm.collect_garbage();
        let stats = vm.heap_stats();
        (text, during, stats.live_objects, stats.live_bytes)
    };
    let (first, during, objects, bytes) = run(&mut vm);
    assert_eq!(first, "probe 300 44% L100", "{:?}", vm.take_display_diagnostics());
    assert!(during > 0, "the :eval must have collected mid-expansion");
    let (second, _, objects2, bytes2) = run(&mut vm);
    assert_eq!(second, first);
    assert_eq!((objects2, bytes2), (objects, bytes), "a redisplay retained heap objects");
    // Only the buffer-local value was set, so an equal second band also
    // proves the local binding survived both collections; elsewhere the
    // default is still buffer.c's.
    assert_eq!(eval(&mut vm, "(progn (set-buffer (get-buffer-create \"other\")) (list (local-variable-p 'mode-line-format) (equal (default-value 'mode-line-format) \"%-\")))"), "(nil t)");
}

/// GNU clips the band at the window's edge (`xdisp.c:30105`), so a field
/// width larger than the window produces at most a window of text, never an
/// allocation sized by the width. A huge width once aborted the process.
#[test]
fn field_widths_wider_than_the_window_are_clipped() {
    let mut vm = probe();
    case(&mut vm, 1, 3, false);
    for format in [r#"("%100000b")"#, r#"((100000 "x"))"#, r#"("%999999999999999999999999999999b")"#,
                   r#"((2305843009213693951 "x"))"#, r#"("%999999999999999999999999999999b|" "%9999999999999999999l|")"#,
                   r#"((-2305843009213693951 "x") (2305843009213693951 "y") "z")"#] {
        let text = band(&mut vm, format);
        assert!(text.starts_with("probe") || text.starts_with('x'), "{format}: {:?}", &text[..text.len().min(40)]);
        assert!(text.chars().count() <= 4 * 80, "{format}: {} columns built for an 80-column window", text.chars().count());
    }
}

/// GNU keeps `%NNN` widths and integer-construct limits in a C `int`
/// (`xdisp.c:28663`, `:28879`), so values past 2^31 wrap: `2305843009213693951`
/// is -1 (a precision of 1), `-2305843009213693951` is +1. A clamp alone
/// padded `x` to the band's edge and hid the `|` GNU shows. Expected strings
/// are GNU 32.0.50's, `crates/romacs-vm-gui/evidence/tierb/gnu/field-wrap.txt`.
#[test]
fn widths_wrap_like_gnus_c_int() {
    let mut vm = probe();
    case(&mut vm, 1, 3, false);
    for (format, gnu) in [
        (r#"("%4294967297b|")"#, "probe|"),
        (r#"("%2147483648b|")"#, "probe|"),
        (r#"("%9999999999999999999l|")"#, "3|"),
        (r#"((2305843009213693951 "x") "|")"#, "x|"),
        (r#"((-2305843009213693951 "abcdefgh") "|")"#, "abcdefgh|"),
        (r#"((4294967297 "abc") "|")"#, "abc|"),
        (r#"((-4294967295 "abc") "|")"#, "abc|"),
        (r#"((4294967295 "abc") "|")"#, "a|"),
        (r#"((-2305843009213693951 "x") (2305843009213693951 "y") "z")"#, "xyz"),
    ] {
        assert_eq!(band(&mut vm, format), gnu, "{format}");
    }
    // GNU: a width of 1073741823 (the 30 nines, wrapped): `probe` padded far
    // past the window's edge, so the window shows `probe` and blanks only.
    let text = band(&mut vm, r#"("%999999999999999999999999999999b|")"#);
    assert!(text.starts_with("probe") && text[5..].chars().all(|c| c == ' ') && text.len() > 80, "{:?}", &text[..text.len().min(40)]);
}

/// Precision counts a TAB as `tab-width` columns and another control
/// character as 2 (`character.h` `ASCII_CHAR_WIDTH`, via `c_string_width`).
/// GNU 32.0.50 answers from `crates/romacs-vm-gui/evidence/tierb/gnu/control-width.txt`;
/// VM2 paints a control character as `^X`, so where GNU's answer keeps one,
/// the band shows it in that form.
#[test]
fn precision_counts_control_characters_like_gnu() {
    let mut vm = probe();
    case(&mut vm, 1, 3, false);
    for (format, gnu) in [
        ("(\"\" (-3 \"a\tb\") \"|\")", "a|"),
        ("(\"\" (-9 \"a\tb\") \"|\")", "a^I|"),
        ("(\"\" (-10 \"a\tb\") \"|\")", "a^Ib|"),
        ("(\"\" (-3 \"a\u{1}b\") \"|\")", "a^A|"),
        ("(\"\" (-2 \"a\u{1}b\") \"|\")", "a|"),
        // Widths select, characters count: GNU's store_mode_line_string
        // advances N by SCHARS, so a TAB uses one unit of the remaining
        // precision once it has been selected, and padding counts characters.
        ("(\"BEGIN:\" (-10 (\"a\t\" \"bcd\")) \":END\")", "BEGIN:a^Ibcd:END"),
        ("(\"\" (-4 (\"a\t\" \"bcd\")) \"|\")", "abcd|"),
        ("(\"\" (5 \"a\t\") \"|\")", "a^I   |"),
        ("(\"\" (-3 \"日本語x\") \"|\")", "日|"),
        ("(\"\" (4 \"日\") \"|\")", "日   |"),
        ("(\"\" (-4 (\"日本\" \"xyz\")) \"|\")", "日本xy|"),
        ("(\"\" (-3 (\"%b\")) \"|\")", "pro|"),
        ("(\"\" (8 \"%b\") \"|\")", "probe   |"),
    ] {
        assert_eq!(band(&mut vm, format), gnu, "{format:?}");
    }
}

/// The head of GNU's `standard-mode-line-format` (bindings.el:819), the format
/// GNU's own Lisp installs once loadup gets past bindings.el's `propertize`
/// forms: `%e` first, then construct variables, here still void. VM2's old
/// mode-line expander refused `%e` ("unsupported VM2 mode-line percent
/// escape") and with it the whole frame. GNU (`decode_mode_spec`, `case 'e'`
/// at 45b0f7699df): always "" on a SYSTEM_MALLOC build such as glibc's, else
/// "!MEM FULL! " while `memory-full` is non-nil; VM2 is always "" (DIVERGENCES.md VM2-G1B).
/// Oracle: `evidence/tierb/gnu/standard-head.txt` (GNU 32.0.50, tty, -Q).
#[test]
fn gnus_standard_mode_line_head_formats_as_gnu() {
    let mut vm = probe();
    for (format, gnu) in [
        ("(\"%e\" \"%b\")", "probe"),
        ("(\"%e\" romacs-void-front-space romacs-void-mule-info \"%b\" \"   \" (romacs-void-project romacs-void-project-format) \"  \" \"|\")", "probe     |"),
        ("\"%3e|\"", "   |"),
    ] {
        assert_eq!(band(&mut vm, format), gnu, "{format}");
    }
    // GNU on a SYSTEM_MALLOC build (glibc, as the oracle): still "" while
    // `memory-full' is non-nil; only a gmalloc build shows "!MEM FULL! ".
    eval(&mut vm, "(setq memory-full t)");
    assert_eq!(band(&mut vm, "\"%e|\""), "|");
}

/// Pinned GNU 45b0f7699df, gtb-fix/gnu.txt: <Foo|/tmp/foo>;
/// %Z keeps EOL after the undecided coding's "-".
#[test]
fn unibyte_percent_strings_are_preserved() {
    let mut vm = probe();
    eval(&mut vm, r#"(setq mode-name (concat "Fo" "o") buffer-file-name (concat "/tmp/" "foo") eol-mnemonic-undecided (concat "E" "OL") buffer-file-coding-system nil)"#);
    for (format, expected) in [(r#"("<%m>")"#, "<Foo>"), (r#"("<%f>")"#, "</tmp/foo>"), (r#"("<%Z>")"#, "<-EOL>")] {
        assert_eq!(band(&mut vm, format), expected, "{format}");
    }
}

/// decode_mode_spec_coding (xdisp.c:29346): %z is the buffer's coding
/// mnemonic, %Z adds its EOL type's. Pinned GNU 45b0f7699df (pgtk) answers
/// these, and "<U|U:>" for utf-8-unix, "<U|U(DOS)>" for utf-8-dos.
#[test]
fn coding_mnemonics_follow_the_buffer_coding_system() {
    let mut vm = probe();
    for (coding, expected) in [("no-conversion", "<=|=:>"), ("undecided", "<-|-:>"), ("nil", "<-|-:>")] {
        eval(&mut vm, &format!("(setq buffer-file-coding-system '{coding})"));
        assert_eq!(band(&mut vm, r#"("<%z|%Z>")"#), expected, "{coding}");
    }
}

fn two_window_eval_probe() -> Vm {
    let mut vm = probe();
    eval(&mut vm, r#"(setq real-window (selected-window))
        (set-window-new-pixel (selected-window) 240)
        (setq lower (split-window-internal (selected-window) 220 nil 0.5))
        (set-window-buffer lower (get-buffer-create "other"))"#);
    vm
}

fn same_buffer_point_probe() -> Vm {
    let mut vm = probe();
    eval(&mut vm, r#"(goto-char 1)
        (setq real-window (selected-window))
        (set-window-new-pixel (selected-window) 240)
        (setq lower (split-window-internal (selected-window) 220 nil 0.5))
        (set-window-point lower 9)"#);
    vm
}

/// Pinned GNU 45b0f7699df redisplay (`crates/romacs-vm-gui/evidence/tierb/gnu/point-context.txt`):
/// :seen ("9/9" "1/1"), :restored (t 1 1 9).
#[test]
fn same_buffer_eval_point_is_window_local() {
    let mut vm = same_buffer_point_probe();
    eval(&mut vm, r#"(setq mode-line-format '((:eval (format "%d/%d" (point) (window-point)))))"#);
    assert_eq!(all_bands(&mut vm), ["1/1", "9/9"], "GNU per-window point during :eval");
    assert_eq!(eval(&mut vm, "(list (eq real-window (selected-window)) (point) (window-point) (window-point lower))"), "(t 1 1 9)");
}

#[test]
fn same_buffer_eval_uses_point_after_forced_start_visibility() {
    let mut vm = same_buffer_point_probe();
    eval(&mut vm, r#"(set-window-start lower 200)
        (setq mode-line-format '((:eval (format "%d/%d" (point) (window-point)))))"#);
    let bands = all_bands(&mut vm);
    let point = eval(&mut vm, "(window-point lower)");
    assert_ne!(point, "9", "forced start must move the non-selected window's point");
    assert_eq!(bands, ["1/1".to_owned(), format!("{point}/{point}")]);
    assert_eq!(eval(&mut vm, "(point)"), "1", "selected buffer point must survive visibility and formatting");
}

#[test]
fn same_buffer_eval_point_is_restored_after_non_local_exits_and_collection() {
    for exit in ["\"OK\"", "(error \"point probe\")", "(throw 'missing 1)", "(signal 'quit nil)"] {
        let mut vm = same_buffer_point_probe();
        eval(&mut vm, &format!(r#"(setq seen nil mode-line-format
            '((:eval (progn (make-string 100 ?x)
                (setq seen (cons (list (point) (window-point)) seen)) {exit}))))"#));
        vm.set_collection_floor(1);
        let before = vm.heap_stats().collections;
        let bands = all_bands(&mut vm);
        assert_eq!(bands, if exit == "\"OK\"" { ["OK", "OK"] } else { ["", ""] }, "{exit}");
        assert_eq!(eval(&mut vm, "seen"), "((9 9) (1 1))", "{exit}: evaluated points");
        assert!(vm.heap_stats().collections > before, "{exit}: must collect during :eval");
        assert_eq!(eval(&mut vm, "(list (eq real-window (selected-window)) (point) (window-point) (window-point lower))"), "(t 1 1 9)", "{exit}: restored points");
        assert_eq!(vm.take_display_diagnostics().len(), if exit == "\"OK\"" { 0 } else { 2 });
    }
}

fn all_bands(vm: &mut Vm) -> Vec<String> {
    vm.frame_snapshot(WIDTH, HEIGHT).unwrap().windows.iter()
        .filter_map(|w| w.snapshot.mode_line.as_ref().map(|b| b.utf8.clone())).collect()
}

/// GNU display_mode_lines temporarily selects W without select-window's point
/// side effects (xdisp.c:28114-28156); gtb-fix/gnu.txt records these answers.
#[test]
fn eval_window_context_is_local_and_restored() {
    let mut vm = two_window_eval_probe();
    eval(&mut vm, r#"(set-default 'mode-line-format '((:eval (if (eq (window-buffer (selected-window)) (current-buffer)) "SELF" "OTHER"))))"#);
    assert_eq!(all_bands(&mut vm), ["SELF", "SELF"]);
    assert_eq!(eval(&mut vm, "(eq real-window (selected-window))"), "t");
}

#[test]
fn eval_window_context_unwinds_after_errors() {
    let mut vm = two_window_eval_probe();
    eval(&mut vm, r#"(setq seen nil) (set-default 'mode-line-format
        '((:eval (progn (setq seen (cons (eq (window-buffer (selected-window)) (current-buffer)) seen)) (error "context probe")))))"#);
    assert_eq!(all_bands(&mut vm), ["", ""]);
    assert_eq!(eval(&mut vm, "seen"), "(t t)", "both erroring evaluations see their own window");
    assert_eq!(eval(&mut vm, "(eq real-window (selected-window))"), "t");
    assert_eq!(vm.take_display_diagnostics().len(), 2);
}

#[test]
fn eval_window_context_survives_collection() {
    let mut vm = two_window_eval_probe();
    eval(&mut vm, r#"(set-default 'mode-line-format '((:eval (progn (make-string 100 ?x) (if (eq (window-buffer (selected-window)) (current-buffer)) "SELF" "OTHER")))))"#);
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let actual = all_bands(&mut vm);
    assert_eq!(actual, ["SELF", "SELF"], "{:?}", vm.take_display_diagnostics());
    assert!(vm.heap_stats().collections > before);
    assert_eq!(eval(&mut vm, "(eq real-window (selected-window))"), "t");
    assert_eq!(all_bands(&mut vm), ["SELF", "SELF"], "rooted selection survives repeated collecting evaluation");
}

#[test]
fn reviewed_exact_reproductions() {
    let mut vm = probe();
    for (source, gnu) in [
        (r#"(setq fn (lambda () "OK")) (setq mode-line-format (list "<" (list :eval (list 'funcall fn)) ">"))"#, "<OK>"),
        (r#"(setq sym (make-symbol "ml-uninterned")) (set sym "GEN") (setq mode-line-format (list "<" sym ">"))"#, "<GEN>"),
        (r#"(setq mode-line-format (list "<" (list sym "YES" "NO") ">"))"#, "<YES>"),
        (r#"(setq mode-line-format (list "<" "a\0b" ">"))"#, "<a>"),
    ] {
        vm.eval_source(source).unwrap();
        let ours = current_band(&mut vm);
        println!("review reproduction: ours={ours:?} GNU={gnu:?}");
        assert_eq!(ours, gnu, "{source}");
    }
}

#[test]
fn reviewed_eval_object_constants_survive_collection() {
    let mut vm = probe();
    eval(&mut vm, r#"(setq fn (lambda () "OK")) (put 'reviewed-format 'risky-local-variable t)"#);
    eval(&mut vm, r#"(setq reviewed-format (list "<" (list :eval (list 'funcall fn)) ">")) t"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<OK>", "GNU evaluates original function constants");
    eval(&mut vm, r#"(setq reviewed-format (list "<" '(:eval (progn (garbage-collect) "")) (list :eval (list 'funcall fn)) ">")) t"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<OK>", "executable constants survive earlier :eval GC");
}

#[test]
fn reviewed_generated_symbols_keep_identity_and_risk() {
    let mut vm = probe();
    eval(&mut vm, r#"(setq sym (make-symbol "ml-uninterned")) (set sym "GEN") (put 'reviewed-format 'risky-local-variable t)"#);
    eval(&mut vm, r#"(setq reviewed-format (list "<" sym ">"))"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<GEN>", "GNU expands uninterned symbols by identity");
    eval(&mut vm, r#"(setq reviewed-format (list "<" (list sym "YES" "NO") ">"))"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<YES>", "GNU tests uninterned symbol values");
    eval(&mut vm, "(set sym nil)");
    assert_eq!(band(&mut vm, "reviewed-format"), "<NO>");
    eval(&mut vm, r#"(set sym '(:eval "SAFE")) (setq reviewed-format (list "<" sym ">"))"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<>", "unmarked generated symbols are risky");
    eval(&mut vm, "(put sym 'risky-local-variable t)");
    assert_eq!(band(&mut vm, "reviewed-format"), "<SAFE>", "risk property belongs to generated symbol");
}

/// Two `(:eval SYMBOL)' forms whose uninterned symbols share a name are two
/// forms: each evaluates its own symbol. GNU shows "<one|two>"; a cache
/// keyed by spelling alone ran the first symbol's lookup for both.
#[test]
fn reviewed_eval_of_distinct_uninterned_symbols_keeps_each_value() {
    let mut vm = probe();
    eval(&mut vm, r#"(setq a (make-symbol "ml-same") b (make-symbol "ml-same")) (set a "one") (set b "two")
                     (put 'reviewed-format 'risky-local-variable t)
                     (setq reviewed-format (list "<" (list :eval a) "|" (list :eval b) ">"))"#);
    assert_eq!(band(&mut vm, "reviewed-format"), "<one|two>");
    eval(&mut vm, r#"(set b "three")"#);
    assert_eq!(current_band(&mut vm), "<one|three>");
}

#[test]
fn reviewed_nul_terminates_only_nonliteral_strings() {
    let mut vm = probe();
    assert_eq!(band(&mut vm, r#"("<" "a\0b" ">")"#), "<a>", "GNU stops nonliteral scanning at NUL");
    eval(&mut vm, r#"(setq ml-nul "a\0b")"#);
    assert_eq!(band(&mut vm, r#"("<" ml-nul ">")"#), "<a^@b>", "GNU preserves literal symbol strings");
}

/// bindings.el's `mode-line-mule-info` and `mode-line-modes` hold keymaps with
/// command closures inside `:propertize` plists. A function object has no
/// source spelling, and the expander used to turn the WHOLE variable into
/// `*invalid*`; GNU ignores what it does not display and shows `*invalid*`
/// only for an object that is itself an element (`display_mode_element`'s
/// `invalid:` label). Oracle: `evidence/tierb/gnu/object-values.txt`.
#[test]
fn function_objects_inside_constructs_do_not_invalidate_them() {
    let mut vm = probe();
    eval(&mut vm, r#"(progn
        (setq ml-probe-propertized
              (list (list :propertize "%b" 'local-map
                          (list 'keymap (cons 'down-mouse-1 (lambda () (interactive) nil)))
                          'help-echo (lambda (&rest _) "help"))))
        (put 'ml-probe-propertized 'risky-local-variable t)
        (setq ml-probe-object (list "<" (lambda () 1) ">"))
        (put 'ml-probe-object 'risky-local-variable t) t)"#);
    for (format, gnu) in [
        ("(\"\" ml-probe-propertized \"|\")", "probe|"),
        ("(\"\" ml-probe-object \"|\")", "<*invalid*>|"),
    ] {
        assert_eq!(band(&mut vm, format), gnu, "{format}");
    }
}
