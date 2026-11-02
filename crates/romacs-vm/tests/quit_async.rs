//! A host's asynchronous C-g (romacs_vm::QuitSignal): GNU keyboard.c
//! handle_interrupt sets `quit-flag' out of band and the evaluator's
//! maybe_quit acts on it at its safepoints, honouring `inhibit-quit' and
//! `throw-on-input'. Each press here comes from another thread, as the GUI's
//! render thread sends it, and must end the evaluation within 100 ms.
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, QuitSignal, Vm, WindowEnd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A VM with the Lisp prelude, busy running a command, and the signal its
/// host's input thread holds.
fn busy_vm() -> (Vm, Arc<QuitSignal>) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let signal = Arc::new(QuitSignal::new());
    vm.share_quit_signal(signal.clone());
    vm.enter_busy();
    (vm, signal)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// Press C-g PRESSES times, AFTER from now and then every 50 ms; the
/// moment of the last press.
fn press(signal: &Arc<QuitSignal>, after: Duration, presses: usize) -> std::thread::JoinHandle<Instant> {
    let signal = signal.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        for n in 0..presses {
            if n > 0 { std::thread::sleep(Duration::from_millis(50)); }
            assert!(signal.interrupt(), "a busy evaluator takes the press");
        }
        Instant::now()
    })
}

/// SOURCE's printed value, C-g pressed 200 ms into it; asserts that the
/// evaluation ended within 100 ms of the press.
fn quit_into(vm: &mut Vm, signal: &Arc<QuitSignal>, source: &str) -> String {
    let presser = press(signal, Duration::from_millis(200), 1);
    let value = eval(vm, source);
    let ended = Instant::now();
    let pressed = presser.join().unwrap();
    let latency = ended.saturating_duration_since(pressed);
    assert!(latency < Duration::from_millis(100), "{source}: ended {latency:?} after C-g");
    value
}

