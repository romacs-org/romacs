use romacs_vm::{TerminalHost, TerminalSnapshot, Vm};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Host(Rc<RefCell<State>>);
struct State { snapshot: TerminalSnapshot, sent: Vec<Vec<u8>>, closed: Vec<u64>, snapshot_reads: usize, pending: bool, resized: Vec<(u16, u16)> }
impl TerminalHost for Host {
    fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64, String> { Ok(1) }
    fn send(&mut self, _: u64, bytes: &[u8]) -> Result<(), String> { self.0.borrow_mut().sent.push(bytes.to_vec()); Ok(()) }
    fn resize(&mut self, _: u64, columns: u16, rows: u16) -> Result<(), String> { self.0.borrow_mut().resized.push((columns, rows)); Ok(()) }
    fn input_state(&mut self, _: u64) -> Result<romacs_vm::TerminalInputState, String> { Ok(self.0.borrow().snapshot.input_state()) }
    fn changed_sessions(&mut self) -> Vec<u64> { if std::mem::take(&mut self.0.borrow_mut().pending) { vec![1] } else { vec![] } }
    fn snapshot(&mut self, _: u64) -> Result<TerminalSnapshot, String> { let mut state = self.0.borrow_mut(); state.snapshot_reads += 1; Ok(state.snapshot.clone()) }
    fn close(&mut self, id: u64) -> Result<(), String> { self.0.borrow_mut().closed.push(id); Ok(()) }
}

fn setup() -> (Vm, Rc<RefCell<State>>) {
    let state = Rc::new(RefCell::new(State {
        snapshot: TerminalSnapshot { revision: 1, text: "one\nλtwo".into(), styles: std::sync::Arc::from([]), default_colors: [None; 2], cursor_byte: 9, screen_start_byte: 0, columns: 80, rows: 24,
            alternate_screen: false, running: true, application_cursor: false, bracketed_paste: false }, sent: vec![], closed: vec![], snapshot_reads: 0, pending: false, resized: vec![],
    }));
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_terminal_host(Some(Box::new(Host(state.clone())))).unwrap();
    vm.eval_source("(setq terminal-buffer (get-buffer-create \"terminal-test\")) (set-buffer terminal-buffer) (setq terminal-id (romacs--term-create \"/bin/sh\" \"/\" 80 24)) (setq buffer-read-only t) (set-window-buffer (selected-window) terminal-buffer)").unwrap();
    (vm, state)
}

#[test]
fn output_updates_only_owned_buffer_and_survives_moving_gc() {
    let (mut vm, state) = setup();
    vm.collect_garbage();
    assert!(vm.poll_terminal_output().unwrap(), "TERMINAL_PUBLICATION_APPLIED");
    assert_eq!(vm.eval_source("(list (buffer-string) (point) (buffer-modified-p))").unwrap().display().unwrap(), "(\"one\\nλtwo\" 9 nil)");
    vm.eval_source("(romacs--term-follow terminal-id nil) (goto-char 2) (setq terminal-mark (copy-marker 3)) (setq another (get-buffer-create \"another\")) (set-buffer another) (insert \"untouched\")").unwrap();
    state.borrow_mut().snapshot.text = "one\nλtwo\nTAIL".into();
    state.borrow_mut().snapshot.revision += 1;
    state.borrow_mut().pending = true;
    vm.collect_garbage();
    assert!(vm.poll_terminal_output().unwrap());
    let got = vm.eval_source("(list (eq (current-buffer) another) (buffer-string) (with-current-buffer terminal-buffer (list (buffer-string) (point) (marker-position terminal-mark) buffer-read-only (buffer-modified-p))))").unwrap().display().unwrap();
    assert_eq!(got, "(t \"untouched\" (\"one\\nλtwo\\nTAIL\" 2 3 t nil))", "DETACHED_OUTPUT_PRESERVES_POINT_MARK_CURRENT_BUFFER");
    assert!(state.borrow().sent.is_empty(), "OBSERVATION_NEVER_SENDS_PTY_BYTES");
}

