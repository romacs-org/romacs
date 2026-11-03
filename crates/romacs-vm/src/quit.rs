//! Quitting: GNU's asynchronous `C-g' (keyboard.c handle_interrupt) and the
//! evaluator's `maybe_quit' (eval.c) at its safepoints.
//!
//! GNU reads a typed `C-g' out of band: kbd_buffer_store_event never queues
//! the quit character, it sets `quit-flag' (handle_interrupt), and the
//! evaluator acts on it at its next `maybe_quit': a backward branch, a
//! function call, a long primitive's own poll. A host here reads input on
//! another thread, so the `C-g' arrives as a press counted in one atomic
//! word ([`QuitSignal`]) that the host's input thread shares with the
//! evaluator. The word also says whether the evaluator is busy (running a
//! command) rather than waiting for input: only a busy evaluator takes a
//! press out of band, so a `C-g' typed while it waits is an ordinary key,
//! read as GNU's read_char returns the quit character. The busy bit and the
//! press count change together, so each press goes exactly one way.
//!
//! The evaluator turns presses into `quit-flag' at a safepoint (GNU reads
//! its input there too, from process_pending_signals), then runs
//! `maybe_quit': nothing while `inhibit-quit' is non-nil, else `quit-flag'
//! is cleared and `quit' signalled, or, when it is `throw-on-input''s tag,
//! thrown to. A third press while one is still pending clears
//! `inhibit-quit' first, GNU's forced quit for a graphical frame.
use crate::{Value, Vm, threaded::Escape};
use std::sync::atomic::{AtomicU32, Ordering};

/// The evaluator's half of the host's `C-g': whether it is busy, how many
/// presses it has not taken, and a count of its busy spans, in one word.
#[derive(Debug)]
pub struct QuitSignal {
    word: AtomicU32,
}
romacs_snapshot::snapshot_struct!(QuitSignal { word });

/// How long a quittable wait blocks between looks at the press count.
const QUIT_SLICE: std::time::Duration = std::time::Duration::from_millis(40);

const BUSY: u32 = 1 << 31;
/// A call's deadline passed (`arm_deadline'): taken, like a press, at the
/// evaluator's next safepoint while it is busy, and only for that call.
const DEADLINE: u32 = 1 << 30;
const PRESSES: u32 = 0xffff;
const SPAN_ONE: u32 = 1 << 16;
const SPANS: u32 = !(BUSY | DEADLINE | PRESSES);

impl QuitSignal {
    /// A signal for an evaluator that is busy: in batch every evaluation may
    /// quit; a host with a keyboard marks its waits idle.
    pub fn new() -> Self {
        Self { word: AtomicU32::new(BUSY) }
    }

    /// The input thread's `C-g': counted, and true, when the evaluator is
    /// busy; false when it waits for input, which then reads the key itself.
    pub fn interrupt(&self) -> bool {
        self.word.fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| {
            (word & BUSY != 0).then(|| if word & PRESSES == PRESSES { word } else { word + 1 })
        }).is_ok()
    }

    /// Mark the evaluator busy (it runs Lisp that may quit) or waiting for
    /// input. Leaving busy takes the presses not yet taken: the wait reads
    /// them as keys. Returns them.
    pub fn set_busy(&self, busy: bool) -> u32 {
        let previous = self.word.fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| Some(match (busy, word & BUSY != 0) {
            (true, true) | (false, false) => word,
            (true, false) => BUSY | (word & SPANS).wrapping_add(SPAN_ONE) & SPANS | (word & (PRESSES | DEADLINE)),
            (false, true) => word & (SPANS | DEADLINE),
        })).expect("the update always answers");
        if busy { 0 } else { previous & PRESSES }
    }

    /// Whether the evaluator is busy, and which busy span: a host's busy
    /// indicator compares spans to tell one long evaluation from several.
    pub fn busy_span(&self) -> Option<u32> {
        let word = self.word.load(Ordering::Acquire);
        (word & BUSY != 0).then_some((word & SPANS) >> 16)
    }

    #[inline(always)]
    pub(crate) fn busy(&self) -> bool {
        self.word.load(Ordering::Relaxed) & BUSY != 0
    }

    /// Whether a press waits, and whether the evaluator is busy: one load.
    #[inline(always)]
    pub(crate) fn state(&self) -> (bool, bool) {
        let word = self.word.load(Ordering::Relaxed);
        (word & (PRESSES | DEADLINE) != 0, word & BUSY != 0)
    }

    #[inline(always)]
    pub(crate) fn pressed(&self) -> bool {
        self.word.load(Ordering::Relaxed) & PRESSES != 0
    }

    /// A press, or a call's deadline passed: what a safepoint polls for.
    #[inline(always)]
    pub(crate) fn due(&self) -> bool {
        self.word.load(Ordering::Relaxed) & (PRESSES | DEADLINE) != 0
    }

    /// Take a passed deadline: whether it was the current call's.
    pub(crate) fn take_deadline(&self) -> bool {
        self.word.fetch_and(!DEADLINE, Ordering::AcqRel) & DEADLINE != 0 && crate::host::call_deadline::expired_is_current()
    }

    /// A call's deadline passed (host.rs `call_deadline'), or with false
    /// one no longer counts.
    pub(crate) fn set_deadline(&self, passed: bool) {
        if passed { self.word.fetch_or(DEADLINE, Ordering::AcqRel); } else { self.word.fetch_and(!DEADLINE, Ordering::AcqRel); }
    }

    /// Take the presses counted so far.
    pub(crate) fn take(&self) -> u32 {
        self.word.fetch_and(!PRESSES, Ordering::AcqRel) & PRESSES
    }
}

