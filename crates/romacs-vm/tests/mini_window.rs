//! The mini-window's display (display.rs, mini_window.rs): its buffer with
//! the overlay strings xdisp.c delivers there, and the height
//! resize_mini_window gives it. An 800x400 frame of 20-pixel lines: the
//! default `max-mini-window-height' 0.25 allows five lines.
use romacs_vm::Vm;

const LINE: u32 = 20;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 400, LINE, 10).unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// Show TEXT in the mini-window's buffer, with point at its end.
fn show(vm: &mut Vm, text: &str) {
    eval(vm, &format!("(let ((old (current-buffer)))
        (set-buffer (window-buffer (minibuffer-window))) (erase-buffer) (insert {}) (set-buffer old))",
        serde_json::to_string(text).unwrap()));
}

/// The mini-window's (text, height, point byte, start byte) and the root's height.
fn mini(vm: &mut Vm) -> (String, u32, usize, usize, u32) {
    let frame = vm.frame_snapshot(800, 400).unwrap();
    let mini = &frame.windows.last().unwrap().snapshot;
    let root: u32 = frame.windows[..frame.windows.len() - 1].iter().map(|w| w.snapshot.height).max().unwrap();
    (mini.text.utf8.clone(), mini.height, mini.point_byte, mini.start_byte, root)
}

#[test]
fn echo_area_fits_its_lines_within_the_maximum() {
    let mut vm = vm();
    eval(&mut vm, "(setq resize-mini-windows 'grow-only)");
    show(&mut vm, "a\nb\nc");
    let (_, height, _, start, root) = mini(&mut vm);
    assert_eq!((height, start, root), (3 * LINE, 0, 400 - 3 * LINE), "three lines, the root shrinks");
    // Nine lines take the five allowed; ad hoc scrolling shows the end.
    show(&mut vm, "1\n2\n3\n4\n5\n6\n7\n8\n9");
    let (text, height, _, start, _) = mini(&mut vm);
    assert_eq!((height, &text[start..]), (5 * LINE, "5\n6\n7\n8\n9"));
    // The echo area fits exactly: one line again, from the beginning.
    show(&mut vm, "x");
    let (_, height, _, start, _) = mini(&mut vm);
    assert_eq!((height, start), (LINE, 0));
    // An integer maximum counts lines.
    eval(&mut vm, "(setq max-mini-window-height 2)");
    show(&mut vm, "a\nb\nc");
    assert_eq!(mini(&mut vm).1, 2 * LINE);
}

#[test]
fn nil_never_resizes_and_t_always_fits() {
    let mut vm = vm();
    eval(&mut vm, "(setq resize-mini-windows nil)");
    show(&mut vm, "a\nb\nc");
    assert_eq!(mini(&mut vm).1, LINE);
    eval(&mut vm, "(setq resize-mini-windows t)");
    assert_eq!(mini(&mut vm).1, 3 * LINE);
    show(&mut vm, "a");
    assert_eq!(mini(&mut vm).1, LINE);
}

#[test]
fn an_active_minibuffer_only_grows_until_it_empties() {
    let mut vm = vm();
    // minibuf.c minibuf_level, as the Lisp half of read-from-minibuffer keeps it.
    eval(&mut vm, "(setq resize-mini-windows 'grow-only romacs--minibuf-level 1)");
    show(&mut vm, "a\nb\nc");
    assert_eq!(mini(&mut vm).1, 3 * LINE);
    show(&mut vm, "a");
    assert_eq!(mini(&mut vm).1, 3 * LINE, "grow-only keeps the height");
    show(&mut vm, "");
    assert_eq!(mini(&mut vm).1, LINE, "an empty minibuffer shrinks");
}