#[test]
fn exit_preserves_navigation_and_sending_after_exit_is_typed() {
    let (mut vm, state) = setup();
    vm.poll_terminal_output().unwrap();
    vm.eval_source("(romacs--term-follow terminal-id nil) (goto-char 2)").unwrap();
    { let mut s = state.borrow_mut(); s.snapshot.running = false; s.snapshot.revision += 1; s.pending = true; s.snapshot.text = "one\nλtwo\nFINAL".into(); }
    vm.poll_terminal_output().unwrap();
    let got = vm.eval_source("(list (point) (buffer-string) buffer-read-only (condition-case e (romacs--term-send terminal-id \"x\") (romacs-terminal-not-running (car e))))").unwrap().display().unwrap();
    assert_eq!(got, "(2 \"one\\nλtwo\\nFINAL\" t romacs-terminal-not-running)", "EXIT_MUST_NOT_MOVE_POINT_OR_LOSE_FINAL_OUTPUT");
    assert!(state.borrow().sent.is_empty());
    vm.eval_source("(romacs--term-close terminal-id)").unwrap();
    assert_eq!(state.borrow().closed, vec![1]);
}

#[test]
fn terminal_output_is_applied_while_a_real_minibuffer_waits() {
    use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput};
    struct Keyboard { state: Rc<RefCell<State>>, reads: usize, seen: Rc<RefCell<Vec<String>>> }
    impl HostInput for Keyboard {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
            self.seen.borrow_mut().push(frame.frame.windows[0].snapshot.text.utf8.clone()); Ok(vec![])
        }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
            self.reads += 1;
            match self.reads {
                1 => { let mut state=self.state.borrow_mut(); state.snapshot.text = "one\nλtwo\nDURING_PROMPT".into(); state.snapshot.revision += 1; state.pending = true; Ok(HostEvent::Background) }
                2 => Ok(HostEvent::Key(CommandEvent::Character('z' as i64))),
                3 => Ok(HostEvent::Key(CommandEvent::Character(13))),
                _ => Err("Keyboard exhausted".into()),
            }
        }
    }
    let (mut vm,state)=setup(); vm.poll_terminal_output().unwrap();
    vm.eval_source("(romacs--term-follow terminal-id nil) (set-window-buffer (selected-window) terminal-buffer) (condition-case nil (read-string \"\") (error nil))").unwrap();
    let seen=Rc::new(RefCell::new(vec![]));
    vm.set_host_input(Some(Box::new(Keyboard {state,reads:0,seen:seen.clone()})));
    assert_eq!(vm.eval_source("(read-string \"M-x \")").unwrap().display().unwrap(), "\"z\"");
    assert!(seen.borrow().iter().any(|s|s.contains("DURING_PROMPT")), "BACKGROUND_OUTPUT_MUST_PAINT_DURING_PROMPT");
}

#[test]
fn input_and_status_do_not_request_history_publications() {
    let (mut vm, state) = setup();
    vm.eval_source("(list (romacs--term-running-p terminal-id) (romacs--term-send terminal-id \"bytes\"))").unwrap();
    assert_eq!(state.borrow().sent, vec![b"bytes".to_vec()]);
    assert_eq!(state.borrow().snapshot_reads, 0, "TERMINAL_INPUT_AND_STATUS_MUST_NOT_MATERIALIZE_HISTORY");
}