impl Default for QuitSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    /// The current values of `quit-flag', `inhibit-quit' and
    /// `throw-on-input': a `let' binds a plain variable in its value cell,
    /// so this is the dynamic value. Unbound reads as nil.
    #[inline(always)]
    pub(crate) fn quit_variables(&self) -> [Value; 3] {
        self.quit_slots.map(|slot| match self.slots[slot].value { v if v == Value::UNBOUND => Value::NIL, v => v })
    }

    /// The quit signal the host's input thread shares with this evaluator.
    pub fn quit_signal(&self) -> std::sync::Arc<QuitSignal> {
        self.quit.clone()
    }

    /// Whether an engine entering Lisp should poll at once: a press is
    /// waiting, or `quit-flag' is set while evaluation may quit. Two loads
    /// and a compare: every engine entry asks.
    #[inline(always)]
    pub(crate) fn quit_due(&self) -> bool {
        let (pressed, busy) = self.quit.state();
        pressed || (busy && self.slots[self.quit_slots[0]].value != Value::NIL && self.slots[self.quit_slots[0]].value != Value::UNBOUND)
    }

    /// The engine's periodic safepoint also polls inside `while-no-input',
    /// whose input arrives as ordinary events (keyboard.c
    /// kbd_buffer_store_event sets `quit-flag' to `throw-on-input').
    pub(crate) fn quit_poll_due(&self) -> bool {
        let [flag, _, throw_on_input] = self.quit_variables();
        self.quit.due() || (self.quit.busy() && (flag != Value::NIL || throw_on_input != Value::NIL))
    }

    /// GNU maybe_quit at a safepoint, with handle_interrupt for each press
    /// the host counted: the escape to raise, or None to go on. Never while
    /// compiling or expanding a macro, whose escape would unwind past the
    /// handlers of the code being compiled; the quit then waits for the
    /// next safepoint.
    pub(crate) fn maybe_quit(&mut self) -> Option<Escape> {
        if self.compile_depth > 0 || !self.expanding.is_empty() || !self.quit.busy() {
            return None;
        }
        for _ in 0..self.quit.take() {
            self.handle_interrupt();
        }
        // A call's budget spent is its C-g (`romacs--call-deadline').
        if self.quit.take_deadline() {
            self.set_quit_variable("quit-flag", Value::TRUE);
        }
        let [flag, inhibit, throw_on_input] = self.quit_variables();
        if flag == Value::NIL && throw_on_input != Value::NIL && self.host_input_pending() {
            self.set_quit_variable("quit-flag", throw_on_input);
            return self.maybe_quit();
        }
        if flag == Value::NIL || inhibit != Value::NIL {
            return None;
        }
        // process_quit_flag.
        self.set_quit_variable("quit-flag", Value::NIL);
        if flag == throw_on_input {
            return Some(Escape::Throw(flag, Value::TRUE));
        }
        let quit = self.intern("quit");
        Some(Escape::Signal(quit, Value::NIL))
    }

    /// `maybe_quit' for Rust that runs between Lisp evaluations (the
    /// loader between forms): the escape left for the caller to propagate.
    pub(crate) fn poll_quit(&mut self) -> crate::Result<()> {
        if !self.quit_poll_due() {
            return Ok(());
        }
        match self.maybe_quit() {
            None => Ok(()),
            Some(escape) => {
                self.propagated_escape = Some(escape);
                self.propagated_escape_hooked = false;
                Err("Quit".into())
            }
        }
    }

    /// keyboard.c handle_interrupt for a graphical frame: `quit-flag' set,
    /// and the third `C-g' in a row that found it set clears `inhibit-quit'.
    pub(crate) fn handle_interrupt(&mut self) {
        let pending = self.quit_variables()[0] != Value::NIL;
        self.force_quit_count = if pending { self.force_quit_count.saturating_add(1) } else { 1 };
        if self.force_quit_count == 3 {
            self.set_quit_variable("inhibit-quit", Value::NIL);
        }
        self.set_quit_variable("quit-flag", Value::TRUE);
    }

    /// keyboard.c read_char, for a host keyboard read: whether a `C-g'
    /// already arrived, as a press the host counted or as `quit-flag', so
    /// the read returns the quit character at once (GNU's
    /// quit_throw_to_read_char). `quit-flag' is cleared: the reader acts on
    /// the key as read_char's caller does on its quit (rlisp/vm/keyboard.el).
    /// When `quit-flag' is `throw-on-input''s tag the read returns nothing
    /// and leaves it for the next safepoint, which throws.
    pub(crate) fn take_read_quit(&mut self) -> ReadQuit {
        for _ in 0..self.quit.take() {
            self.handle_interrupt();
        }
        let [flag, _, throw_on_input] = self.quit_variables();
        if flag == Value::NIL {
            return ReadQuit::None;
        }
        if flag == throw_on_input {
            return ReadQuit::Thrown;
        }
        self.set_quit_variable("quit-flag", Value::NIL);
        ReadQuit::Key
    }

    /// Share SIGNAL with the host's input thread, the evaluator waiting for
    /// input until the host says it runs a command (`enter_busy').
    pub fn share_quit_signal(&mut self, signal: std::sync::Arc<QuitSignal>) {
        signal.set_busy(false);
        self.quit = signal;
    }

    /// The host runs a command: a C-g typed from now on quits it.
    pub fn enter_busy(&mut self) {
        self.quit.set_busy(true);
    }

    /// The host's command ended: a C-g it did not take sets `quit-flag', as
    /// GNU's handle_interrupt does, for the next read (`take_quit_key').
    pub fn leave_busy(&mut self) {
        for _ in 0..self.quit.set_busy(false) {
            self.handle_interrupt();
        }
    }

    /// keyboard.c read_char between commands: whether a quit is pending, so
    /// that the next key is the quit character, ahead of any queued input.
    /// `quit-flag' is cleared, as read_key_sequence clears it once read
    /// (`inhibit-quit' held the C-g to the end of the command, or it came
    /// after the command's last safepoint, or a timer set it).
    pub fn take_quit_key(&mut self) -> bool {
        if !self.quit_input_pending() {
            return false;
        }
        for _ in 0..self.quit.take() {
            self.handle_interrupt();
        }
        self.set_quit_variable("quit-flag", Value::NIL);
        true
    }

    /// kbd_buffer_store_event's last step for an input event read while
    /// `throw-on-input' is non-nil.
    pub(crate) fn note_input_for_throw(&mut self) {
        let throw_on_input = self.quit_variables()[2];
        if throw_on_input != Value::NIL {
            self.set_quit_variable("quit-flag", throw_on_input);
        }
    }

    /// Store NAME, a quit variable, as GNU's C does: no watcher runs.
    fn set_quit_variable(&mut self, name: &str, value: Value) {
        let slot = self.slot(name);
        // A forwarded object variable always takes a store.
        let _ = self.assign_variable(slot, value);
    }

    /// subprocess::poll of FDS for at most TIMEOUT, which a host's C-g ends
    /// early, as keyboard input wakes GNU's select: while a host shares the
    /// signal the wait goes in slices of [`QUIT_SLICE`], and returns no
    /// events once a press waits, for the caller's next safepoint to quit.
    pub(crate) fn poll_quittable(&self, fds: &[(std::os::fd::RawFd, i16)], timeout: Option<std::time::Duration>)
                                 -> std::io::Result<Vec<i16>> {
        if std::sync::Arc::strong_count(&self.quit) == 1 {
            return crate::subprocess::poll(fds, timeout);
        }
        let deadline = timeout.map(|timeout| crate::host::monotonic() + timeout);
        loop {
            if self.quit.pressed() {
                return Ok(vec![0; fds.len()]);
            }
            let left = deadline.map(|end| end.saturating_duration_since(crate::host::monotonic()));
            let revents = crate::subprocess::poll(fds, Some(left.map_or(QUIT_SLICE, |left| left.min(QUIT_SLICE))))?;
            if revents.iter().any(|&r| r != 0) || left.is_some_and(|left| left <= QUIT_SLICE) {
                return Ok(revents);
            }
        }
    }

    /// A quit read_char would return as the quit character.
    pub(crate) fn quit_input_pending(&self) -> bool {
        self.quit.pressed() || self.quit_variables()[0] != Value::NIL
    }

    pub(crate) fn host_input_pending(&mut self) -> bool {
        self.host_input.as_mut().is_some_and(|host| host.input_pending())
    }
}

