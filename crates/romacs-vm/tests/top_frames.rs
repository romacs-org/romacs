//! Top-level frames against pinned GNU: making, selecting, cycling,
//! hiding and deleting them (top_frames.el), evaluated form by form in one
//! session over a host that shows more than one top-level frame, and
//! compared, as printed, with pinned GNU X11's answers (top_frames.gnu,
//! child_frames_oracle.py --probe).
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::collections::VecDeque;
#[path = "support/gui_probe.rs"]
mod gui_probe;

/// A host whose display shows any number of top-level frames, and which
/// is never asked for input.
struct Display;

impl HostInput for Display {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> { Err("no input".into()) }
    fn top_level_frames(&mut self) -> bool { true }
}

fn forms(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(';'))
}

fn session(host: bool) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(gui_probe::Pgtk)));
    assert!(vm.open_display());
    vm.eval_source("(set-buffer \"*scratch*\")").unwrap();
    if host { vm.set_host_input(Some(Box::new(Display))); }
    vm
}

#[test]
fn top_level_frames_match_pinned_gnu() {
    let probe = include_str!("top_frames.el");
    let expected: Vec<&str> = forms(include_str!("top_frames.gnu")).collect();
    let mut vm = session(true);
    let actual: Vec<String> = forms(probe).map(|form| {
        if form.starts_with("(setq ") {
            vm.eval_source(form).map(|_| "-".to_owned()).unwrap_or_else(|e| format!("setup failed: {e}"))
        } else {
            let source = format!("(prin1-to-string (condition-case e {form} (error (list 'signal e))))");
            vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
                printed[1..printed.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            }).unwrap_or_else(|e| format!("eval failed: {e}"))
        }
    }).collect();
    assert_eq!(actual.len(), expected.len(), "top_frames.gnu is stale: rerun child_frames_oracle.py --probe");
    let failures: Vec<String> = forms(probe).zip(&expected).zip(&actual).filter(|((_, gnu), romacs)| gnu != romacs)
        .map(|((form, gnu), romacs)| format!("{form}\n  GNU    {gnu}\n  Romacs {romacs}")).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A host showing one top-level frame keeps it the only one: `make-frame'
/// without a `parent-frame' is refused, as before a second window exists.
#[test]
fn a_host_with_one_window_refuses_a_second_top_level_frame() {
    let mut vm = session(false);
    let made = vm.eval_source("(condition-case e (romacs--create-frame '((name . \"two\"))) (error (car (cdr e))))").unwrap();
    assert_eq!(made.display().unwrap(), "\"This display shows one top-level frame; a new frame needs a `parent-frame' parameter\"");
    assert_eq!(vm.eval_source("(length (frame-list))").unwrap().display().unwrap(), "1");
}

/// Each top-level frame lays its windows out in its own size: making and
/// sizing a second frame leaves the first frame's windows where they were.
#[test]
fn a_second_frame_leaves_the_first_frames_windows_alone() {
    let mut vm = session(true);
    let edges = "(mapcar (lambda (w) (list (window-pixel-left w) (window-pixel-top w) (window-pixel-width w) (window-pixel-height w))) (window-list F t))";
    vm.eval_source("(setq F (selected-frame))").unwrap();
    let before = vm.eval_source(edges).unwrap().display().unwrap();
    vm.eval_source("(setq G (romacs--create-frame '((width . 30) (height . 8))))").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
    assert_eq!(vm.eval_source("(list (frame-text-cols G) (frame-text-lines G) (frame-text-cols F) (frame-text-lines F))").unwrap().display().unwrap(),
               "(30 8 78 30)");
    vm.eval_source("(delete-frame G)").unwrap();
    assert_eq!(vm.eval_source(edges).unwrap().display().unwrap(), before);
}


/// A display of several top-level frames whose input comes from a script:
/// each event names the id of the frame whose window it came from.
struct Frames {
    events: VecDeque<(HostEvent, u64)>,
    last: Option<u64>,
}

impl HostInput for Frames {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        let (event, frame) = self.events.pop_front().ok_or("host script exhausted")?;
        self.last = Some(frame);
        Ok(event)
    }
    fn event_frame(&mut self) -> Option<u64> { self.last }
    fn top_level_frames(&mut self) -> bool { true }
}

fn key(c: char) -> HostEvent { HostEvent::Key(CommandEvent::Character(c as i64)) }

