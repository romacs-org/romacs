//! Lisp timers in the GUI's command loop, as GNU keyboard.c `read_char' runs
//! them: once the loop has no key to dispatch it starts idling and runs ripe
//! timers (`romacs--timer-check-idle', the same `romacs--timer-check' the Lisp
//! waits use), then blocks until the next one is due; a key press for a
//! command ends idleness (`timer_stop_idle').
use romacs_display_runtime::thread_comm::InputEvent;
use romacs_vm::Vm;
use std::time::Duration;

/// What one idle check did.
#[derive(Debug, PartialEq)]
pub struct Idle {
    /// A timer ran: redisplay, then check again before blocking.
    pub ran: bool,
    /// The timers left a new message for the echo area.
    pub message: bool,
    /// How long to block before the next timer is due; None when none is.
    pub delay: Option<Duration>,
}

pub fn idle(vm: &mut Vm) -> Result<Idle, String> {
    let (ran, rest) = vm.eval_source(
        "(let* ((message (current-message)) (check (romacs--timer-check-idle)))
           (list (car check) (not (eq message (current-message)))
                 (and (cdr check) (ceiling (* (cdr check) 1e6)))))",
    )?.cons_pair()?;
    let ran = !ran.is_nil();
    let (message, rest) = rest.cons_pair()?;
    let message = !message.is_nil();
    let micros = rest.cons_pair()?.0;
    let delay = if micros.is_nil() { None } else { Some(Duration::from_micros(micros.as_integer()?.max(0) as u64)) };
    Ok(Idle { ran, message, delay })
}