/// SOURCE's printed value, C-g pressed once MARKER exists (written by a
/// form SOURCE evaluates), so the quit always lands after work has run
/// however long reading took; asserts the evaluation ended within 100 ms of
/// the press.
fn quit_after_marker(vm: &mut Vm, signal: &Arc<QuitSignal>, source: &str, marker: &std::path::Path) -> String {
    let (signal_, marker_) = (signal.clone(), marker.to_path_buf());
    let presser = std::thread::spawn(move || {
        let start = Instant::now();
        while !marker_.exists() {
            assert!(start.elapsed() < Duration::from_secs(60), "the first form never ran");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(signal_.interrupt(), "a busy evaluator takes the press");
        Instant::now()
    });
    let value = eval(vm, source);
    let ended = Instant::now();
    let pressed = presser.join().unwrap();
    let latency = ended.saturating_duration_since(pressed);
    assert!(latency < Duration::from_millis(100), "{source}: ended {latency:?} after C-g");
    value
}

#[test]
fn a_tight_loop_and_a_calling_loop_quit() {
    let (mut vm, signal) = busy_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (while t) (quit 'quit))"), "quit");
    eval(&mut vm, "(defun quit-async-f (x) (1+ x))");
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (let ((n 0)) (while t (setq n (quit-async-f n)))) (quit 'quit))"), "quit");
    // The signal is consumed: nothing is pending afterwards.
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(nil nil)");
    assert_eq!(eval(&mut vm, "(let ((n 0)) (while (< n 100000) (setq n (1+ n))) n)"), "100000");
}

#[test]
fn a_pathological_regexp_quits() {
    let (mut vm, signal) = busy_vm();
    // Exponential backtracking through a back reference.
    let pattern = r#""\\(a*\\)*\\1b""#;
    assert_eq!(quit_into(&mut vm, &signal, &format!("(condition-case nil (string-match {pattern} (make-string 40 ?a)) (quit 'quit))")), "quit");
    assert_eq!(quit_into(&mut vm, &signal, &format!(
        "(with-temp-buffer (insert (make-string 40 ?a)) (goto-char (point-min)) (condition-case nil (re-search-forward {pattern} nil t) (quit 'quit)))")), "quit");
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// Quitting out of `eval-buffer' and `load' keeps the forms evaluated
/// before the quit, as GNU's readevalloop does.
#[test]
fn eval_buffer_and_load_quit_between_forms_keeping_what_ran() {
    let (mut vm, signal) = busy_vm();
    // Forms that reach no safepoint of their own: a primitive's work, then
    // a count. Only the poll between forms can end the load.
    const FORMS: usize = 200_000;
    let marker = std::env::temp_dir().join(format!("quit-async-{}.ran", std::process::id()));
    // The eleventh form leaves MARKER, so C-g comes after ten counted forms
    // ran even when reading the file is slow under load.
    let form = "(progn (make-string 30000 ?a) (setq quit-async-n (1+ quit-async-n)))\n";
    let text = form.repeat(10) + &format!("(write-region \"\" nil {:?})\n", marker.display().to_string())
        + &form.repeat(FORMS - 10);
    let file = std::env::temp_dir().join(format!("quit-async-{}.el", std::process::id()));
    std::fs::write(&file, &text).unwrap();
    eval(&mut vm, &format!("(progn (defvar quit-async-n 0) (set-buffer (get-buffer-create \"quit-async\")) (insert-file-contents {:?}) t)", file.display().to_string()));
    let quit = quit_after_marker(&mut vm, &signal, "(condition-case nil (progn (eval-buffer) 'done) (quit 'quit))", &marker);
    std::fs::remove_file(&marker).unwrap();
    let n: usize = eval(&mut vm, "quit-async-n").parse().unwrap();
    assert_eq!(quit, "quit");
    assert!(n > 0 && n < FORMS, "eval-buffer ran {n} forms");
    eval(&mut vm, "(setq quit-async-n 0)");
    let quit = quit_after_marker(&mut vm, &signal, &format!("(condition-case nil (progn (load {:?} nil t t) 'done) (quit 'quit))", file.display().to_string()), &marker);
    let n: usize = eval(&mut vm, "quit-async-n").parse().unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::remove_file(&marker).unwrap();
    assert_eq!(quit, "quit");
    assert!(n > 0 && n < FORMS, "load ran {n} forms");
}

// A macro whose expansion never ends quits like any loop, to the handler
// around the expansion: in `eval' at the call's reach, inside a function
// `eval' defined, through `macroexpand', and during `load''s expansion of
// a top-level form (a quit is no error, so no "Eager macro-expansion
// failure" wraps it).
fn endless_macro_vm() -> (Vm, Arc<QuitSignal>) {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, "(defmacro quit-async-endless () (while t) nil)");
    (vm, signal)
}

#[test]
fn an_endless_expansion_in_eval_quits_to_its_handler() {
    let (mut vm, signal) = endless_macro_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(eval '(condition-case nil (quit-async-endless) (quit 'quit)) t)"), "quit");
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(nil nil)");
}

#[test]
fn an_endless_expansion_in_a_function_eval_defined_quits() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(eval '(defun quit-async-g () (quit-async-endless)) t)");
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (quit-async-g) (quit 'quit))"), "quit");
}

#[test]
fn an_endless_macroexpand_quits() {
    let (mut vm, signal) = endless_macro_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (macroexpand '(quit-async-endless)) (quit 'quit))"), "quit");
}

#[test]
fn an_endless_expansion_during_load_quits_unwrapped() {
    let (mut vm, signal) = endless_macro_vm();
    let file = std::env::temp_dir().join(format!("quit-async-macro-{}.el", std::process::id()));
    std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(quit-async-endless)\n").unwrap();
    let load = format!("(condition-case nil (progn (load {:?} nil t t) 'done) (quit 'quit))", file.display().to_string());
    let value = quit_into(&mut vm, &signal, &load);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(value, "quit");
}

/// The escape that leaves a quit expansion is raised at the form's reach,
/// whatever the quit became on its way out, and the macro does not run
/// again: here the macro's cleanup replaces the quit with its own error,
/// as GNU's unwind-protect lets it.
#[test]
fn a_quit_expansion_raises_what_left_it() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(defmacro quit-async-cleanup () (unwind-protect (while t) (error \"cleanup\")) nil)");
    let form = "(eval '(condition-case e (quit-async-cleanup) (error (list 'error e)) (quit 'quit)) t)";
    assert_eq!(quit_into(&mut vm, &signal, form), r#"(error (error "cleanup"))"#);
}

/// A quit taken by an `eval' nested in a macro's expansion leaves the
/// outer expansion too: one C-g, and the outer macro does not run again.
#[test]
fn a_quit_in_a_nested_eval_leaves_the_outer_expansion() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(defmacro quit-async-outer () (eval '(quit-async-endless) t) nil)");
    assert_eq!(quit_into(&mut vm, &signal, "(eval '(condition-case nil (quit-async-outer) (quit 'quit)) t)"), "quit");
}

#[test]
fn inhibit_quit_holds_the_quit_until_it_is_unbound() {
    let (mut vm, signal) = busy_vm();
    let presser = press(&signal, Duration::from_millis(100), 1);
    let started = Instant::now();
    let value = eval(&mut vm, "(condition-case nil
                                 (progn (let ((inhibit-quit t) (end (+ (float-time) 0.4)))
                                          (while (< (float-time) end))
                                          (setq quit-async-held quit-flag))
                                        (while t))
                               (quit (list 'quit quit-async-held)))");
    presser.join().unwrap();
    assert_eq!(value, "(quit t)", "held in `quit-flag' while inhibited, then quit");
    assert!(started.elapsed() >= Duration::from_millis(400));
}

/// GNU's forced quit for a graphical frame: the third C-g while the first
/// is still pending clears `inhibit-quit'.
#[test]
fn a_third_press_forces_the_quit() {
    let (mut vm, signal) = busy_vm();
    let presser = press(&signal, Duration::from_millis(100), 3);
    let value = eval(&mut vm, "(condition-case nil (let ((inhibit-quit t)) (while t)) (quit 'forced))");
    let pressed = presser.join().unwrap();
    assert_eq!(value, "forced");
    assert!(pressed.elapsed() < Duration::from_millis(100));
}

/// A host keyboard whose input arrives FROM_NOW + 200 ms and holds C-g.
struct Arriving(Arc<AtomicBool>, Vec<i64>);
impl HostInput for Arriving {
    fn input_pending(&mut self) -> bool { self.0.load(Ordering::SeqCst) || !self.1.is_empty() }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> { Ok(Vec::new()) }
    fn next_event(&mut self, _: Option<Duration>) -> Result<HostEvent, String> {
        Ok(match self.1.pop() { Some(c) => HostEvent::Key(CommandEvent::Character(c)), None => HostEvent::Timeout })
    }
}

/// `while-no-input''s mechanism: input arriving while `throw-on-input' is
/// bound sets `quit-flag' to its tag, and the next safepoint throws there.
#[test]
fn input_throws_to_throw_on_input() {
    let (mut vm, _signal) = busy_vm();
    let arrived = Arc::new(AtomicBool::new(false));
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_host_input(Some(Box::new(Arriving(arrived.clone(), Vec::new()))));
    let flag = arrived.clone();
    let typist = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(200)); flag.store(true, Ordering::SeqCst); Instant::now() });
    let value = eval(&mut vm, "(catch 'quit-async-input (let ((throw-on-input 'quit-async-input)) (while t)))");
    let typed = typist.join().unwrap();
    assert_eq!(value, "t");
    assert!(typed.elapsed() < Duration::from_millis(100));
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// read_char's quit: a C-g read from the keyboard quits `read-event'
/// unless `inhibit-quit' is non-nil, when it is the event; a press that
/// came while the evaluator was busy is the next read's C-g; a key
/// sequence reads it as a key and clears `quit-flag'.
#[test]
fn a_keyboard_read_takes_c_g_as_read_char_does() {
    let (mut vm, signal) = busy_vm();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let keys = |vm: &mut Vm, keys: Vec<i64>| vm.set_host_input(Some(Box::new(Arriving(Arc::new(AtomicBool::new(false)), keys))));
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(condition-case nil (read-event) (quit 'quit))"), "quit");
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t)) (list (read-event) quit-flag))"), "(7 nil)");
    keys(&mut vm, vec![]);
    let presser = press(&signal, Duration::from_millis(100), 1);
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t) (end (+ (float-time) 0.3))) (while (< (float-time) end)) (list (read-event) quit-flag))"),
               "(7 nil)", "a press held by `inhibit-quit' is the next read's C-g");
    presser.join().unwrap();
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(list (read-key-sequence-vector nil) quit-flag)"), "([7] nil)");
    // An unread C-g is no keyboard quit.
    assert_eq!(eval(&mut vm, "(let ((unread-command-events (list 7))) (read-event))"), "7");
}