#[test]
fn hidden_output_updates_its_buffer_without_requesting_frame_repaint() {
    let (mut vm, state) = setup();
    vm.eval_source("(setq ordinary (get-buffer-create \"ordinary\")) (set-window-buffer (selected-window) ordinary) (set-buffer ordinary) (insert \"ordinary text\") (goto-char 4)").unwrap();
    assert!(!vm.poll_terminal_output().unwrap(), "HIDDEN_TERMINAL_MUST_NOT_INVALIDATE_FRAME");
    assert_eq!(vm.eval_source("(list (buffer-string) (point) (with-current-buffer terminal-buffer (buffer-string)))").unwrap().display().unwrap(), "(\"ordinary text\" 4 \"one\\nλtwo\")");
    state.borrow_mut().snapshot.text = "one\nλtwo\nHIDDEN".into();
    state.borrow_mut().snapshot.revision += 1;
    state.borrow_mut().pending = true;
    assert!(!vm.poll_terminal_output().unwrap(), "HIDDEN_OUTPUT_MUST_NOT_REPAINT_ORDINARY_FILE");
    vm.eval_source("(set-window-buffer (selected-window) terminal-buffer)").unwrap();
    state.borrow_mut().snapshot.revision += 1;
    state.borrow_mut().pending = true;
    assert!(vm.poll_terminal_output().unwrap(), "VISIBLE_TERMINAL_MUST_REQUEST_PAINT");
}

