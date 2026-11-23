//! Typing receipts (src/typing.rs) through real commands: every native key
//! the command loop reads (`romacs--typing-step', lisp/vm/keyboard.el) is
//! settled exactly once, and charged a frame only when the frame shows the
//! very state its step left, the bytes the step edited among those painted.
use romacs_display_protocol::frame_time::{EventTime, observe_platform_now};
use romacs_display_protocol::submission::SubmissionReceipt;
use romacs_layout_input::{FrameSnapshot, PointPlace, Row, WindowSnapshot};
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, KeyOutcome, Outcome, Vm, WindowEnd};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;

/// What the screen does with a redisplay.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Paint {
    /// Submits a scene showing every window whole, its cursor drawn.
    Whole,
    /// Submits a scene showing only the first N bytes of each window.
    Head(usize),
    /// Submits a scene that does not show the step's window.
    Elsewhere,
    /// Submits nothing: the frame is as last painted.
    Nothing,
}

/// A scripted event: what the platform delivers, the top-level frame it
/// comes from, and whether it was typed while the key before it ran (so a
/// read finds it pending and does not redisplay first).
type Scripted = (HostEvent, Option<u64>, bool);

/// The host's keyboard and screen, driven by the test.
struct Screen {
    events: Rc<RefCell<VecDeque<Scripted>>>,
    paint: Rc<Cell<Paint>>,
    last: Option<u64>,
    presented: Rc<Cell<u64>>,
    /// The frame on screen and how it was painted: a redisplay that would
    /// paint it again submits nothing, as the GUI's does.
    shown: Option<(FrameSnapshot, u64, Paint)>,
    /// The outcomes handed over as reads settled them, and how many had
    /// been handed when each event was read.
    handed: Rc<RefCell<Vec<KeyOutcome>>>,
    handed_at_reads: Rc<RefCell<Vec<usize>>>,
}

/// SNAPSHOT laid out on one row, showing its bytes from its start up to
/// `visible_end`.
/// Whether frames A and B paint alike (the GUI's `frames_alike`, in short).
fn alike(a: &FrameSnapshot, b: &FrameSnapshot) -> bool {
    a.lays_out_like(b) && a.children == b.children
}

fn laid(snapshot: &WindowSnapshot, paint: Paint) -> WindowEnd {
    let len = snapshot.text.utf8.len();
    let visible_end = match paint { Paint::Head(n) => (snapshot.start_byte + n).min(len + 1), _ => len + 1 };
    WindowEnd { id: snapshot.id, end: len, visible_end, body_height: snapshot.height as f32,
        rows: vec![Row { start: snapshot.start_byte, y: 0.0, height: 20.0 }], origin_y: 0.0, point: Some(PointPlace::default()) }
}

impl HostInput for Screen {
    fn input_pending(&mut self) -> bool { self.events.borrow().front().is_some_and(|event| event.2) }
    fn redisplay(&mut self, frame: HostFrame<'_>) -> Result<Vec<WindowEnd>, String> {
        let paint = self.paint.get();
        let ends: Vec<_> = frame.frame.windows.iter().map(|placed| laid(&placed.snapshot, paint)).collect();
        let again = self.shown.as_ref().is_some_and(|(shown, selected, painted)| (*selected, *painted) == (frame.selected, paint) && alike(shown, &frame.frame));
        if paint != Paint::Nothing && !again {
            self.shown = Some((frame.frame.clone(), frame.selected, paint));
            self.presented.set(self.presented.get() + 1);
            let picture = frame.typing.as_ref().filter(|_| paint != Paint::Elsewhere);
            let window = picture.and_then(|picture| ends.iter().find_map(|end| picture.painted(end, true)));
            *frame.submission = Some(SubmissionReceipt { presentation: self.presented.get(), submitted_at: observe_platform_now(), window });
        }
        Ok(ends)
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.handed_at_reads.borrow_mut().push(self.handed.borrow().len());
        let (event, frame, _) = self.events.borrow_mut().pop_front().ok_or("host script exhausted")?;
        self.last = frame;
        Ok(event)
    }
    fn event_frame(&mut self) -> Option<u64> { self.last }
    fn top_level_frames(&mut self) -> bool { true }
    fn typing_settled(&mut self, outcomes: Vec<KeyOutcome>) -> Vec<KeyOutcome> {
        self.handed.borrow_mut().extend(outcomes);
        Vec::new()
    }
}

/// A GNU session typing into the buffer "typed", whose screen the test drives.
struct Session {
    vm: Vm,
    events: Rc<RefCell<VecDeque<Scripted>>>,
    paint: Rc<Cell<Paint>>,
    presented: Rc<Cell<u64>>,
    handed: Rc<RefCell<Vec<KeyOutcome>>>,
    handed_at_reads: Rc<RefCell<Vec<usize>>>,
    keys: Vec<EventTime>,
}

