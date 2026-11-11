//! Lisp timers and process output in the GUI's command loop, as GNU
//! keyboard.c `read_char' runs them: once the loop has no key to dispatch it
//! starts idling, handles the process output that is ready
//! (`romacs--process-idle') and runs ripe timers (`romacs--timer-check-idle',
//! the same `romacs--timer-check' the Lisp waits use), then blocks until the
//! next timer is due, a key arrives or a process is ready; a key press for a
//! command ends idleness (`timer_stop_idle').
use romacs_display_runtime::thread_comm::InputEvent;
use romacs_vm::Vm;
use std::time::Duration;

/// What one idle check did.
#[derive(Debug, PartialEq)]
pub struct Idle {
    /// A timer ran or process output was handled: redisplay, then check
    /// again before blocking.
    pub ran: bool,
    /// The echo area holds a message the display has not shown: one the
    /// timers left, here or in a wait elsewhere (`romacs--echo-shown',
    /// echo.rs).
    pub message: bool,
    /// How long to block before the next timer is due; None when none is.
    pub delay: Option<Duration>,
}

/// One idle pass: file notification events (read_char's special events),
/// process output and worker (isolate) messages, each every pass so a
/// busy connection never starves the workers, ripe timers, the idle
/// auto-save (SAVE),
/// then, on a pass where none of those ran and no message waits to be
/// shown, whether to collect (`romacs--idle-collect', lisp/romacs/vm/keyboard.el).
macro_rules! idle_form {
    ($save:literal) => {
        concat!("(let* ((notified (romacs--file-notify-idle))
                  (processes (romacs--process-idle)) (isolates (romacs--isolate-idle))
                  (output (or processes isolates notified)) (check (romacs--timer-check-idle))
                  (save ", $save, ")
                  (ran (or output (car check) (eq save t)))
                  (message (not (eq (current-message) (and (boundp 'romacs--echo-shown) romacs--echo-shown))))
                  (collect (and (not ran) (not message) (fboundp 'romacs--idle-collect) (romacs--idle-collect)))
                  (delays (delq nil (list (and (numberp save) save) (cdr check) (and (numberp collect) collect)))))
             (list ran message (and delays (ceiling (* (apply #'min delays) 1e6))) (eq collect t)))")
    };
}
const IDLE: &str = idle_form!("nil");
const IDLE_AUTO_SAVE: &str = idle_form!("(and (fboundp 'romacs--idle-auto-save) (romacs--idle-auto-save))");

/// With AUTO_SAVE, the idle auto-save (lisp/romacs/vm/keyboard.el) is checked
/// too, once the command loop's Lisp half is installed, and its delay joins
/// the timers'. A save that ran is redisplayed, as read_char's is: its
/// hooks may change buffers. The loop leaves it out while a paint it has
/// not retired is in flight, which it polls every millisecond: GNU
/// auto-saves after redisplay. An idle collection the Lisp asks for is
/// made here, after it returned: no request is running, so the request
/// code it compiled retires too. A key press arriving while it still
/// marks abandons it (`Vm::collect_garbage_until_key'): the key is read
/// at once, and the collection waits for the next quiet moment.
pub fn idle(vm: &mut Vm, auto_save: bool) -> Result<Idle, String> {
    let (ran, rest) = vm.eval_source(if auto_save { IDLE_AUTO_SAVE } else { IDLE })?.cons_pair()?;
    let ran = !ran.is_nil();
    let (message, rest) = rest.cons_pair()?;
    let message = !message.is_nil();
    let (micros, rest) = rest.cons_pair()?;
    let delay = if micros.is_nil() { None } else { Some(Duration::from_micros(micros.as_integer()?.max(0) as u64)) };
    if !rest.cons_pair()?.0.is_nil() {
        vm.collect_garbage_until_key();
    }
    Ok(Idle { ran, message, delay })
}

/// Whether EVENT is input that ends idleness: a key press that is an event
/// (not a lone modifier). Releases, resizes and focus changes are not, as GNU
/// resumes idling after such events. By reference: a tracked event's receipt
/// is never cloned.
pub fn ends_idle(event: &InputEvent) -> bool {
    match event {
        InputEvent::Tracked { event, .. } => ends_idle(event),
        &InputEvent::Key { ref key, modifiers, pressed: true, emacs_frame_id } => crate::native::translate(
            InputEvent::Key { key: key.clone(), modifiers, pressed: true, emacs_frame_id },
        ).is_some(),
        _ => false,
    }
}

/// The shorter of two optional waits; None (block) only when both are.
pub fn sooner(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GNU's timer.el, after GNU's loadup on the evaluator's stack.
    #[test]
    fn idle_runs_ordinary_then_idle_timers_as_gnus_command_loop() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(timers_body).unwrap().join().unwrap();
    }
    fn timers_body() {
        let mut vm = Vm::new();
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        // The idle collection has its own test below.
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil romacs-gc-idle-delay nil) (message nil)");
        assert_eq!(idle(&mut vm, true).unwrap(), Idle { ran: false, message: false, delay: None });
        vm.timer_stop_idle();
        // Ordinary timers: the ripe one runs once; the next delay is the far one's.
        printed(&mut vm, "(run-with-timer 0 nil (lambda () (push 'ordinary probe)))
                          (run-with-timer 60 nil #'ignore)
                          (run-with-idle-timer 0.05 nil (lambda () (push (list 'idle (and (current-idle-time) t)) probe)))");
        assert_eq!(printed(&mut vm, "(current-idle-time)"), "nil", "a command is not idle");
        let first = idle(&mut vm, true).unwrap();
        assert!(first.ran && !first.message, "{first:?}");
        assert_eq!(printed(&mut vm, "probe"), "(ordinary)");
        // Idle now: the idle timer is due 0.05s after idleness began.
        let waiting = idle(&mut vm, true).unwrap();
        assert!(!waiting.ran && waiting.delay.is_some_and(|d| d <= Duration::from_millis(50)), "{waiting:?}");
        std::thread::sleep(waiting.delay.unwrap());
        assert!(idle(&mut vm, true).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "((idle t) ordinary)", "an idle timer sees idleness");
        // The next delay is the 60 s timer's, however slow a loaded host
        // made the steps since; the idle timer due again would be 0.05 s.
        let after = idle(&mut vm, true).unwrap();
        assert!(!after.ran && after.delay.is_some_and(|d| d > Duration::from_secs(10)), "runs once per idleness: {after:?}");
        // A key ends idleness; the next idle period may run the idle timer again.
        vm.timer_stop_idle();
        assert_eq!(printed(&mut vm, "(current-idle-time)"), "nil");
        printed(&mut vm, "(setq probe nil) (run-with-idle-timer 0 t (lambda () (push 'again probe)))");
        assert!(idle(&mut vm, true).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "(again)");
        assert!(!idle(&mut vm, true).unwrap().ran, "a repeating idle timer waits for the next idleness");
        vm.timer_stop_idle();
        assert!(idle(&mut vm, true).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "(again again)");
        // An error is reported as timer-event-handler does and the loop goes on.
        vm.timer_stop_idle();
        printed(&mut vm, "(setq timer-idle-list nil) (run-with-timer 0 nil (lambda () (error \"boom\")))");
        let failed = idle(&mut vm, true).unwrap();
        assert!(failed.ran && failed.message, "{failed:?}");
        assert_eq!(printed(&mut vm, "(current-message)"), "\"Error running timer: (error \\\"boom\\\")\"");
        // read_char's own wait (lisp/romacs/vm/keyboard.el): a prompt idles and runs
        // timers until its key comes; `read-event' with a time limit runs
        // ordinary timers only and answers nil once the limit passes.
        vm.timer_stop_idle();
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_millis(80)), Step::Key('a'), Step::Key('\r')].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil)
            (run-with-timer 0.01 nil (lambda () (push 'ordinary probe)))
            (run-with-idle-timer 0.03 nil (lambda () (push (list 'idle (and (current-idle-time) t)) probe)))");
        assert_eq!(printed(&mut vm, "(list (read-string \"P: \") (reverse probe) (current-idle-time))"),
                   "(\"a\" (ordinary (idle t)) nil)");
        // A keyboard key is recorded as typed before the input method runs,
        // and a method that returns no event brings the message back.
        vm.set_host_input(Some(Box::new(Paced([Step::Key('a'), Step::Key('b')].into()))));
        printed(&mut vm, "(setq probe nil input-method-function
                  (lambda (k) (push (list k input-method-previous-message (current-message)) probe) (and (= k ?b) (list ?x))))
            (message \"before\")");
        assert_eq!(printed(&mut vm, "(prog1 (list (read-event nil t) (reverse probe) (current-message)
                                                  (let ((keys (append (recent-keys) nil))) (nthcdr (- (length keys) 2) keys)))
                                       (setq input-method-function 'list))"),
                   "(120 ((97 \"before\" nil) (98 \"before\" nil)) nil (97 98))");
        // SECONDS bound the read, however long a method that returned no
        // event took.
        vm.set_host_input(Some(Box::new(Paced([Step::Key('a'), Step::Pause(Duration::from_secs(10))].into()))));
        assert_eq!(printed(&mut vm, "(let ((start (float-time))
                                          (input-method-function (lambda (_) (sleep-for 0.3) nil)))
                                      (list (read-event nil t 0.4) (< (- (float-time) start) 0.6)))"),
                   "(nil t)");
        // A C-g while the wait's Lisp ran is the read's key: it does not
        // quit through the caller.
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        assert_eq!(printed(&mut vm, "(progn (run-with-timer 0.01 nil (lambda () (setq quit-flag t)))
                                            (read-key-sequence-vector nil))"), "[7]");
        // Nor does a quit from the wait's redisplay, in a recursive edit
        // too, which a closed frame's quit leaves first.
        let quit_in_wait = |data: &str| format!("(let ((host (symbol-function 'romacs--read-host-event))
                                                       (depth (symbol-function 'recursion-depth)))
                                                   (fset 'romacs--read-host-event (lambda (&rest _) (signal 'quit '{data})))
                                                   (fset 'recursion-depth (lambda () 1))
                                                   (unwind-protect
                                                       (catch 'exit (read-key-sequence-vector nil))
                                                     (fset 'romacs--read-host-event host)
                                                     (fset 'recursion-depth depth)))");
        assert_eq!(printed(&mut vm, &quit_in_wait("nil")), "[7]");
        assert_eq!(printed(&mut vm, &format!("(functionp {})", quit_in_wait("frame-closed"))), "t");
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq probe nil)
            (run-with-timer 0.01 nil (lambda () (push 'ordinary probe)))
            (run-with-idle-timer 0 nil (lambda () (push 'idle probe)))");
        let start = std::time::Instant::now();
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 0.05) probe (current-idle-time))"), "(nil (ordinary) nil)");
        assert!(start.elapsed() >= Duration::from_millis(50));
        // A timer's message during the read is in the echo area when the
        // wait redisplays, as GNU's `message' redisplays it at once; the key
        // that ends the read then clears it (keyboard.c read_char).
        crate::echo::clear(&mut vm).unwrap();
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_millis(80)), Step::Key('k')].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil
              romacs-echo-area-functions
              (list (lambda (text)
                      (push (list (and text (string-to-multibyte text))
                                  (string-to-multibyte (with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))))
                            probe))))
            (run-with-timer 0.01 nil (lambda () (message \"during\")))");
        assert_eq!(printed(&mut vm, "(prog1 (list (read-event) probe (current-message)) (setq romacs-echo-area-functions nil))"),
                   "(107 ((\"during\" \"during\")) nil)");
        // A timer that adds a timer: the wait checks again before blocking,
        // so the new one runs when due, not when the limit ends the block.
        // Pinned GNU 45b0f7699df, batch, stdin open: (nil (a (b t))).
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil start (float-time))
            (run-at-time 0 nil (lambda () (push 'a probe)
              (run-at-time 0.01 nil (lambda () (push (list 'b (< (- (float-time) start) 1.0)) probe)))))");
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 2) (reverse probe))"), "(nil (a (b t)))");
        // A timer that queues input wakes the read, which takes the event.
        // Pinned GNU 45b0f7699df, batch, stdin open: (97 t nil).
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil start (float-time))
            (run-at-time 0.01 nil (lambda () (push ?a unread-command-events)))");
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 3) (< (- (float-time) start) 1.5) unread-command-events)"),
                   "(97 t nil)");
        // sit-for in a timer reads a key typed during it and pushes it back
        // as (t . KEY) (subr.el); the command loop, which reads Lisp's
        // unread queues first, then runs it as the next command, though no
        // other key follows. As simple.el's M-x suggestion timer does in a
        // session (`interactive' sit-for).
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_millis(30)), Step::Key('x')].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil unread-command-events nil probe 'unset)
            (run-with-timer 0 nil (lambda () (let ((noninteractive nil)) (setq probe (sit-for 5)))))
            (set-buffer (get-buffer-create \"unread\")) (erase-buffer) (set-window-buffer nil (current-buffer))");
        vm.timer_stop_idle();
        assert!(idle(&mut vm, true).unwrap().ran);
        assert_eq!(printed(&mut vm, "(list probe unread-command-events)"), "(nil ((t . 120)))");
        assert!(crate::command::unread_pending(&mut vm).unwrap());
        let outcome = crate::command::run(&mut vm).unwrap();
        assert_eq!(outcome.command.as_deref(), Some("self-insert-command"));
        assert_eq!(printed(&mut vm, "(list (buffer-string) unread-command-events)"), "(\"x\" nil)");
        assert!(!crate::command::unread_pending(&mut vm).unwrap());
    }

    /// The session's first idle check runs an ordinary timer that outlasts
    /// an idle timer's delay, then reads with a time limit: the idle timer
    /// runs in that read, and its message is shown to the echo area's
    /// functions while the read waits, as GNU's `message' shows it at once,
    /// though nothing was shown before, input (a focus change) may be
    /// pending, and the message is gone before the command loop looks
    /// (gnosis-feed's dwell probe, read_char's wipe); a timer those
    /// functions add runs within the read; the loop then shows it cleared.
    #[test]
    fn a_message_left_in_the_first_idle_checks_wait_is_shown() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(first_wait_body).unwrap().join().unwrap();
    }
    fn first_wait_body() {
        let mut vm = Vm::new();
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        // As the session does once its Lisp is ready (main.rs).
        crate::echo::install(&mut vm).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        let painted = std::rc::Rc::new(std::cell::Cell::new(0));
        let hosts: [Box<dyn romacs_vm::HostInput>; 2] =
            [Box::new(Paced([Step::Pause(Duration::from_secs(10))].into())), Box::new(Pending(painted.clone()))];
        for host in hosts {
            vm.set_host_input(Some(host));
            printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil
                  romacs-echo-area-functions
                  (list (lambda (text)
                          (push (and text (string-to-multibyte text)) probe)
                          ;; As a toast's expiry: due long before the read's limit.
                          (when (equal text \"idle\")
                            (let ((shown (float-time)))
                              (run-with-timer 0.03 nil (lambda () (push (list 'expired (current-message) (< (- (float-time) shown) 1.5))
                                                                        probe))))))))
                (run-with-idle-timer 0.05 nil (lambda () (message \"idle\")))
                (run-with-timer 0 nil (lambda ()
                  (let ((end (+ (float-time) 0.1))) (while (< (float-time) end)))
                  (read-event nil nil 3)
                  (message nil)))");
            vm.timer_stop_idle();
            let first = idle(&mut vm, true).unwrap();
            assert!(first.ran && first.message, "{first:?}");
            assert_eq!(printed(&mut vm, "(list (reverse probe) (current-message))"),
                       "((\"idle\" (expired \"idle\" t)) nil)", "a timer the echo area's functions add runs on time");
            // The loop then shows that it was cleared.
            crate::echo::sync_current_message(&mut vm).unwrap();
            assert_eq!(printed(&mut vm, "(reverse probe)"), "(\"idle\" (expired \"idle\" t) nil)");
        }
        assert_eq!(painted.get(), 1, "with input pending, the message alone repaints");
    }

    /// A keyboard whose pending event (a focus change) never comes within a
    /// read's time limit; it counts the frames it is given.
    struct Pending(std::rc::Rc<std::cell::Cell<usize>>);

    impl romacs_vm::HostInput for Pending {
        fn input_pending(&mut self) -> bool { true }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
            self.0.set(self.0.get() + 1);
            Ok(vec![])
        }
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<romacs_vm::HostEvent, String> {
            std::thread::sleep(timeout.ok_or("an untimed read")?);
            Ok(romacs_vm::HostEvent::Timeout)
        }
    }

    /// The idle collection (lisp/romacs/vm/keyboard.el): nothing is scheduled
    /// until enough was allocated; then one collection once the session has
    /// been idle for the delay, on a pass where nothing else ran, made by
    /// the host outside any request.
    /// A collection at a command's end, after `romacs--command-loop-top'
    /// cleared the running command, still names it.
    #[test]
    fn a_collection_at_a_commands_end_names_the_command() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(command_end_body).unwrap().join().unwrap();
    }
    fn command_end_body() {
        let mut vm = Vm::new();
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        // The command loop's Lisp half, as a session's first key installs it.
        vm.eval_source("(read-event nil nil 0)").unwrap();
        vm.eval_source("(defun collected-command () (interactive)) (global-set-key \"q\" 'collected-command)").unwrap();
        vm.timer_stop_idle();
        vm.eval_source("(setq unread-command-events (list ?q)) (romacs--run-host-command) (romacs--take-collections)").unwrap();
        // Where a request boundary collects: the loop's top has run.
        assert_eq!(vm.eval_source("real-this-command").unwrap().display().unwrap(), "nil");
        vm.collect_garbage();
        let taken = vm.eval_source("(romacs--take-collections)").unwrap().display().unwrap();
        assert!(taken.contains("command \"collected-command\""), "{taken}");
    }

    #[test]
    fn idle_collects_after_its_delay_once_enough_was_allocated() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(idle_collect_body).unwrap().join().unwrap();
    }
    fn idle_collect_body() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        // The command loop's Lisp half, as a session's first key installs it.
        printed(&mut vm, "(read-event nil nil 0)");
        // A trigger of a million objects, 16 bytes each: the idle collection
        // waits for a tenth of that.
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil gc-cons-threshold 16000000 gc-cons-percentage nil
                                romacs-gc-idle-delay 0.05 romacs-gc-idle-factor 10)
                          (message nil)");
        vm.collect_garbage();
        vm.timer_stop_idle();
        let quiet = idle(&mut vm, true).unwrap();
        assert_eq!(quiet, Idle { ran: false, message: false, delay: None }, "nothing to collect, no wake-up");
        let done = printed(&mut vm, "(progn (dotimes (_ 20000) (make-list 10 nil)) gcs-done)");
        vm.timer_stop_idle();
        let waiting = idle(&mut vm, true).unwrap();
        assert!(!waiting.ran && waiting.delay.is_some_and(|d| d <= Duration::from_millis(50)), "{waiting:?}");
        assert_eq!(printed(&mut vm, "gcs-done"), done, "not before the delay");
        std::thread::sleep(waiting.delay.unwrap());
        let collected = idle(&mut vm, true).unwrap();
        assert_eq!(collected, Idle { ran: false, message: false, delay: None }, "a collection is not a redisplay");
        let after: i64 = printed(&mut vm, "gcs-done").parse().unwrap();
        assert_eq!(after, done.parse::<i64>().unwrap() + 1, "one idle collection");
        assert_eq!(printed(&mut vm, "(< (romacs--collection-share) 0.01)"), "t", "made at a request boundary: request code retired");
        // Off: nil never collects at idle.
        printed(&mut vm, "(progn (setq romacs-gc-idle-delay nil) (dotimes (_ 20000) (make-list 10 nil)))");
        vm.timer_stop_idle();
        assert_eq!(idle(&mut vm, true).unwrap().delay, None);
    }

    /// A keyboard that is idle for WAITS waits, each the time asked for or
    /// 20 ms at most, then gives KEY.
    struct Idling(usize, Option<romacs_vm::CommandEvent>);
    impl romacs_vm::HostInput for Idling {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<romacs_vm::HostEvent, String> {
            if self.0 == 0 {
                return self.1.take().map(romacs_vm::HostEvent::Key).ok_or_else(|| "keys exhausted".into());
            }
            self.0 -= 1;
            std::thread::sleep(timeout.unwrap_or(Duration::MAX).min(Duration::from_millis(20)));
            Ok(romacs_vm::HostEvent::Timeout)
        }
    }

    /// A prompt's wait for a key (a minibuffer read) collects at idle as
    /// the host's idle pass does: once idle for the delay with enough
    /// allocated, one collection, and the key it waited for is read. Only
    /// the objects allocated count there: request code, which a collection
    /// inside Lisp cannot retire, would have it collect again and again.
    #[test]
    fn a_prompt_wait_collects_at_idle() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(prompt_collect_body).unwrap().join().unwrap();
    }
    fn prompt_collect_body() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        vm.eval_source(include_str!("../../../lisp/romacs/vm/process.el")).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        // The command loop's Lisp half, as a session's first key installs it.
        printed(&mut vm, "(read-event nil nil 0)");
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil gc-cons-threshold 16000000 gc-cons-percentage nil
                                romacs-gc-idle-delay 0.05 romacs-gc-idle-factor 10)");
        vm.collect_garbage();
        // Closures that count toward a code collection: past the idle
        // share, short of an automatic one.
        printed(&mut vm, "(dotimes (i 500) (eval (list 'function (list 'lambda nil i)) t))");
        assert_eq!(printed(&mut vm, "(> (romacs--collection-share) 0.2)"), "t");
        let done: i64 = printed(&mut vm, "(progn (dotimes (_ 20000) (make-list 10 nil)) gcs-done)").parse().unwrap();
        vm.set_host_input(Some(Box::new(Idling(10, Some(romacs_vm::CommandEvent::Character(120))))));
        assert_eq!(printed(&mut vm, "(list (romacs--read-host-char nil) gcs-done)"), format!("(120 {})", done + 1));
        // Under the delay, none.
        printed(&mut vm, "(progn (setq romacs-gc-idle-delay 5) (dotimes (_ 20000) (make-list 10 nil)))");
        vm.set_host_input(Some(Box::new(Idling(3, Some(romacs_vm::CommandEvent::Character(121))))));
        assert_eq!(printed(&mut vm, "(list (romacs--read-host-char nil) gcs-done)"), format!("(121 {})", done + 1));
    }

    /// Work that runs during a command-key wait and schedules a timer,
    /// a background command's callback or a mode-line :eval in the wait's
    /// redisplay, counts toward the wait's deadline: the timer fires on time
    /// while C-x waits for its next key, before C-x h runs.
    #[test]
    fn timers_scheduled_during_a_key_wait_fire_on_time() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(scheduled_body).unwrap().join().unwrap();
    }
    fn scheduled_body() {
        let mut vm = Vm::new();
        vm.set_echo_area_host(true);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        vm.eval_source(include_str!("../../../lisp/romacs/vm/process.el")).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        let schedule = "(run-with-timer 0.2 nil (lambda () (setq log (cons (list 'timer (- (float-time) start)) log))))";
        // A background command completes while C-x waits (it outlasts the
        // start of that wait); its callback schedules the timer.
        printed(&mut vm, &format!("(setq timer-list nil timer-idle-list nil log nil start nil)
            (set-buffer (get-buffer-create \"keys\")) (set-window-buffer nil (current-buffer))
            (romacs-process-start (list \"sleep\" \"0.3\") \"\" default-directory 10
              (lambda (_) (setq start (float-time) log (cons 'callback log)) {schedule} nil))"));
        vm.set_host_input(Some(Box::new(Paced([Step::Key('\u{18}'), Step::Completed(Duration::from_millis(800)),
            Step::Pause(Duration::from_secs(2)), Step::Key('h')].into()))));
        vm.timer_stop_idle();
        assert_eq!(crate::command::run(&mut vm).unwrap().command.as_deref(), Some("mark-whole-buffer"));
        let fired = printed(&mut vm, "(list (reverse (mapcar (lambda (e) (if (consp e) (car e) e)) log)) (let ((timer (assq 'timer log))) (and timer (< (cadr timer) 1.8))))");
        assert_eq!(fired, "((callback timer) t)", "a callback's timer");
        // A mode-line :eval schedules it in the redisplay of the wait for
        // C-x's next key.
        printed(&mut vm, &format!("(setq timer-list nil log nil start nil)
            (setq mode-line-format (list (list :eval (quote (progn (when (and (null start) (equal (this-command-keys-vector) [24]))
              (setq start (float-time) log (cons 'eval log)) {schedule}) \"\")))))"));
        vm.set_host_input(Some(Box::new(Paced([Step::Key('\u{18}'), Step::Pause(Duration::from_secs(2)), Step::Key('h')].into()))));
        assert_eq!(crate::command::run(&mut vm).unwrap().command.as_deref(), Some("mark-whole-buffer"));
        let fired = printed(&mut vm, "(list (reverse (mapcar (lambda (e) (if (consp e) (car e) e)) log)) (let ((timer (assq 'timer log))) (and timer (< (cadr timer) 1.8))))");
        assert_eq!(fired, "((eval timer) t)", "a redisplay's timer");
    }

    /// A timer that reschedules itself at once runs once per round of a
    /// wait, which reads a queued key and ends at its time limit between
    /// rounds. Pinned GNU 45b0f7699df, -nw: the key typed 0.3s in is read
    /// then; its time limit waits for the rescheduling to stop
    /// (DIVERGENCES.md VM2-KBD1). A starved wait ends only when the
    /// rescheduling does, after 10 s; half that bounds a loaded host's
    /// slowness.
    #[test]
    fn a_rescheduling_timer_never_starves_a_wait() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(rescheduling_body).unwrap().join().unwrap();
    }
    fn rescheduling_body() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        let rescheduling = "(setq timer-list nil timer-idle-list nil unread-command-events nil runs 0 start (float-time))
            (defun rescheduling () (setq runs (1+ runs)) (when (< (- (float-time) start) 10) (run-with-timer 0 nil #'rescheduling)))
            (run-with-timer 0 nil #'rescheduling)";
        vm.set_host_input(Some(Box::new(Paced([Step::Key('a')].into()))));
        printed(&mut vm, rescheduling);
        let key = printed(&mut vm, "(list (read-event) (< (- (float-time) start) 5) (<= runs 2))");
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, rescheduling);
        let limit = printed(&mut vm, "(list (read-event nil nil 0.1) (< (- (float-time) start) 5))");
        assert_eq!((key.as_str(), limit.as_str()), ("(97 t t)", "(nil t)"), "a queued key, then the time limit");
    }

    /// Process output reaches its filter while the session waits for a key,
    /// with no Lisp wait: `read-event' with a time limit runs the filter
    /// of a :nowait connection's data as it arrives, as pinned GNU's
    /// read_char does (batch, stdin open), and the command loop's idle wait
    /// wakes for the process, not its time limit, then runs its filter.
    #[test]
    fn process_output_runs_while_waiting_for_a_key() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(process_body).unwrap().join().unwrap();
    }
    /// A local server: each connection gets "hi" after DELAY, then stays open.
    fn serve_after(delay: Duration) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                use std::io::Write;
                std::thread::sleep(delay);
                let mut stream = stream;
                let _ = stream.write_all(b"hi");
                std::thread::sleep(Duration::from_secs(30));
            });
        });
        port
    }
    fn process_body() {
        let port = serve_after(Duration::from_millis(200));
        let form = format!("(let ((got nil) (states nil) (start (float-time)))
            (make-network-process :name \"p\" :host \"127.0.0.1\" :service {port} :nowait t
              :filter (lambda (_ s) (push s got)) :sentinel (lambda (_ e) (push (car (split-string e)) states)))
            (list (read-event nil nil 1) (reverse states) got (< (- (float-time) start) 3)))");
        let gnu = std::process::Command::new(std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle"))
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")])
            .stdin(std::process::Stdio::piped()).output().unwrap();
        let expected = String::from_utf8(gnu.stdout).unwrap();
        assert_eq!(expected, "(nil (\"open\") (\"hi\") t)", "pinned GNU");
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        vm.set_host_input(Some(Box::new(Waiting(vec![]))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil)");
        assert_eq!(printed(&mut vm, &form), expected, "read-event's wait");
        // The command loop: idle, block on the process, idle again.
        printed(&mut vm, &format!("(setq got nil states nil)
            (make-network-process :name \"q\" :host \"127.0.0.1\" :service {port} :nowait t
              :filter (lambda (_ s) (push s got)) :sentinel (lambda (_ e) (push (car (split-string e)) states)))"));
        vm.timer_stop_idle();
        let (_keys, input) = crossbeam_channel::bounded::<InputEvent>(1);
        let (_terminals, terminal) = crossbeam_channel::bounded::<()>(1);
        let mut woke = vec![];
        for _ in 0..4 {
            if idle(&mut vm, true).unwrap().ran { continue; }
            if printed(&mut vm, "got") != "nil" { break; }
            let processes = vm.process_wait_fds();
            woke.push(matches!(crate::terminal_wait::wait(&input, &terminal, Some(Duration::from_secs(5)), &processes),
                               crate::terminal_wait::Wake::Process));
        }
        assert_eq!(printed(&mut vm, "(list (reverse states) got)"), "((\"open\") (\"hi\"))");
        assert!(!woke.is_empty() && woke.iter().all(|&process| process), "the wait woke for the process: {woke:?}");
    }

    /// A process whose output never stops arriving starves neither the
    /// keyboard nor the timers: wait_reading_process_output, called by
    /// read_char, returns when keyboard input or a timer is due, so each
    /// idle pass reads what is ready once and the wait goes on to the
    /// timers, the keys and redisplay.  A 50 ms timer fires and a key
    /// pending after 300 ms is read while the output streams; the command
    /// loop's idle step returns at once.
    #[test]
    fn a_streaming_process_starves_neither_keys_nor_timers() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(streaming_body).unwrap().join().unwrap();
    }
    fn streaming_body() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil bytes 0)");
        // Output for 5 s, faster than its filter, which works on each
        // chunk as a parser does, takes it: the pipe is never empty.
        printed(&mut vm, "(make-process :name \"stream\" :command '(\"timeout\" \"5\" \"yes\" \"0123456789abcdef\")
                            :filter (lambda (_ s) (dotimes (_ 3000) (setq bytes (+ bytes 0))) (setq bytes (+ bytes (length s)))))");
        std::thread::sleep(Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(200));
        vm.set_host_input(Some(Box::new(KeyAfter(std::time::Instant::now() + Duration::from_millis(300), vec![]))));
        let read = printed(&mut vm, "(let ((start (float-time)) fired)
            (run-with-timer 0.05 nil (lambda () (setq fired (- (float-time) start))))
            (let ((key (read-event)))
              (list key (and fired t) (< (- (float-time) start) 3.0) (> bytes 0)
                    (eq (process-status \"stream\") 'run))))");
        assert_eq!(read, "(97 t t t t)", "read-event's wait: (key, timer on time, key on time, output read, still streaming)");
        vm.set_host_input(None);
        vm.timer_stop_idle();
        let (start, before) = (std::time::Instant::now(), printed(&mut vm, "bytes"));
        assert!(idle(&mut vm, true).unwrap().ran, "the idle step read the output");
        assert!(start.elapsed() < Duration::from_millis(500), "the idle step returned after {:?}", start.elapsed());
        assert_ne!(printed(&mut vm, "bytes"), before);
        printed(&mut vm, "(delete-process \"stream\")");
    }

    /// A keyboard whose one key arrives at AT; its waits watch the process
    /// descriptors meanwhile, as the GUI's prompt host does.
    struct KeyAfter(std::time::Instant, Vec<(std::os::fd::RawFd, i16)>);

    impl romacs_vm::HostInput for KeyAfter {
        fn input_pending(&mut self) -> bool { std::time::Instant::now() >= self.0 }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn watch_processes(&mut self, fds: &[(std::os::fd::RawFd, i16)]) { self.1 = fds.to_vec(); }
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<romacs_vm::HostEvent, String> {
            let left = self.0.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() { return Ok(romacs_vm::HostEvent::Key(romacs_vm::CommandEvent::Character('a' as i64))); }
            let (_keys, input) = crossbeam_channel::bounded::<InputEvent>(1);
            let (_terminals, terminal) = crossbeam_channel::bounded::<()>(1);
            let wait = timeout.map_or(left, |timeout| timeout.min(left));
            Ok(match crate::terminal_wait::wait(&input, &terminal, Some(wait), &std::mem::take(&mut self.1)) {
                crate::terminal_wait::Wake::Process => romacs_vm::HostEvent::Process,
                _ => romacs_vm::HostEvent::Timeout,
            })
        }
    }

    /// A keyboard nobody types on whose waits watch the process
    /// descriptors, as the GUI's prompt host does.
    struct Waiting(Vec<(std::os::fd::RawFd, i16)>);

    impl romacs_vm::HostInput for Waiting {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn watch_processes(&mut self, fds: &[(std::os::fd::RawFd, i16)]) { self.0 = fds.to_vec(); }
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<romacs_vm::HostEvent, String> {
            let (_keys, input) = crossbeam_channel::bounded::<InputEvent>(1);
            let (_terminals, terminal) = crossbeam_channel::bounded::<()>(1);
            Ok(match crate::terminal_wait::wait(&input, &terminal, timeout, &std::mem::take(&mut self.0)) {
                crate::terminal_wait::Wake::Process => romacs_vm::HostEvent::Process,
                _ => romacs_vm::HostEvent::Timeout,
            })
        }
    }

    /// A pause, a key, or the wake a completed background command gives
    /// after the time it takes to complete.
    enum Step { Pause(Duration), Key(char), Completed(Duration) }

    /// A keyboard whose keys come after pauses, honouring each read's time limit.
    struct Paced(std::collections::VecDeque<Step>);

    impl romacs_vm::HostInput for Paced {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<romacs_vm::HostEvent, String> {
            match self.0.pop_front().ok_or("paced keys exhausted")? {
                Step::Key(c) => Ok(romacs_vm::HostEvent::Key(romacs_vm::CommandEvent::Character(c as i64))),
                Step::Completed(after) => { std::thread::sleep(after); Ok(romacs_vm::HostEvent::Background) }
                Step::Pause(left) => {
                    let wait = timeout.map_or(left, |timeout| timeout.min(left));
                    std::thread::sleep(wait);
                    if wait < left { self.0.push_front(Step::Pause(left - wait)); }
                    if timeout.is_some_and(|timeout| timeout <= left) { Ok(romacs_vm::HostEvent::Timeout) }
                    else { self.next_event(timeout.map(|timeout| timeout - wait)) }
                }
            }
        }
    }

    #[test]
    fn only_key_presses_end_idleness() {
        use romacs_display_runtime::host_types::FrontendKey;
        let key = |pressed| InputEvent::Key { key: FrontendKey::Character('a'), modifiers: 0, pressed, emacs_frame_id: 0 };
        assert!(ends_idle(&key(true)));
        assert!(!ends_idle(&key(false)));
        let shift = InputEvent::Key { key: FrontendKey::Keysym(0xffe1), modifiers: 0, pressed: true, emacs_frame_id: 0 };
        assert!(!ends_idle(&shift), "a lone modifier is no event");
        assert!(!ends_idle(&InputEvent::WindowClose { emacs_frame_id: 0 }));
    }
}