#[test]
fn terminal_sessions_keep_independent_buffers_and_close_ownership() {
    struct Sessions {
        values: Rc<RefCell<Vec<TerminalSnapshot>>>,
        closed: Rc<RefCell<Vec<u64>>>,
        next: u64,
        pending: Rc<RefCell<Vec<u64>>>,
        reads: Rc<RefCell<Vec<u64>>>,
    }
    impl TerminalHost for Sessions {
        fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64, String> { self.next += 1; Ok(self.next) }
        fn send(&mut self, _: u64, _: &[u8]) -> Result<(), String> { Ok(()) }
        fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(), String> { Ok(()) }
        fn input_state(&mut self, id: u64) -> Result<romacs_vm::TerminalInputState, String> { Ok(self.values.borrow()[id as usize - 1].input_state()) }
        fn changed_sessions(&mut self) -> Vec<u64> { std::mem::take(&mut *self.pending.borrow_mut()) }
        fn snapshot(&mut self, id: u64) -> Result<TerminalSnapshot, String> { self.reads.borrow_mut().push(id); Ok(self.values.borrow()[id as usize - 1].clone()) }
        fn close(&mut self, id: u64) -> Result<(), String> { self.closed.borrow_mut().push(id); Ok(()) }
    }
    let (_, first) = setup();
    let mut a = first.borrow().snapshot.clone();
    a.text = "A\n".into(); a.cursor_byte = 2;
    let mut b = a.clone(); b.text = "B\n".into();
    let values = Rc::new(RefCell::new(vec![a, b]));
    let closed = Rc::new(RefCell::new(vec![]));
    let pending = Rc::new(RefCell::new(vec![]));
    let reads = Rc::new(RefCell::new(vec![]));
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_terminal_host(Some(Box::new(Sessions { values: values.clone(), closed: closed.clone(), next: 0, pending: pending.clone(), reads: reads.clone() }))).unwrap();
    vm.eval_source(r#"
      (setq a (get-buffer-create "term-a") b (get-buffer-create "term-b") file (get-buffer-create "file"))
      (set-buffer a) (setq a-id (romacs--term-create "/bin/sh" "/" 80 24))
      (set-buffer b) (setq b-id (romacs--term-create "/bin/sh" "/" 80 24))
      (set-window-buffer (selected-window) file) (set-buffer file) (insert "unsaved file") (goto-char 5)
    "#).unwrap();
    vm.collect_garbage();
    assert!(!vm.poll_terminal_output().unwrap());
    reads.borrow_mut().clear();
    { let mut output = values.borrow_mut(); output[0].text = "A\nonly-a".into(); output[0].revision += 1; }
    pending.borrow_mut().push(1);
    assert!(!vm.poll_terminal_output().unwrap());
    assert_eq!(*reads.borrow(), vec![1], "TERMINAL_WAKE_MUST_QUERY_ONLY_ITS_OWNER");
    assert_eq!(vm.eval_source("(with-current-buffer b (buffer-string))").unwrap().display().unwrap(), "\"B\\n\"");
    // B shows in a window of its own that its user moved away from.
    vm.eval_source("(setq b-window (let* ((w (selected-window)) (h (window-pixel-height w))) (set-window-new-pixel w (- h 200)) (split-window-internal w 200 nil 0.5))) (set-window-buffer b-window b) (romacs--term-follow b-id nil b-window) (set-window-point b-window 1)").unwrap();
    { let mut output = values.borrow_mut(); output[0].text = "A\nfirst".into(); output[0].revision += 1; output[1].text = "B\nsecond".into(); output[1].revision += 1; }
    pending.borrow_mut().extend([1, 2]);
    assert!(vm.poll_terminal_output().unwrap(), "B IS DISPLAYED");
    let got = vm.eval_source("(list (eq (current-buffer) file) (buffer-string) (point) (buffer-modified-p) (with-current-buffer a (buffer-string)) (with-current-buffer b (buffer-string)) (window-point b-window))").unwrap().display().unwrap();
    assert_eq!(got, "(t \"unsaved file\" 5 t \"A\\nfirst\" \"B\\nsecond\" 1)", "TWO_TERMINALS_MUST_NOT_CHANGE_ORDINARY_BUFFER_OR_EACH_OTHER");
    vm.eval_source("(kill-buffer a)").unwrap();
    assert!(!vm.poll_terminal_output().unwrap());
    assert_eq!(*closed.borrow(), vec![1], "KILLING_A_TERMINAL_CLOSES_ONLY_ITS_SESSION");
    assert_eq!(vm.eval_source("(with-current-buffer b (buffer-string))").unwrap().display().unwrap(), "\"B\\nsecond\"");
}

#[test]
fn unchanged_sessions_are_not_queried_on_editor_input_or_idle() {
    let (mut vm, state) = setup();
    vm.poll_terminal_output().unwrap();
    state.borrow_mut().snapshot_reads = 0;
    for _ in 0..100 { assert!(!vm.poll_terminal_output().unwrap()); }
    assert_eq!(state.borrow().snapshot_reads, 0, "IDLE_EDITOR_MUST_NOT_QUERY_TERMINAL_PUBLICATIONS");
    // A view change applies the last publication at once; it queries nothing.
    vm.eval_source("(romacs--term-follow terminal-id nil) (romacs--term-follow terminal-id t)").unwrap();
    vm.poll_terminal_output().unwrap();
    assert_eq!(state.borrow().snapshot_reads, 0, "A_VIEW_CHANGE_QUERIES_NO_PUBLICATION");
}

#[test]
fn late_native_notifications_are_drained_after_the_last_terminal_closes() {
    let (mut vm, state) = setup();
    vm.poll_terminal_output().unwrap();
    vm.eval_source("(romacs--term-close terminal-id)").unwrap();
    assert!(!vm.has_terminals());
    state.borrow_mut().pending = true;
    assert!(!vm.poll_terminal_output().unwrap());
    assert!(!state.borrow().pending, "CLOSED_TERMINAL_NOTIFICATIONS_MUST_BE_DRAINED");
    assert_eq!(state.borrow().snapshot_reads, 1, "CLOSED_OWNER_MUST_NOT_REQUEST_A_SNAPSHOT");
    assert!(!Vm::new().poll_terminal_output().unwrap(), "NO_HOST_REMAINS_A_NOOP");
}

#[test]
fn terminal_styles_are_buffer_owned_and_invalid_ranges_do_not_mutate_the_buffer() {
    use romacs_vm::{TerminalColor, TerminalStyle, TerminalStyleRun};
    let (mut vm, state) = setup();
    let style = TerminalStyle { foreground: TerminalColor::Rgb([12, 34, 56]), background: TerminalColor::Rgb([65, 43, 21]), inverse: false, hidden: false };
    state.borrow_mut().snapshot.styles = vec![TerminalStyleRun { bytes: 0..3, style }].into();
    vm.poll_terminal_output().unwrap();
    vm.collect_garbage();
    let terminal = vm.frame_snapshot(800, 600).unwrap().windows.remove(0).snapshot;
    let run = terminal.text.face_runs.iter().find(|r| r.bytes.contains(&0)).unwrap();
    assert_eq!(terminal.faces[run.face.0 as usize].foreground, [12, 34, 56]);
    assert_eq!(terminal.faces[run.face.0 as usize].background, [65, 43, 21]);
    vm.eval_source("(setq normal (get-buffer-create \"normal-colors\")) (set-window-buffer (selected-window) normal) (set-buffer normal) (insert \"ordinary\") (goto-char 4)").unwrap();
    let ordinary = vm.frame_snapshot(800, 600).unwrap().windows.remove(0).snapshot;
    assert_eq!(ordinary.faces[0].foreground, [255, 255, 255], "TERMINAL_COLORS_MUST_NOT_CHANGE_OTHER_BUFFERS");
    assert_eq!(ordinary.faces.len() + 1, terminal.faces.len(), "TERMINAL_STYLES_MUST_NOT_CREATE_GLOBAL_FACES");
    for ranges in [vec![0..99], vec![0..5], vec![0..3, 2..4]] {
        let mut output = state.borrow_mut();
        output.snapshot.styles = ranges.into_iter().map(|bytes| TerminalStyleRun { bytes, style }).collect::<Vec<_>>().into();
        output.snapshot.text = "NEW\nλtwo".into();
        output.snapshot.revision += 1;
        output.pending = true;
        drop(output);
        assert!(vm.poll_terminal_output().is_err(), "INVALID_NATIVE_STYLES_MUST_BE_REJECTED_BEFORE_TEXT_EDITS");
        assert_eq!(vm.eval_source("(with-current-buffer terminal-buffer (buffer-string))").unwrap().display().unwrap(), "\"one\\nλtwo\"");
        assert_eq!(vm.eval_source("(list (buffer-string) (point))").unwrap().display().unwrap(), "(\"ordinary\" 4)");
    }
}

#[test]
fn snapshots_are_validated_before_mutating_text() {
    let (mut vm, state) = setup();
    vm.poll_terminal_output().unwrap();
    { let mut s = state.borrow_mut(); s.snapshot.cursor_byte = 5; s.snapshot.revision += 1; s.pending = true; }
    assert!(vm.poll_terminal_output().is_err(), "CURSOR_CANNOT_SPLIT_UTF8");
    assert_eq!(vm.eval_source("(buffer-string)").unwrap().display().unwrap(), "\"one\\nλtwo\"");
}

/// Redisplay's geometry publication: the frame snapshot the host paints.
fn publish(vm: &mut Vm) { vm.frame_snapshot(800, 600).unwrap(); }

fn mini_text(vm: &mut Vm, text: &str) {
    vm.eval_source(&format!("(setq resize-mini-windows t) (with-current-buffer (window-buffer (minibuffer-window)) (erase-buffer) (insert {text:?}))")).unwrap();
}

fn eval(vm: &mut Vm, source: &str) -> String { vm.eval_source(source).unwrap().display().unwrap() }

/// window.el `split-window' with a SIZE: WINDOW keeps the rest, and both
/// take their pixel share as normal size.
const SPLIT: &str = "(fset 'test-split (lambda (window size horizontal)
  (let* ((total (if horizontal (window-pixel-width window) (window-pixel-height window))) (rest (- total size)))
    (set-window-new-pixel window rest) (set-window-new-normal window (/ rest (float total)))
    (split-window-internal window size horizontal (/ size (float total))))))";