const COMMANDS: &str = r#"(progn
  (switch-to-buffer (get-buffer-create "typed"))
  (insert "hello\nworld\n")
  (goto-char 3)
  (defun typed-tail () (save-excursion (goto-char (point-max)) (insert "tail")))
  (global-set-key [f5] (lambda () (interactive) (push (read-event) unread-command-events)))
  (global-set-key [f6] (lambda () (interactive) (execute-kbd-macro "xy")))
  (global-set-key [f7] (lambda () (interactive) (insert (read-string "? "))))
  (global-set-key [f8] (lambda () (interactive) (recursive-edit)))
  (global-set-key [f10] (lambda () (interactive) (buffer-swap-text (get-buffer-create "other"))))
  (global-set-key [f11] (lambda () (interactive) (with-current-buffer (get-buffer-create "other") (insert "o")) (insert "m"))))"#;

impl Session {
    fn new() -> Self {
        let mut vm = gnu_lisp::gnu_lisp(&[]);
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.eval_source(COMMANDS).unwrap();
        let s = Self { vm, events: Rc::default(), paint: Rc::new(Cell::new(Paint::Whole)), presented: Rc::default(),
                       handed: Rc::default(), handed_at_reads: Rc::default(), keys: Vec::new() };
        let screen = Screen { events: Rc::clone(&s.events), paint: Rc::clone(&s.paint), last: None, presented: Rc::clone(&s.presented), shown: None,
                              handed: Rc::clone(&s.handed), handed_at_reads: Rc::clone(&s.handed_at_reads) };
        let mut s = s;
        s.vm.set_host_input(Some(Box::new(screen)));
        s.vm.take_typing_outcomes();
        s
    }

    /// The platform delivers KEY from FRAME; BURST when typed while the key
    /// before it still runs.
    fn deliver(&mut self, key: CommandEvent, frame: Option<u64>, burst: bool) {
        let at = observe_platform_now();
        self.keys.push(at);
        self.events.borrow_mut().push_back((HostEvent::NativeKey(key, at), frame, burst));
    }

    fn press(&mut self, key: CommandEvent) {
        self.deliver(key, None, false);
    }

    /// KEY typed while the key before it still runs: its read does not
    /// redisplay first.
    fn press_at_once(&mut self, key: CommandEvent) {
        self.deliver(key, None, true);
    }

    fn eval(&mut self, source: &str) -> String {
        self.vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
    }

    /// One command, as the GUI loop runs it.
    fn command(&mut self) {
        let ran = self.eval("(romacs--run-host-command)");
        assert!(!ran.starts_with("VM-ERROR"), "command: {ran}");
    }

    /// The redisplay the loop does once input is read: a read that times out.
    fn redisplay(&mut self) {
        self.events.borrow_mut().push_back((HostEvent::Timeout, None, false));
        assert_eq!(self.eval("(romacs--read-host-event 0)"), "nil");
    }

    /// The keys settled since the last call, those handed over first: each
    /// by its index among the keys pressed, what became of it, and the
    /// presentation it was charged.
    fn outcomes(&mut self) -> Vec<(usize, &'static str, Option<u64>)> {
        let mut settled: Vec<KeyOutcome> = self.handed.borrow_mut().drain(..).collect();
        settled.extend(self.vm.take_typing_outcomes());
        settled.into_iter().map(|key| {
            let index = self.keys.iter().position(|&at| at == key.ingress).expect("a pressed key");
            match key.outcome {
                Outcome::Rendered(frame) => (index, "rendered", Some(frame.presentation)),
                Outcome::Opened(frame) => (index, "opened", Some(frame.presentation)),
                Outcome::Unchanged => (index, "unchanged", None),
                Outcome::Unattributed(why) => (index, why.name(), None),
            }
        }).collect()
    }

    /// The keys settled since the last look, each once, as OUTCOMES says.
    fn expect(&mut self, outcomes: &[(usize, &str)]) {
        let settled = self.outcomes();
        let names: Vec<(usize, &str)> = settled.iter().map(|(i, o, _)| (*i, *o)).collect();
        assert_eq!(names, outcomes, "settled: {settled:?}");
    }
}

fn character(c: char) -> CommandEvent { CommandEvent::Character(c as i64) }
fn named(name: &str) -> CommandEvent { CommandEvent::Named(name.into()) }