/// What a host keyboard read does about a `C-g' that already arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadQuit {
    None,
    /// Return the quit character.
    Key,
    /// Return nothing: `quit-flag' holds `throw-on-input''s tag.
    Thrown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_goes_to_a_busy_evaluator_and_a_waiting_one_reads_it() {
        let signal = QuitSignal::new();
        assert!(signal.interrupt() && signal.interrupt());
        assert_eq!(signal.set_busy(false), 2, "presses not taken go to the wait");
        assert!(!signal.interrupt(), "a waiting evaluator reads the key itself");
        assert_eq!(signal.busy_span(), None);
        signal.set_busy(true);
        assert!(signal.interrupt());
        assert_eq!(signal.take(), 1);
        assert_eq!(signal.take(), 0);
    }

    #[test]
    fn each_busy_span_is_distinct() {
        let signal = QuitSignal::new();
        let first = signal.busy_span().unwrap();
        signal.set_busy(true);
        assert_eq!(signal.busy_span(), Some(first), "already busy: the same span");
        signal.set_busy(false);
        signal.set_busy(true);
        assert_ne!(signal.busy_span(), Some(first));
    }

    /// Racing presses against the evaluator leaving busy: each press is
    /// either counted and returned by the transition, or refused (and so
    /// queued as a key by the host), never lost or doubled.
    #[test]
    fn presses_racing_the_wait_are_never_lost() {
        let signal = std::sync::Arc::new(QuitSignal::new());
        let presser = {
            let signal = signal.clone();
            std::thread::spawn(move || (0..20_000).filter(|_| !signal.interrupt()).count() as u32)
        };
        let mut taken = 0;
        for i in 0..20_000 {
            taken += if i % 2 == 0 { signal.set_busy(false) } else { signal.set_busy(true); signal.take() };
        }
        taken += signal.set_busy(false);
        let refused = presser.join().unwrap();
        assert_eq!(taken + refused, 20_000);
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use std::time::Duration;

    fn wait_due(signal: &QuitSignal) -> bool {
        (0..100).any(|_| { std::thread::sleep(Duration::from_millis(5)); signal.due() })
    }

    /// One test, as the deadline is one per process: the current call's
    /// passed deadline is taken once; a call that ended first leaves none;
    /// one that passed while not taken is retired at the call's end.
    #[test]
    fn a_deadline_reaches_only_its_own_call() {
        let signal = std::sync::Arc::new(QuitSignal::new());
        crate::host::call_deadline::arm(signal.clone(), 1, Duration::from_millis(10));
        assert!(wait_due(&signal), "the deadline passed");
        assert!(!signal.pressed(), "a deadline is no key press");
        assert!(signal.take_deadline(), "taken by its call");
        assert!(!signal.due(), "taken once");
        crate::host::call_deadline::disarm(&signal);

        crate::host::call_deadline::arm(signal.clone(), 2, Duration::from_millis(30));
        crate::host::call_deadline::disarm(&signal);
        assert!(!wait_due(&signal), "an ended call's deadline never fires");

        crate::host::call_deadline::arm(signal.clone(), 3, Duration::from_millis(5));
        assert!(wait_due(&signal));
        crate::host::call_deadline::disarm(&signal);
        assert!(!signal.due(), "retired with its call");
        crate::host::call_deadline::arm(signal.clone(), 4, Duration::from_secs(60));
        assert!(!signal.take_deadline(), "a later call takes nothing of an earlier one's");
        crate::host::call_deadline::disarm(&signal);
    }

    #[test]
    fn leaving_busy_keeps_a_passed_deadline_for_the_call() {
        let signal = QuitSignal::new();
        signal.set_deadline(true);
        signal.set_busy(false);
        assert!(signal.due(), "a wait inside the call keeps it");
        signal.set_busy(true);
        assert!(signal.due(), "and the call has it when it runs again");
    }
}