/// Whether EVENT is input that ends idleness: a key press that is an event
/// (not a lone modifier). Releases, resizes and focus changes are not, as GNU
/// resumes idling after such events. By reference: a tracked event's receipt
/// is never cloned.
pub fn ends_idle(event: &InputEvent) -> bool {
    match event {
        InputEvent::Tracked { event, .. } | InputEvent::Observed { event, .. } => ends_idle(event),
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
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil)");
        assert_eq!(idle(&mut vm).unwrap(), Idle { ran: false, message: false, delay: None });
        vm.timer_stop_idle();
        // Ordinary timers: the ripe one runs once; the next delay is the far one's.
        printed(&mut vm, "(run-with-timer 0 nil (lambda () (push 'ordinary probe)))
                          (run-with-timer 60 nil #'ignore)
                          (run-with-idle-timer 0.05 nil (lambda () (push (list 'idle (and (current-idle-time) t)) probe)))");
        assert_eq!(printed(&mut vm, "(current-idle-time)"), "nil", "a command is not idle");
        let first = idle(&mut vm).unwrap();
        assert!(first.ran && !first.message, "{first:?}");
        assert_eq!(printed(&mut vm, "probe"), "(ordinary)");
        // Idle now: the idle timer is due 0.05s after idleness began.
        let waiting = idle(&mut vm).unwrap();
        assert!(!waiting.ran && waiting.delay.is_some_and(|d| d <= Duration::from_millis(50)), "{waiting:?}");
        std::thread::sleep(waiting.delay.unwrap());
        assert!(idle(&mut vm).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "((idle t) ordinary)", "an idle timer sees idleness");
        let after = idle(&mut vm).unwrap();
        assert!(!after.ran && after.delay.is_some_and(|d| d > Duration::from_secs(59)), "runs once per idleness: {after:?}");
        // A key ends idleness; the next idle period may run the idle timer again.
        vm.timer_stop_idle();
        assert_eq!(printed(&mut vm, "(current-idle-time)"), "nil");
        printed(&mut vm, "(setq probe nil) (run-with-idle-timer 0 t (lambda () (push 'again probe)))");
        assert!(idle(&mut vm).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "(again)");
        assert!(!idle(&mut vm).unwrap().ran, "a repeating idle timer waits for the next idleness");
        vm.timer_stop_idle();
        assert!(idle(&mut vm).unwrap().ran);
        assert_eq!(printed(&mut vm, "probe"), "(again again)");
        // An error is reported as timer-event-handler does and the loop goes on.
        vm.timer_stop_idle();
        printed(&mut vm, "(setq timer-idle-list nil) (run-with-timer 0 nil (lambda () (error \"boom\")))");
        let failed = idle(&mut vm).unwrap();
        assert!(failed.ran && failed.message, "{failed:?}");
        assert_eq!(printed(&mut vm, "(current-message)"), "\"Error running timer: (error \\\"boom\\\")\"");
        // read_char's own wait (rlisp/vm/keyboard.el): a prompt idles and runs
        // timers until its key comes; `read-event' with a time limit runs
        // ordinary timers only and answers nil once the limit passes.
        vm.timer_stop_idle();
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_millis(80)), Step::Key('a'), Step::Key('\r')].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil)
            (run-with-timer 0.01 nil (lambda () (push 'ordinary probe)))
            (run-with-idle-timer 0.03 nil (lambda () (push (list 'idle (and (current-idle-time) t)) probe)))");
        assert_eq!(printed(&mut vm, "(list (read-string \"P: \") (reverse probe) (current-idle-time))"),
                   "(\"a\" (ordinary (idle t)) nil)");
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq probe nil)
            (run-with-timer 0.01 nil (lambda () (push 'ordinary probe)))
            (run-with-idle-timer 0 nil (lambda () (push 'idle probe)))");
        let start = std::time::Instant::now();
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 0.05) probe (current-idle-time))"), "(nil (ordinary) nil)");
        assert!(start.elapsed() >= Duration::from_millis(50));
        // A timer that adds a timer: the wait checks again before blocking,
        // so the new one runs when due, not when the limit ends the block.
        // Pinned GNU 45b0f7699df, batch, stdin open: (nil (a (b t))).
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil probe nil start (float-time))
            (run-at-time 0 nil (lambda () (push 'a probe)
              (run-at-time 0.01 nil (lambda () (push (list 'b (< (- (float-time) start) 0.1)) probe)))))");
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 0.2) (reverse probe))"), "(nil (a (b t)))");
        // A timer that queues input wakes the read, which takes the event.
        // Pinned GNU 45b0f7699df, batch, stdin open: (97 t nil).
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, "(setq timer-list nil timer-idle-list nil start (float-time))
            (run-at-time 0.01 nil (lambda () (push ?a unread-command-events)))");
        assert_eq!(printed(&mut vm, "(list (read-event nil nil 1) (< (- (float-time) start) 0.5) unread-command-events)"),
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
        assert!(idle(&mut vm).unwrap().ran);
        assert_eq!(printed(&mut vm, "(list probe unread-command-events)"), "(nil ((t . 120)))");
        assert!(crate::command::unread_pending(&mut vm).unwrap());
        let outcome = crate::command::run(&mut vm).unwrap();
        assert_eq!(outcome.command.as_deref(), Some("self-insert-command"));
        assert_eq!(printed(&mut vm, "(list (buffer-string) unread-command-events)"), "(\"x\" nil)");
        assert!(!crate::command::unread_pending(&mut vm).unwrap());
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
        vm.eval_source(include_str!("../../../rlisp/vm/process.el")).unwrap();
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
        let fired = printed(&mut vm, "(list (reverse (mapcar (lambda (e) (if (consp e) (car e) e)) log)) (let ((timer (assq 'timer log))) (and timer (< (cadr timer) 1.0))))");
        assert_eq!(fired, "((callback timer) t)", "a callback's timer");
        // A mode-line :eval schedules it in the redisplay of the wait for
        // C-x's next key.
        printed(&mut vm, &format!("(setq timer-list nil log nil start nil)
            (setq mode-line-format (list (list :eval (quote (progn (when (and (null start) (equal (this-command-keys-vector) [24]))
              (setq start (float-time) log (cons 'eval log)) {schedule}) \"\")))))"));
        vm.set_host_input(Some(Box::new(Paced([Step::Key('\u{18}'), Step::Pause(Duration::from_secs(2)), Step::Key('h')].into()))));
        assert_eq!(crate::command::run(&mut vm).unwrap().command.as_deref(), Some("mark-whole-buffer"));
        let fired = printed(&mut vm, "(list (reverse (mapcar (lambda (e) (if (consp e) (car e) e)) log)) (let ((timer (assq 'timer log))) (and timer (< (cadr timer) 1.0))))");
        assert_eq!(fired, "((eval timer) t)", "a redisplay's timer");
    }

    /// A timer that reschedules itself at once runs once per round of a
    /// wait, which reads a queued key and ends at its time limit between
    /// rounds. Pinned GNU 45b0f7699df, -nw: the key typed 0.3s in is read
    /// then; its time limit waits for the rescheduling to stop
    /// (DIVERGENCES.md VM2-KBD1).
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
            (defun rescheduling () (setq runs (1+ runs)) (when (< (- (float-time) start) 2) (run-with-timer 0 nil #'rescheduling)))
            (run-with-timer 0 nil #'rescheduling)";
        vm.set_host_input(Some(Box::new(Paced([Step::Key('a')].into()))));
        printed(&mut vm, rescheduling);
        let key = printed(&mut vm, "(list (read-event) (< (- (float-time) start) 0.5) (<= runs 2))");
        vm.set_host_input(Some(Box::new(Paced([Step::Pause(Duration::from_secs(10))].into()))));
        printed(&mut vm, rescheduling);
        let limit = printed(&mut vm, "(list (read-event nil nil 0.1) (< (- (float-time) start) 0.5))");
        assert_eq!((key.as_str(), limit.as_str()), ("(97 t t)", "(nil t)"), "a queued key, then the time limit");
    }

    /// A pause, a key, or the wake a completed background command gives
    /// after the time it takes to complete.
    enum Step { Pause(Duration), Key(char), Completed(Duration) }

    /// A keyboard whose keys come after pauses, honouring each read's time limit.
    struct Paced(std::collections::VecDeque<Step>);

    impl romacs_vm::HostInput for Paced {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<(u64, usize)>, String> { Ok(vec![]) }
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