/// A key whose step's state the next submitted frame shows is rendered;
/// one a later step replaced first is superseded; one followed by no
/// submission is unchanged; one whose window the frame left out is not
/// shown; one whose edit the frame did not paint is offscreen.
#[test]
fn each_native_key_settles_once() {
    let mut s = Session::new();
    s.press(character('a'));
    s.command();
    s.redisplay();
    s.expect(&[(0, "rendered")]);
    // Typed faster than the screen: no redisplay between the two.
    s.press(character('b'));
    s.press_at_once(character('c'));
    s.command();
    s.command();
    s.redisplay();
    s.expect(&[(1, "superseded"), (2, "rendered")]);
    s.paint.set(Paint::Nothing);
    s.press(character('d'));
    s.command();
    s.redisplay();
    s.expect(&[(3, "unchanged")]);
    s.paint.set(Paint::Elsewhere);
    s.press(character('e'));
    s.command();
    s.redisplay();
    s.expect(&[(4, "not-shown")]);
    s.paint.set(Paint::Head(2));
    s.press(character('f'));
    s.command();
    s.redisplay();
    s.expect(&[(5, "offscreen")]);
}

/// The span a step is charged by is the bounding box of every write since
/// it began, `post-command-hook`'s too; what happens after it ends is not
/// its own: a timer's edit before the frame supersedes it, a collection
/// changes nothing, a swapped text is an unknown edit.
#[test]
fn a_steps_edit_is_every_write_it_made() {
    let mut s = Session::new();
    s.paint.set(Paint::Head(2));
    s.eval("(add-hook 'post-command-hook #'typed-tail)");
    s.press(CommandEvent::Character(6));
    s.command();
    s.eval("(remove-hook 'post-command-hook #'typed-tail)");
    s.redisplay();
    s.expect(&[(0, "offscreen")]);
    s.paint.set(Paint::Whole);
    s.press(named("f11"));
    s.command();
    s.redisplay();
    s.expect(&[(1, "rendered")]);
    s.press(character('g'));
    s.command();
    s.eval("(insert \"z\")");
    s.redisplay();
    s.expect(&[(2, "superseded")]);
    s.press(character('h'));
    s.command();
    s.eval("(garbage-collect)");
    s.redisplay();
    s.expect(&[(3, "rendered")]);
    s.press(named("f10"));
    s.command();
    s.redisplay();
    s.expect(&[(4, "edit-unknown")]);
}

/// A key's latency ends at the first frame it caused; time waiting for the
/// user is never latency. A key its step holds when it reads the next
/// native key settles then: C-q and a bare prefix C-x with no frame since
/// awaited input. The key read next is charged its own frame. A key
/// pushed back and read again is charged once, and named for it; a
/// keyboard macro's events are no keys; a key read with no command running
/// is settled at once.
#[test]
fn keys_a_command_reads_are_its_own() {
    let mut s = Session::new();
    s.press(CommandEvent::Character(17));
    s.press_at_once(character('x'));
    s.command();
    s.redisplay();
    // x is charged the frame its step's redisplay submitted, the newest.
    assert_eq!(s.outcomes(), [(0, "awaited-input", None), (1, "rendered", Some(s.presented.get()))]);
    s.press(CommandEvent::Character(24));
    s.press_at_once(character('h'));
    s.command();
    s.redisplay();
    s.expect(&[(2, "awaited-input"), (3, "rendered")]);
    // f5 reads z after a redisplay that finds nothing to paint, so f5
    // waited for input; it pushes z back for the next step.
    s.press(named("f5"));
    s.press(character('z'));
    s.command();
    s.command();
    s.redisplay();
    s.expect(&[(4, "awaited-input"), (5, "unread")]);
    s.press(named("f6"));
    s.command();
    s.redisplay();
    s.expect(&[(6, "rendered")]);
    s.press(character('q'));
    assert_eq!(s.eval("(read-event)"), "113");
    s.expect(&[(7, "outside-command")]);
}