#[test]
fn a_grown_mini_window_never_resizes_the_pty() {
    let (mut vm, state) = setup();
    publish(&mut vm);
    // 800x600 at 20 px lines: 29 lines above the one-line mini-window, one of them the mode line.
    assert_eq!(state.borrow().resized, vec![(80, 28)], "THE_FIRST_PUBLICATION_SIZES_THE_PTY_TO_ITS_WINDOW");
    mini_text(&mut vm, "1\n2\n3\n4\n5");
    publish(&mut vm);
    assert_eq!(eval(&mut vm, "(window-body-height)"), "24", "THE_ROOT_WINDOW_SHRINKS_FOR_THE_MINI_WINDOW");
    mini_text(&mut vm, "");
    publish(&mut vm);
    assert_eq!(state.borrow().resized, vec![(80, 28)], "MINI_WINDOW_GROWTH_MUST_NOT_RESIZE_THE_PTY");
}

/// A window system that answers nothing about fonts.
struct Pgtk;
impl romacs_vm::DisplayHost for Pgtk {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
}

/// On a window-system frame with the pane gap (dividers and border 6) the
/// pty's rows exclude the window's bottom divider and its columns the
/// fringes, and a grown mini-window still never resizes it.
#[test]
fn the_pty_excludes_the_bottom_divider() {
    let (mut vm, state) = setup();
    vm.set_display_host(Some(Box::new(Pgtk)));
    assert!(vm.open_display());
    vm.eval_source("(modify-frame-parameters nil '((right-divider-width . 6) (bottom-divider-width . 6) (internal-border-width . 6)))").unwrap();
    publish(&mut vm);
    // 788x588 inside the border, 20 px of mini-window, 6 of divider, 20 of mode line.
    assert_eq!(eval(&mut vm, "(list (window-body-width) (window-body-height) (window-bottom-divider-width) (window-pixel-height))"), "(77 27 6 568)");
    assert_eq!(state.borrow().resized.last(), Some(&(77, 27)), "THE_PTY_EXCLUDES_THE_BOTTOM_DIVIDER");
    let before = state.borrow().resized.len();
    mini_text(&mut vm, "1\n2\n3\n4\n5");
    publish(&mut vm);
    mini_text(&mut vm, "");
    publish(&mut vm);
    assert_eq!(state.borrow().resized.len(), before, "MINI_WINDOW_GROWTH_MUST_NOT_RESIZE_THE_PTY");
}

