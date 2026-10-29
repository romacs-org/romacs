//! `read-from-minibuffer' reading a host keyboard in the middle of an
//! evaluation (host_input.rs): GNU keyboard.c `read_char' redisplays, then
//! waits, while the command that prompted stays suspended.
//!
//! Expected minibuffer contents come from live GNU 32.0.50, not from VM2:
//! `ert-simulate-keys' with a `post-command-hook' recording `buffer-string'
//! in the minibuffer (evidence: crates/romacs-vm-gui/evidence/prompt/
//! gnu-prompt.el and gnu-prompt.out). GNU printed, for keys "a b c RET" at
//! "Name: ": RESULT "abc", contents "Name: " "Name: a" "Name: ab" "Name: abc";
//! for "a C-g" at "P: ": (quit (minibuffer-quit)).
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Default)]
struct Seen {
    /// Minibuffer-window text at each redisplay.
    mini: Vec<String>,
    /// Was the minibuffer window the selected one at that redisplay?
    mini_selected: Vec<bool>,
    /// Mode line of the first window at each redisplay.
    mode_lines: Vec<String>,
    /// Frame width at each redisplay.
    widths: Vec<u32>,
    reads: usize,
}

struct Script {
    events: VecDeque<HostEvent>,
    pending: bool,
    seen: Rc<RefCell<Seen>>,
    /// Layout end (a byte) reported for the first window, as a painter would.
    first_end: Option<usize>,
}

impl HostInput for Script {
    fn input_pending(&mut self) -> bool { self.pending }
    fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<(u64, usize)>, String> {
        let mut seen = self.seen.borrow_mut();
        let mini = frame.frame.windows.last().ok_or("frame without minibuffer")?;
        seen.mini.push(mini.snapshot.text.utf8.clone());
        seen.mini_selected.push(mini.snapshot.id == frame.selected);
        seen.mode_lines.push(
            frame.frame.windows[0].snapshot.mode_line.as_ref().map(|m| m.utf8.clone()).unwrap_or_default(),
        );
        seen.widths.push(frame.frame.width);
        Ok(self.first_end.map(|end| vec![(frame.frame.windows[0].snapshot.id, end)]).unwrap_or_default())
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.seen.borrow_mut().reads += 1;
        // An exhausted script fails the request instead of hanging the test.
        self.events.pop_front().ok_or_else(|| "host script exhausted".to_string())
    }
}

fn key(c: char) -> HostEvent { HostEvent::Key(CommandEvent::Character(c as i64)) }
const RET: HostEvent = HostEvent::Key(CommandEvent::Character(13));
const CG: HostEvent = HostEvent::Key(CommandEvent::Character(7));