/// The input side counts a press only while the evaluator is busy; a
/// waiting evaluator reads its C-g as a key.
#[test]
fn a_waiting_evaluator_reads_c_g_itself() {
    let (mut vm, signal) = busy_vm();
    vm.leave_busy();
    assert!(!signal.interrupt());
    assert!(!vm.take_quit_key());
    vm.enter_busy();
    assert!(signal.interrupt());
    vm.leave_busy();
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(t t)", "untaken, it is pending input");
    assert!(vm.take_quit_key(), "the host reads it as the next key");
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// `quit-flag' set by Lisp, as GNU's eval.c and subr.el's with-local-quit
/// use it (the same forms run against pinned GNU in the lane's oracle).
#[test]
fn lisp_quit_flag_follows_maybe_quit() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(eval(&mut vm, "(condition-case nil (progn (setq quit-flag t) (while t)) (quit 'quit))"), "quit");
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t))
                                (prog1 (list (condition-case nil
                                                 (let ((inhibit-quit nil)) (setq quit-flag t) (while t) 'not)
                                               (quit (setq quit-flag t) (eval '(progn nil) t) nil))
                                             quit-flag)
                                  (setq quit-flag nil)))"), "(nil t)");
    assert_eq!(eval(&mut vm, "(catch 'quit-async-x (let ((throw-on-input 'quit-async-x)) (setq quit-flag 'quit-async-x) (while t)))"), "t");
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t)) (setq quit-flag t) (prog1 (input-pending-p) (setq quit-flag nil)))"), "t");
}