#[test]
fn split_delete_and_host_resize_follow_the_controlling_window() {
    let (mut vm, state) = setup();
    publish(&mut vm);
    vm.eval_source(SPLIT).unwrap();
    vm.eval_source("(setq main (selected-window) other (test-split main 280 nil))").unwrap();
    publish(&mut vm);
    let split = *state.borrow().resized.last().unwrap();
    assert_eq!(split.1, eval(&mut vm, "(window-body-height)").parse::<u16>().unwrap(), "SPLIT_RESIZES_TO_THE_NEW_BODY");
    // The same buffer in both windows: the selected one controls.
    vm.eval_source("(set-window-buffer other terminal-buffer) (select-window other)").unwrap();
    publish(&mut vm);
    assert_eq!(state.borrow().resized.last().unwrap().1, eval(&mut vm, "(window-body-height other)").parse::<u16>().unwrap());
    // window.el `delete-window': MAIN takes the parent's whole extent.
    vm.eval_source("(select-window main) (set-window-new-pixel main 580) (set-window-new-normal main 1.0) (set-window-new-pixel (window-parent main) 580) (delete-window-internal other)").unwrap();
    publish(&mut vm);
    assert_eq!(*state.borrow().resized.last().unwrap(), (80, 28), "DELETE_RESIZES_TO_THE_WHOLE_BODY");
    // A host resize while a minibuffer shows five lines: one resize, to the stable body.
    let before = state.borrow().resized.len();
    mini_text(&mut vm, "1\n2\n3\n4\n5");
    vm.set_frame_geometry(1000, 800, 20, 10).unwrap();
    vm.frame_snapshot(1000, 800).unwrap();
    vm.frame_snapshot(1000, 800).unwrap();
    assert_eq!(state.borrow().resized[before..], [(100, 38)], "HOST_RESIZE_DURING_A_PROMPT_RESIZES_ONCE");
}