fn vm_with(events: Vec<HostEvent>, pending: bool) -> (Vm, Rc<RefCell<Seen>>) {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    // Install the Lisp half before the host is attached. MNB DEFECT, not this
    // lane's: on a fresh VM the FIRST prompt, called outside any
    // condition-case, fails with "S0 call depth exceeded" (host or not);
    // wrapped calls and every later call work. Batch, this reaches the gate.
    assert_eq!(
        eval(&mut vm, "(condition-case e (read-string \"\") (error e))"),
        "(end-of-file \"Error reading from stdin\")"
    );
    let seen = Rc::new(RefCell::new(Seen::default()));
    vm.set_host_input(Some(Box::new(Script { events: events.into(), pending, seen: seen.clone(), first_end: None })));
    (vm, seen)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

/// The prompt is displayed in the selected minibuffer window before every
/// key, each key echoes into it, and RET returns the text to the waiting Lisp.
#[test]
fn host_keys_reach_a_real_prompt_and_ret_returns_the_string() {
    let (mut vm, seen) = vm_with(vec![key('a'), key('b'), key('c'), RET], false);
    assert_eq!(eval(&mut vm, "(read-from-minibuffer \"Name: \")"), "\"abc\"");
    let seen = seen.borrow();
    assert_eq!(seen.mini, ["Name: ", "Name: a", "Name: ab", "Name: abc"], "GNU's minibuffer contents per key");
    assert_eq!(seen.mini_selected, [true; 4], "the prompt's window is selected while it waits");
    assert_eq!(seen.reads, 4);
    // read_minibuf_unwind gives the minibuffer window back its idle buffer.
    assert_eq!(eval(&mut vm, "(eq (selected-window) (minibuffer-window))"), "nil");
}

/// C-g at the prompt SIGNALS `minibuffer-quit' (GNU: (quit (minibuffer-quit)));
/// it does not return.
#[test]
fn c_g_signals_minibuffer_quit() {
    let (mut vm, seen) = vm_with(vec![key('a'), CG], false);
    assert_eq!(
        eval(&mut vm, "(condition-case q (list 'returned (read-from-minibuffer \"P: \")) (error (list 'error q)) (quit (list 'quit q)))"),
        "(quit (minibuffer-quit))"
    );
    assert_eq!(seen.borrow().mini, ["P: ", "P: a"]);
    // A later prompt still works: the level was restored by the unwind.
    vm.set_host_input(Some(Box::new(Script { events: vec![key('z'), RET].into(), pending: false, seen: seen.clone(), first_end: None })));
    assert_eq!(eval(&mut vm, "(read-string \"Q: \")"), "\"z\"");
}

/// Without a host keyboard VM2 stays batch GNU (minibuf.c:659).
#[test]
fn detached_host_is_batch_end_of_file() {
    let (mut vm, seen) = vm_with(vec![key('a'), RET], false);
    vm.set_host_input(None);
    assert_eq!(
        eval(&mut vm, "(condition-case e (read-from-minibuffer \"P: \") (error e))"),
        "(end-of-file \"Error reading from stdin\")"
    );
    assert_eq!(seen.borrow().reads, 0);
    assert_eq!(eval(&mut vm, "(romacs--read-host-event)"), "nil", "no host: end of input");
}

/// A keyboard macro still wins over the host keyboard (GNU reads the macro).
#[test]
fn keyboard_macro_input_is_not_taken_from_the_host() {
    let (mut vm, seen) = vm_with(vec![key('h'), RET], false);
    assert_eq!(
        eval(&mut vm, "(let ((unread-command-events (append \"mac\\r\" nil)) (executing-kbd-macro t)) (read-from-minibuffer \"P: \"))"),
        "\"mac\""
    );
    assert_eq!(seen.borrow().reads, 0);
}

/// Values the prompting form holds wait in its suspended registers. A command
/// typed at the prompt collects with those registers live (poll_resume), after
/// each host wait has handed the stack back; identity and bytes survive.
#[test]
fn values_held_across_host_waits_survive_forced_collections() {
    let churn = HostEvent::Key(CommandEvent::Character(1)); // C-a, bound below
    let (mut vm, _) = vm_with(vec![key('x'), churn.clone(), key('y'), churn, RET], false);
    vm.eval_source(
        "(defun host-input-churn () (interactive) (let ((n 0)) (while (< n 20000) (setq n (1+ n)) (cons n (make-string 3 ?q))))) \
         (define-key minibuffer-local-map \"\\C-a\" 'host-input-churn)",
    )
    .unwrap();
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let result = eval(
        &mut vm,
        "(let* ((s (concat \"held-\" (make-string 3 ?é) \"-bytes\")) (v (vector s 'sym 42)) (c (list v s))) \
           (let ((answer (read-from-minibuffer \"P: \"))) \
             (list answer (eq (car c) v) (eq (aref v 0) s) s (string-bytes s) (aref v 2))))",
    );
    let during = vm.heap_stats().collections - before;
    assert_eq!(result, "(\"xy\" t t \"held-ééé-bytes\" 17 42)");
    assert!(during >= 2, "the churn commands must collect mid-prompt ({during} collections)");
}

/// Redisplay inside the wait runs Lisp (a mode-line `:eval') in a nested
/// engine while the prompting frame is suspended, then the outer frame goes on.
#[test]
fn redisplay_lisp_runs_nested_inside_the_wait() {
    let (mut vm, seen) = vm_with(vec![key('k'), RET], false);
    vm.set_collection_floor(1);
    vm.eval_source("(defvar mode-line-format nil)").unwrap();
    vm.eval_source("(setq mode-line-format '((:eval (format \"ml-%d\" (length (list 1 2 3 4 5 6 7))))))").unwrap();
    assert_eq!(eval(&mut vm, "(let ((keep (list 1 2 3))) (list (read-from-minibuffer \"P: \") keep))"), "(\"k\" (1 2 3))");
    let seen = seen.borrow();
    assert_eq!(seen.mini, ["P: ", "P: k"]);
    assert!(seen.mode_lines.iter().all(|m| m.contains("ml-7")), "{:?} {:?}", seen.mode_lines, vm.take_display_diagnostics());
}

/// Pending input skips redisplay (keyboard.c detect_input_pending); a resize
/// redisplays at the new geometry and keeps waiting; a closed frame signals
/// `quit' through every level.
#[test]
fn pending_input_resize_and_close() {
    let (mut vm, seen) = vm_with(vec![key('p'), RET], true);
    assert_eq!(eval(&mut vm, "(read-string \"P: \")"), "\"p\"");
    assert!(seen.borrow().mini.is_empty(), "no redisplay while input is pending");

    let (mut vm, seen) = vm_with(vec![HostEvent::Resize { width: 640, height: 480 }, key('r'), RET], false);
    assert_eq!(eval(&mut vm, "(read-string \"P: \")"), "\"r\"");
    assert_eq!(seen.borrow().widths, [800, 640, 640]);

    let (mut vm, _) = vm_with(vec![key('c'), HostEvent::Closed], false);
    assert_eq!(
        eval(&mut vm, "(condition-case q (read-string \"P: \") (error (list 'error q)) (quit (list 'quit q)))"),
        "(quit (quit))"
    );
}


/// The painter's window ends are committed as GNU's redisplay does, so Lisp
/// after the prompt sees the `window-end' of the frame that was shown.
#[test]
fn layout_ends_from_the_wait_are_committed() {
    let (mut vm, seen) = vm_with(vec![], false);
    vm.eval_source("(insert \"hello world\")").unwrap();
    let before = eval(&mut vm, "(window-end)");
    vm.set_host_input(Some(Box::new(Script { events: vec![RET].into(), pending: false, seen: seen.clone(), first_end: Some(5) })));
    assert_eq!(eval(&mut vm, "(progn (read-string \"P: \") (window-end))"), "6", "window-end before the prompt was {before}");
}

/// Every call path reaches the same engine-control read: direct, `funcall'
/// and `apply' (threaded.rs routes all three through `reentrant_load').
/// A wrong arity signals instead of waiting.
#[test]
fn every_call_path_reads_through_the_reentrant_primitive() {
    let (mut vm, seen) = vm_with(vec![key('d'), key('f'), key('a')], false);
    vm.set_collection_floor(1);
    assert_eq!(
        eval(&mut vm, "(let ((keep (list 7 8))) (list (romacs--read-host-event) (funcall 'romacs--read-host-event) (apply #'romacs--read-host-event nil) keep))"),
        "(100 102 97 (7 8))"
    );
    assert_eq!(seen.borrow().reads, 3);
    assert_eq!(
        eval(&mut vm, "(condition-case e (romacs--read-host-event 1 nil 3) (error (car e)))"),
        "wrong-number-of-arguments"
    );
}

/// keyboard.c read_char: a host character goes through
/// `keyboard-translate-table' (a char-table, or a string or vector it is
/// shorter than), a nil entry meaning no translation; macro and unread
/// events do not.
#[test]
fn host_characters_go_through_keyboard_translate_table() {
    let (mut vm, _) = vm_with(vec![key('x'), key('q'), RET], false);
    assert_eq!(eval(&mut vm, "(let ((keyboard-translate-table (make-char-table 'keyboard-translate-table))) (aset keyboard-translate-table ?x ?y) (read-string \"T: \"))"), "\"yq\"");
    let (mut vm, _) = vm_with(vec![key('a'), RET], false);
    assert_eq!(eval(&mut vm, "(let ((keyboard-translate-table (make-char-table 'keyboard-translate-table))) (aset keyboard-translate-table ?a ?b) (setq unread-command-events (list ?a)) (read-string \"T: \"))"), "\"ab\"");
    let (mut vm, _) = vm_with(vec![key('x'), RET], false);
    assert_eq!(eval(&mut vm, "(let ((keyboard-translate-table \"\\0z\")) (read-string \"T: \"))"), "\"x\"");
}

/// keyboard.c read_char: a focus change arriving while Lisp reads (M-x, any
/// minibuffer prompt) runs its `special-event-map' binding at once, with the
/// event as `last-input-event', and is never returned as input; the read
/// goes on to the next key.
#[test]
fn focus_events_during_a_read_run_their_binding_and_are_not_input() {
    let (mut vm, _) = vm_with(vec![HostEvent::Focus(false), HostEvent::Focus(true), key('a'), RET], false);
    eval(&mut vm, "(progn (setq focus-log nil)
                     (defun handle-focus-in (event) (interactive \"e\")
                       (setq focus-log (cons (list (car event) (framep (car (cdr event))) (eq last-input-event event)) focus-log)))
                     (defun handle-focus-out (event) (interactive \"e\")
                       (setq focus-log (cons (list (car event) (framep (car (cdr event)))) focus-log))))");
    assert_eq!(eval(&mut vm, "(read-string \"P: \")"), "\"a\"");
    assert_eq!(eval(&mut vm, "(reverse focus-log)"), "((focus-out t) (focus-in t t))");
}