#[test]
fn overlay_strings_show_with_their_faces_and_cursor() {
    let mut vm = vm();
    eval(&mut vm, "(setq resize-mini-windows 'grow-only)");
    show(&mut vm, "M-x fi");
    // icomplete's shape: before-string at the start, after-string at point
    // whose first character asks for the cursor, a face list on a candidate.
    eval(&mut vm, "(let ((old (current-buffer)))
        (set-buffer (window-buffer (minibuffer-window)))
        (let ((o (make-overlay (point-min) (point-max))) (s (concat \" \" \"\\nfind-file\\nfill\")))
          (put-text-property 0 1 'cursor t s)
          (put-text-property 2 11 'face '(mode-line default) s)
          (overlay-put o 'before-string \"1/2 \")
          (overlay-put o 'after-string s))
        (goto-char (point-max)) (set-window-point (minibuffer-window) (point)) (set-buffer old))");
    let (text, height, point, start, _) = mini(&mut vm);
    assert_eq!(text, "1/2 M-x fi \nfind-file\nfill");
    assert_eq!((height, start), (3 * LINE, 0));
    assert_eq!(point, "1/2 M-x fi".len(), "the cursor property puts point before the string");
    let frame = vm.frame_snapshot(800, 400).unwrap();
    let snapshot = &frame.windows.last().unwrap().snapshot;
    let face_at = |byte: usize| snapshot.face_runs_face(byte);
    assert_ne!(face_at(text.find("find-file").unwrap()), 0, "the face list merges to a face of its own");
    assert_eq!(face_at(text.find("fill").unwrap()), 0);
    // A layout's end, a display byte inside the after-string, is the
    // string's buffer position: the window shows its buffer to the end.
    let id = snapshot.id;
    vm.display_apply_layout(&[romacs_vm::WindowEnd { id, end: text.find("fill").unwrap(), visible_end: 0, body_height: 0.0, rows: vec![] }]).unwrap();
    assert_eq!(eval(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window))
        (list (window-end (minibuffer-window)) (point-max)))"), "(7 7)");
    // Without a cursor property point follows the string.
    eval(&mut vm, "(let ((old (current-buffer)))
        (set-buffer (window-buffer (minibuffer-window)))
        (let ((o (car (overlays-in (point-min) (point-max)))))
          (overlay-put o 'after-string \"[x]\") (overlay-put o 'before-string nil))
        (set-buffer old))");
    let (text, _, point, _, _) = mini(&mut vm);
    assert_eq!((text.as_str(), point), ("M-x fi[x]", "M-x fi[x]".len()));
}

trait FaceAt {
    fn face_runs_face(&self, byte: usize) -> u32;
}
impl FaceAt for romacs_layout_input::WindowSnapshot {
    fn face_runs_face(&self, byte: usize) -> u32 {
        self.text.face_runs.iter().find(|r| r.bytes.contains(&byte)).map_or(0, |r| r.face.0)
    }
}

/// xdisp.c resize_mini_window counts display lines: a line wider than a
/// row continues on the next one, as a long prompt does. Without fringes a
/// row of the 80-column window holds 79 columns, its last showing the
/// continuation glyph. Ad hoc scrolling then starts at the text line from
/// which the rest fits.
#[test]
fn a_line_wider_than_the_window_takes_its_continuation_lines() {
    let mut vm = vm();
    eval(&mut vm, "(setq resize-mini-windows 'grow-only)");
    show(&mut vm, &"x".repeat(150));
    assert_eq!(mini(&mut vm).1, 2 * LINE, "150 columns in rows of 79 take two lines");
    show(&mut vm, "");
    show(&mut vm, &"y".repeat(79));
    assert_eq!(mini(&mut vm).1, LINE, "a row's width is one line");
    show(&mut vm, &"y".repeat(80));
    assert_eq!(mini(&mut vm).1, 2 * LINE, "the window's width continues");
    let long = format!("a\nb\n{}\nc", "z".repeat(170));
    show(&mut vm, &long);
    let (text, height, _, start, _) = mini(&mut vm);
    assert_eq!((height, &text[start..]), (5 * LINE, &long[2..]), "b, the three-line text line and c fit");
}

/// read_minibuf_unwind (minibuf.c:1197): leaving the outermost minibuffer
/// resizes the mini-window at once, so the command's next prompt grows
/// from one line rather than keeping the height the last prompt grew to.
#[test]
fn leaving_the_minibuffer_shrinks_the_mini_window_at_once() {
    let mut vm = vm();
    eval(&mut vm, "(setq resize-mini-windows 'grow-only romacs--minibuf-level 1)");
    show(&mut vm, "a\nb\nc");
    assert_eq!(mini(&mut vm).1, 3 * LINE);
    show(&mut vm, "");
    let line = LINE.to_string();
    assert_eq!(eval(&mut vm, "(setq romacs--minibuf-level 0) (list (romacs--resize-mini-window) (window-pixel-height (minibuffer-window)) (romacs--resize-mini-window))"),
               format!("(t {line} nil)"));
}