/// GNU's eval_sub and funcall run maybe_quit before the body: a quit due at
/// an engine's entry ends it before straight-line code runs.
#[test]
fn a_due_quit_ends_an_entry_before_its_first_instruction() {
    let (mut vm, signal) = busy_vm();
    assert!(signal.interrupt());
    assert!(eval(&mut vm, "(setq quit-async-side 1)").starts_with("ERROR"), "quit before the setq");
    assert_eq!(eval(&mut vm, "(boundp 'quit-async-side)"), "nil");
    assert_eq!(eval(&mut vm, "(list (condition-case nil (progn (setq quit-flag t) (eval '(setq quit-async-side 3) t)) (quit 'quit)) (boundp 'quit-async-side))"), "(quit nil)");
}

/// A keyboard read leaves the evaluator busy while it redisplays (its Lisp
/// stays quittable, as GNU's is) and marks it waiting only while it blocks.
#[test]
fn a_read_is_waiting_only_while_it_blocks() {
    struct Watching(Arc<QuitSignal>, Vec<(&'static str, bool)>, Arc<std::sync::Mutex<Vec<(&'static str, bool)>>>);
    impl HostInput for Watching {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> {
            self.2.lock().unwrap().push(("redisplay", self.0.busy_span().is_some()));
            Ok(Vec::new())
        }
        fn next_event(&mut self, _: Option<Duration>) -> Result<HostEvent, String> {
            self.2.lock().unwrap().push(("wait", self.0.busy_span().is_some()));
            Ok(HostEvent::Key(CommandEvent::Character(120)))
        }
    }
    let (mut vm, signal) = busy_vm();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    vm.set_host_input(Some(Box::new(Watching(signal.clone(), Vec::new(), seen.clone()))));
    assert_eq!(eval(&mut vm, "(read-event)"), "120");
    assert_eq!(*seen.lock().unwrap(), [("redisplay", true), ("wait", false)]);
    assert!(signal.busy_span().is_some(), "busy again after the read");
}
