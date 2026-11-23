//! Typing receipts (src/typing.rs) through real commands: every native key
//! the command loop reads (`romacs--typing-step', lisp/vm/keyboard.el) is
//! settled exactly once, and charged a frame only when the frame shows the
//! very state its step left, the bytes the step edited among those painted.
use romacs_display_protocol::frame_time::{EventTime, observe_platform_now};
use romacs_display_protocol::submission::SubmissionReceipt;
use romacs_layout_input::{PointPlace, Row, WindowSnapshot};
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Outcome, Vm, WindowEnd};
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

/// The host's keyboard and screen, driven by the test: each event comes
/// from the frame whose id it names, if any.
struct Screen {
    events: Rc<RefCell<VecDeque<(HostEvent, Option<u64>)>>>,
    paint: Rc<Cell<Paint>>,
    pending: Rc<Cell<bool>>,
    last: Option<u64>,
    presentation: u64,
}

/// SNAPSHOT laid out on one row, showing its bytes from its start up to
/// `visible_end`.
fn laid(snapshot: &WindowSnapshot, paint: Paint) -> WindowEnd {
    let len = snapshot.text.utf8.len();
    let visible_end = match paint { Paint::Head(n) => (snapshot.start_byte + n).min(len + 1), _ => len + 1 };
    WindowEnd { id: snapshot.id, end: len, visible_end, body_height: snapshot.height as f32,
        rows: vec![Row { start: snapshot.start_byte, y: 0.0, height: 20.0 }], origin_y: 0.0, point: Some(PointPlace::default()) }
}

impl HostInput for Screen {
    fn input_pending(&mut self) -> bool { self.pending.get() }
    fn redisplay(&mut self, frame: HostFrame<'_>) -> Result<Vec<WindowEnd>, String> {
        let paint = self.paint.get();
        let ends: Vec<_> = frame.frame.windows.iter().map(|placed| laid(&placed.snapshot, paint)).collect();
        if paint != Paint::Nothing {
            self.presentation += 1;
            let picture = frame.typing.as_ref().filter(|_| paint != Paint::Elsewhere);
            let window = picture.and_then(|picture| ends.iter().find_map(|end| picture.painted(end, true)));
            *frame.submission = Some(SubmissionReceipt { presentation: self.presentation, submitted_at: observe_platform_now(), drawn_at: None, window });
        }
        Ok(ends)
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        let (event, frame) = self.events.borrow_mut().pop_front().ok_or("host script exhausted")?;
        self.last = frame;
        Ok(event)
    }
    fn event_frame(&mut self) -> Option<u64> { self.last }
    fn top_level_frames(&mut self) -> bool { true }
}

/// A GNU session typing into the buffer "typed", whose screen the test drives.
struct Session {
    vm: Vm,
    events: Rc<RefCell<VecDeque<(HostEvent, Option<u64>)>>>,
    paint: Rc<Cell<Paint>>,
    pending: Rc<Cell<bool>>,
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
        let (events, paint, pending) = (Rc::default(), Rc::new(Cell::new(Paint::Whole)), Rc::new(Cell::new(false)));
        vm.set_host_input(Some(Box::new(Screen { events: Rc::clone(&events), paint: Rc::clone(&paint), pending: Rc::clone(&pending), last: None, presentation: 0 })));
        vm.take_typing_outcomes();
        Self { vm, events, paint, pending, keys: Vec::new() }
    }

    /// The platform delivers KEY from FRAME.
    fn press_from(&mut self, key: CommandEvent, frame: Option<u64>) {
        let at = observe_platform_now();
        self.keys.push(at);
        self.events.borrow_mut().push_back((HostEvent::NativeKey(key, at), frame));
    }

    fn press(&mut self, key: CommandEvent) {
        self.press_from(key, None);
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
        self.events.borrow_mut().push_back((HostEvent::Timeout, None));
        assert_eq!(self.eval("(romacs--read-host-event 0)"), "nil");
    }

    /// The keys settled since the last call: each by its index among the
    /// keys pressed, and what became of it.
    fn settled(&mut self) -> Vec<(usize, String)> {
        self.vm.take_typing_outcomes().into_iter().map(|key| {
            let index = self.keys.iter().position(|&at| at == key.ingress).expect("a pressed key");
            let outcome = match key.outcome {
                Outcome::Rendered(receipt) => format!("rendered {}", receipt.presentation),
                Outcome::Unchanged => "unchanged".into(),
                Outcome::Unattributed(why) => why.name().into(),
            };
            (index, outcome)
        }).collect()
    }

    /// The keys settled since the last look, each once, as OUTCOMES says.
    fn expect(&mut self, outcomes: &[(usize, &str)]) {
        let settled = self.settled();
        let names: Vec<(usize, &str)> = settled.iter().map(|(i, o)| (*i, o.split(' ').next().unwrap())).collect();
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
    s.pending.set(true);
    s.press(character('b'));
    s.press(character('c'));
    s.command();
    s.command();
    s.pending.set(false);
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

/// Keys a command reads, as data or as the rest of its sequence, are its
/// own; a key pushed back and read again is charged once; a keyboard
/// macro's events are no keys; a key read with no command running is
/// settled at once.
#[test]
fn keys_a_command_reads_are_its_own() {
    let mut s = Session::new();
    s.press(CommandEvent::Character(17));
    s.press(character('x'));
    s.command();
    s.redisplay();
    s.expect(&[(0, "rendered"), (1, "rendered")]);
    s.press(CommandEvent::Character(24));
    s.press(character('h'));
    s.command();
    s.redisplay();
    s.expect(&[(2, "rendered"), (3, "rendered")]);
    s.press(named("f5"));
    s.press(character('z'));
    s.command();
    s.command();
    s.redisplay();
    s.expect(&[(4, "superseded"), (5, "superseded")]);
    s.press(named("f6"));
    s.command();
    s.redisplay();
    s.expect(&[(6, "rendered")]);
    s.press(character('q'));
    assert_eq!(s.eval("(read-event)"), "113");
    s.expect(&[(7, "outside-command")]);
}

/// A step inside another (the minibuffer's, a recursive edit's) settles
/// its own keys at the prompt's redisplay; the key that throws out of it
/// belongs to the step around it.
#[test]
fn nested_command_loops_settle_their_keys() {
    let mut s = Session::new();
    s.press(named("f7"));
    s.press(character('x'));
    s.press(character('\n'));
    s.command();
    s.redisplay();
    assert_eq!(s.eval("(buffer-substring (line-beginning-position) (line-end-position))"), "\"hexllo\"");
    s.expect(&[(1, "rendered"), (0, "rendered"), (2, "rendered")]);
    s.press(named("f8"));
    s.press(character('a'));
    s.press(CommandEvent::Character((1 << 27) | 3));
    s.command();
    s.redisplay();
    s.expect(&[(4, "rendered"), (3, "rendered"), (5, "rendered")]);
}

/// A key from another frame is read after its `switch-frame', which is no
/// key: the key is claimed once, by the step that reads it.
#[test]
fn a_deferred_key_is_claimed_once() {
    let mut s = Session::new();
    let other = s.eval("(frame-id (romacs--create-frame '((name . \"two\"))))").parse().expect("a frame id");
    s.press_from(character('k'), Some(other));
    s.command();
    s.command();
    s.redisplay();
    let settled = s.settled();
    assert_eq!(settled.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0], "settled: {settled:?}");
}