#[test]
fn a_hidden_terminal_keeps_its_size() {
    let (mut vm, state) = setup();
    publish(&mut vm);
    vm.eval_source(SPLIT).unwrap();
    vm.eval_source("(set-window-buffer (selected-window) (get-buffer-create \"ordinary\")) (test-split (selected-window) 280 nil)").unwrap();
    publish(&mut vm);
    assert_eq!(state.borrow().resized, vec![(80, 28)], "HIDDEN_TERMINAL_MUST_KEEP_ITS_SIZE");
}

#[test]
fn a_shorter_window_shows_the_cursor_row_last() {
    let (mut vm, state) = setup();
    let grid: String = (0..27).map(|n| format!("row{n}\n")).collect();
    {
        let mut s = state.borrow_mut();
        s.snapshot.text = format!("history\n{grid}$ ").into();
        s.snapshot.screen_start_byte = 8;
        s.snapshot.cursor_byte = s.snapshot.text.len();
        s.snapshot.rows = 28;
    }
    vm.poll_terminal_output().unwrap();
    publish(&mut vm);
    assert_eq!(eval(&mut vm, "(window-start)"), "9", "A_GRID_THAT_FITS_SHOWS_FROM_ITS_FIRST_ROW");
    mini_text(&mut vm, "1\n2\n3\n4\n5");
    publish(&mut vm);
    // 24 body rows: the cursor's row (the 28th) last, so rows 4.. show.
    assert_eq!(eval(&mut vm, "(save-excursion (goto-char (window-start)) (buffer-substring (point) (line-end-position)))"), "\"row4\"");
    mini_text(&mut vm, "");
    publish(&mut vm);
    assert_eq!(eval(&mut vm, "(window-start)"), "9", "THE_RESTORED_WINDOW_SHOWS_THE_GRID_FROM_ITS_FIRST_ROW");
}

#[test]
fn prompt_cycles_leave_an_uneven_split_unchanged() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 1430, 26, 10).unwrap();
    vm.eval_source(SPLIT).unwrap();
    // 1404 px of root: 728 over 676, the lower one split beside.
    vm.eval_source("(test-split (test-split (selected-window) 676 nil) 400 t)").unwrap();
    vm.frame_snapshot(800, 1430).unwrap();
    let heights = "(mapcar (lambda (w) (list (window-pixel-height w) (window-pixel-width w) (window-pixel-top w))) (window-list))";
    let before = eval(&mut vm, heights);
    for _ in 0..10 {
        mini_text(&mut vm, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12");
        vm.frame_snapshot(800, 1430).unwrap();
        assert_ne!(eval(&mut vm, heights), before, "THE_MINI_WINDOW_GREW");
        mini_text(&mut vm, "");
        vm.frame_snapshot(800, 1430).unwrap();
    }
    assert_eq!(eval(&mut vm, heights), before, "PROMPT_CYCLES_MUST_NOT_DRIFT_WINDOW_EDGES");
}

#[test]
fn output_scrolls_an_unselected_terminal_window_with_its_grid() {
    let (mut vm, state) = setup();
    let publish_text = |state: &Rc<RefCell<State>>, history: &str| {
        let mut s = state.borrow_mut();
        s.snapshot.text = format!("{history}g0\ng1\n$ ").into();
        s.snapshot.screen_start_byte = history.len();
        s.snapshot.cursor_byte = s.snapshot.text.len();
        s.snapshot.revision += 1;
        s.pending = true;
    };
    publish_text(&state, "h0\n");
    vm.poll_terminal_output().unwrap();
    publish(&mut vm);
    vm.eval_source(SPLIT).unwrap();
    // The terminal stays in the upper window; the selection moves away, as
    // it does into a minibuffer.
    vm.eval_source("(setq term-window (selected-window)) (select-window (test-split term-window 280 nil)) (set-window-buffer (selected-window) (get-buffer-create \"ordinary\"))").unwrap();
    publish(&mut vm);
    assert_eq!(eval(&mut vm, "(window-start term-window)"), "4");
    publish_text(&state, "h0\nh1\nh2\n");
    vm.poll_terminal_output().unwrap();
    publish(&mut vm);
    assert_eq!(eval(&mut vm, "(list (window-start term-window) (window-point term-window))"), "(10 18)",
        "OUTPUT_MUST_MOVE_AN_UNSELECTED_FOLLOWING_WINDOW_WITH_THE_GRID");
}