/// The key that opens a prompt is charged the prompt's first frame
/// (`opened`), not the prompt: M-x's minibuffer, C-x C-f's after a prefix
/// with no echo. A prefix typed with a pause, whose read's redisplay finds
/// the frame as painted, waited for input; one whose read changed the
/// screen is charged that frame.
#[test]
fn an_opener_is_charged_its_prompts_first_frame() {
    let mut s = Session::new();
    let before = s.presented.get();
    s.press(CommandEvent::Character((1 << 27) | 'x' as i64));
    for c in "ignore".chars() { s.press(character(c)); }
    s.press(character('\n'));
    s.command();
    s.redisplay();
    let settled = s.outcomes();
    // A redisplay before the M-x read, then the minibuffer's first frame.
    assert_eq!(settled[0], (0, "opened", Some(before + 2)), "settled: {settled:?}");
    assert_eq!(settled.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), (0..8).collect::<Vec<_>>(), "settled: {settled:?}");
    assert!(settled[1..].iter().all(|(_, o, _)| *o == "rendered"), "settled: {settled:?}");
    let before = s.presented.get();
    s.press(CommandEvent::Character(24));
    s.press_at_once(CommandEvent::Character(6));
    s.press(character('z'));
    s.press(CommandEvent::Character(7));
    s.command();
    s.redisplay();
    let settled = s.outcomes();
    assert_eq!(&settled[..3], &[(8, "awaited-input", None), (9, "opened", Some(before + 2)), (10, "rendered", Some(before + 3))], "settled: {settled:?}");
    assert_eq!(settled.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), (8..12).collect::<Vec<_>>(), "settled: {settled:?}");
    // C-f typed a while after C-x, the screen settled first: the read of
    // C-f redisplays, finds the frame as painted and submits nothing, so
    // C-x waited for input; C-f is charged the prompt's first frame.
    s.eval("(message nil)");
    s.redisplay();
    let before = s.presented.get();
    s.press(CommandEvent::Character(24));
    s.press(CommandEvent::Character(6));
    s.press(character('z'));
    s.press(CommandEvent::Character(7));
    s.command();
    s.redisplay();
    let settled = s.outcomes();
    assert_eq!(&settled[..3], &[(12, "awaited-input", None), (13, "opened", Some(before + 1)), (14, "rendered", Some(before + 2))], "settled: {settled:?}");
    assert_eq!(settled.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), (12..16).collect::<Vec<_>>(), "settled: {settled:?}");
    // A prefix whose read changed the screen (here it cleared the echo
    // area; Romacs's key-help echo is another) is charged that frame.
    s.eval("(message \"hello\")");
    s.redisplay();
    let before = s.presented.get();
    s.press(CommandEvent::Character(24));
    s.press(CommandEvent::Character(6));
    s.press(character('z'));
    s.press(CommandEvent::Character(7));
    s.command();
    s.redisplay();
    let settled = s.outcomes();
    assert_eq!(&settled[..3], &[(16, "opened", Some(before + 1)), (17, "opened", Some(before + 2)), (18, "rendered", Some(before + 3))], "settled: {settled:?}");
    assert_eq!(settled.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), (16..20).collect::<Vec<_>>(), "settled: {settled:?}");
}

/// A step inside another (the minibuffer's, a recursive edit's) settles
/// its own keys at the prompt's redisplay, and hands them over then, while
/// the prompt still reads; the key that throws out of it belongs to the
/// step around it.
#[test]
fn nested_command_loops_settle_their_keys() {
    let mut s = Session::new();
    s.press(named("f7"));
    s.press(character('x'));
    s.press(character('\n'));
    s.command();
    s.redisplay();
    assert_eq!(s.eval("(buffer-substring (line-beginning-position) (line-end-position))"), "\"hexllo\"");
    // Reading C-j, the third read: f7 and x were handed over already.
    assert!(s.handed_at_reads.borrow()[2] >= 2, "handed at reads: {:?}", s.handed_at_reads.borrow());
    s.expect(&[(0, "opened"), (1, "rendered"), (2, "rendered")]);
    s.press(named("f8"));
    s.press(character('a'));
    s.press(CommandEvent::Character((1 << 27) | 3));
    s.command();
    s.redisplay();
    s.expect(&[(3, "opened"), (4, "rendered"), (5, "rendered")]);
}

/// A key from another frame is read after its `switch-frame', which is no
/// key: the step that reads it claims it, once, and the frame that shows
/// that frame's window charges it.
#[test]
fn a_deferred_key_is_claimed_once() {
    let mut s = Session::new();
    let other = s.eval("(frame-id (romacs--create-frame '((name . \"two\"))))").parse().expect("a frame id");
    s.deliver(character('k'), Some(other), false);
    s.command();
    s.command();
    s.redisplay();
    s.expect(&[(0, "rendered")]);
}

/// A C-g that quits a key sequence throws out of its step: the sequence's
/// keys still settle, each once.
#[test]
fn a_quit_sequence_settles_its_keys() {
    let mut s = Session::new();
    s.press(CommandEvent::Character(24));
    s.press(CommandEvent::Character(7));
    s.command();
    s.redisplay();
    let settled = s.outcomes();
    assert_eq!(settled.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), vec![0, 1], "settled: {settled:?}");
}

/// A window deleted before the frame: the frame cannot show its step.
#[test]
fn a_window_deleted_before_the_frame_is_not_shown() {
    let mut s = Session::new();
    s.eval("(select-window (split-window))");
    s.press(character('w'));
    s.command();
    s.eval("(delete-window)");
    s.redisplay();
    s.expect(&[(0, "not-shown")]);
}