/// F, the initial frame, selected, and G, a second top-level frame; the
/// script's events then come from frames by id (`frame-id').
fn two_frames(script: impl FnOnce(u64, u64) -> Vec<(HostEvent, u64)>) -> Vm {
    let mut vm = session(true);
    vm.eval_source("(setq F (selected-frame) G (romacs--create-frame '((name . \"two\"))))").unwrap();
    let g = vm.eval_source("(frame-id G)").unwrap().display().unwrap().parse().unwrap();
    vm.set_host_input(Some(Box::new(Frames { events: script(1, g).into(), last: None })));
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// keyboard.c kbd_buffer_get_event: a key from a frame neither the last
/// event's nor the selected one is read after a `switch-frame' to that
/// frame, and later keys from it are not.
#[test]
fn a_key_from_another_frame_is_read_after_its_switch_frame() {
    let mut vm = two_frames(|_, g| vec![(key('a'), g), (key('b'), g)]);
    assert_eq!(eval(&mut vm, "(let* ((e1 (read-event)) (e2 (read-event)) (e3 (read-event))) (list (car-safe e1) (eq (car (cdr e1)) G) e2 e3 (eq last-event-frame G) (eq (selected-frame) F)))"),
               "(switch-frame t 97 98 t t)");
}

/// read_filtered_event's NO-SWITCH-FRAME: `read-char' reads the key and
/// leaves the `switch-frame' for the next read.
#[test]
fn read_char_puts_off_the_switch_frame() {
    let mut vm = two_frames(|_, g| vec![(key('a'), g)]);
    assert_eq!(eval(&mut vm, "(let* ((c (read-char)) (e (read-event))) (list c (car-safe e) (eq (car (cdr e)) G)))"), "(97 switch-frame t)");
}

/// read_key_sequence: a `switch-frame' is the sequence when the caller can
/// take one, else it is delayed until after the sequence.
#[test]
fn a_key_sequence_returns_or_delays_the_switch_frame() {
    let mut vm = two_frames(|_, g| vec![(key('a'), g)]);
    assert_eq!(eval(&mut vm, "(let ((k (read-key-sequence nil nil nil t))) (list (length k) (car-safe (aref k 0)) (read-event)))"), "(1 switch-frame 97)");
    let mut vm = two_frames(|_, g| vec![(key('a'), g)]);
    assert_eq!(eval(&mut vm, "(let* ((k (read-key-sequence nil)) (e (read-event))) (list k (car-safe e)))"), "(\"a\" switch-frame)");
}

/// Finternal_handle_focus_in: the focus moving to another frame queues
/// its `switch-frame', read before any host event.
#[test]
fn focus_in_another_frame_queues_its_switch_frame() {
    let mut vm = two_frames(|_, _| vec![]);
    assert_eq!(eval(&mut vm, "(progn (internal-handle-focus-in (list 'focus-in G)) (let ((e (read-event))) (list (car-safe e) (eq (car (cdr e)) G))))"), "(switch-frame t)");
    assert_eq!(eval(&mut vm, "(progn (internal-handle-focus-in (list 'focus-in G)) (romacs--take-unread-switch-frame))"), "nil");
}

/// `handle-switch-frame' selects the event's frame.
#[test]
fn handle_switch_frame_selects_the_frame() {
    let mut vm = two_frames(|_, _| vec![]);
    assert_eq!(eval(&mut vm, "(progn (handle-switch-frame (list 'switch-frame G)) (list (eq (selected-frame) G) (eq (window-frame (selected-window)) G)))"), "(t t)");
}

/// A resize names the frame whose window changed size: only its windows
/// are laid out anew.
#[test]
fn a_resize_lays_out_the_frame_it_names() {
    let mut vm = two_frames(|_, g| vec![(HostEvent::Resize { width: 500, height: 300 }, g), (key('x'), 1)]);
    let before = eval(&mut vm, "(list (frame-native-width F) (frame-native-height F))");
    assert_eq!(eval(&mut vm, "(read-event)"), "120");
    assert_eq!(eval(&mut vm, "(list (frame-native-width G) (frame-native-height G))"), "(500 300)");
    assert_eq!(eval(&mut vm, "(list (frame-native-width F) (frame-native-height F))"), before);
}

/// Closing the window of one of two visible frames is GNU's
/// `(delete-frame (FRAME))' event, which `handle-delete-frame' answers.
#[test]
fn closing_one_of_two_frames_is_a_delete_frame_event() {
    let mut vm = two_frames(|_, g| vec![(HostEvent::Closed, g)]);
    assert_eq!(eval(&mut vm, "(let ((e (romacs--read-host-event))) (list (car-safe e) (eq (car (car (cdr e))) G)))"), "(delete-frame t)");
}