#[test]
fn history_rollover_keeps_an_unselected_terminal_window_following() {
    let (mut vm, state) = setup();
    let publish_text = |state: &Rc<RefCell<State>>, history: &str, grid: &str| {
        let mut s = state.borrow_mut();
        s.snapshot.text = format!("{history}{grid}0\n{grid}1\n$ ").into();
        s.snapshot.screen_start_byte = history.len();
        s.snapshot.cursor_byte = s.snapshot.text.len();
        s.snapshot.revision += 1;
        s.pending = true;
    };
    let history = |from: usize, to: usize| (from..to).map(|n| format!("h{n}\n")).collect::<String>();
    publish_text(&state, &history(0, 50), "g");
    vm.poll_terminal_output().unwrap();
    vm.eval_source(SPLIT).unwrap();
    vm.eval_source("(setq term-window (selected-window)) (select-window (test-split term-window 280 nil)) (set-window-buffer (selected-window) (get-buffer-create \"ordinary\"))").unwrap();
    publish(&mut vm);
    // The scrollback drops its oldest lines as new ones arrive: the whole
    // text before the grid is replaced.
    let rolled = history(40, 90);
    publish_text(&state, &rolled, "k");
    vm.poll_terminal_output().unwrap();
    publish(&mut vm);
    let screen = rolled.len() + 1;
    assert_eq!(eval(&mut vm, "(list (window-start term-window) (window-point term-window))"),
        format!("({screen} {})", screen + 8), "ROLLOVER_MUST_KEEP_AN_UNSELECTED_FOLLOWING_WINDOW_ON_THE_GRID");
}

#[test]
fn a_deletion_that_promotes_children_rebases_their_normal_sizes() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 1430, 26, 10).unwrap();
    vm.eval_source(SPLIT).unwrap();
    // A over B; B beside C; B over D. Deleting C promotes B and D into the root.
    // window.el `delete-window': every window but C spans the frame's width.
    vm.eval_source("(setq a (selected-window) b (test-split a 702 nil) c (test-split b 400 t) d (test-split b 351 nil))
      (dolist (w (list (frame-root-window) a (window-parent c) (window-parent b) b d))
        (set-window-new-pixel w 800) (set-window-new-normal w nil))
      (delete-window-internal c)").unwrap();
    vm.frame_snapshot(800, 1430).unwrap();
    let shape = "(mapcar (lambda (w) (list (window-pixel-height w) (window-normal-size w))) (window-list))";
    assert_eq!(eval(&mut vm, shape), "((702 0.5) (351 0.25) (351 0.25))", "PROMOTED_CHILDREN_TAKE_THEIR_SHARE_OF_THE_NEW_PARENT");
    mini_text(&mut vm, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12");
    vm.frame_snapshot(800, 1430).unwrap();
    assert_eq!(eval(&mut vm, "(mapcar #'window-pixel-height (window-list))"), "(559 279 280)", "GROWTH_IS_SHARED_BY_NORMAL_SIZE");
    mini_text(&mut vm, "");
    vm.frame_snapshot(800, 1430).unwrap();
    assert_eq!(eval(&mut vm, shape), "((702 0.5) (351 0.25) (351 0.25))");
}
