//! Keyboard input read in the middle of an evaluation: the wait in GNU's
//! keyboard.c `read_char' (redisplay, then `wait_reading_process_output').
//!
//! GNU prompts by recursion: `read_minibuf' runs a recursive edit whose
//! `read_char' redisplays and blocks for a key while every outer C frame stays
//! suspended on the native stack. VM2 has the same shape. The Lisp half of
//! `read-from-minibuffer' (prims/minibuf.el) calls `romacs--read-host-event'
//! for each key; that primitive is engine control like `eval' and `load'
//! (threaded.rs `reentrant_load'): the suspended register stack leaves the
//! collector's view and collection is inhibited, so a redisplay that runs Lisp
//! (a mode-line `:eval') executes in its own engine without moving anything a
//! suspended frame holds. Nothing here holds a `Value' across the wait: the
//! host sees only plain frame data and answers with plain events.
//!
//! A VM with no host attached keeps batch GNU's behaviour: the primitive
//! returns nil (end of input) and `read-from-minibuffer' still signals
//! `(end-of-file "Error reading from stdin")' outside a keyboard macro.
use crate::{CommandEvent, Result, Value, Vm, threaded};

/// One redisplay request: the frozen frame and the selected window's id.
pub struct HostFrame {
    pub frame: romacs_layout_input::FrameSnapshot,
    pub selected: u64,
}

/// What the host answers a read with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// A complete input event, GNU's encoding (an integer or a symbol name).
    Key(CommandEvent),
    /// The frame was resized; the read redisplays and keeps waiting.
    Resize { width: u32, height: u32 },
    /// The frame is gone: the read signals `quit' so every level unwinds.
    Closed,
    /// Native background state changed.
    Background,
    /// The read's time limit passed first.
    Timeout,
}

/// The embedder's keyboard, screen, and optional native clipboard.
pub trait HostInput {
    /// True when an event is already queued. GNU's `read_char' does not
    /// redisplay while input is pending (keyboard.c, `detect_input_pending').
    fn input_pending(&mut self) -> bool;
    /// Paint `frame`; return the layout's window ends to commit
    /// (`Vm::display_apply_layout').
    fn redisplay(&mut self, frame: HostFrame) -> std::result::Result<Vec<(u64, usize)>, String>;
    /// Block until the next event, or until TIMEOUT passes (`Timeout').
    fn next_event(&mut self, timeout: Option<std::time::Duration>) -> std::result::Result<HostEvent, String>;

    /// Read CLIPBOARD text with a bounded wait and without running Lisp.
    /// No text (`None`) is distinct from an unavailable host or backend error.
    fn clipboard_text(&mut self) -> std::result::Result<Option<String>, String> {
        Err("native clipboard is unavailable".into())
    }

    /// Publish CLIPBOARD text with a bounded wait and without running Lisp.
    fn set_clipboard_text(&mut self, _text: &str) -> std::result::Result<(), String> {
        Err("native clipboard is unavailable".into())
    }
}

/// The Lisp variable that tells the minibuffer a host keyboard exists.
pub(crate) const HOST_INPUT_VARIABLE: &str = "romacs--host-input";

impl Vm {
    /// Attach (Some) or detach (None) the host keyboard. While attached,
    /// `read-from-minibuffer' outside a keyboard macro reads its keys here.
    pub fn set_host_input(&mut self, host: Option<Box<dyn HostInput>>) {
        let attached = if host.is_some() { Value::TRUE } else { Value::NIL };
        self.host_input = host;
        self.declare_special_raw(HOST_INPUT_VARIABLE, attached);
    }

    /// `romacs--read-host-event' &optional SECONDS: redisplay unless input
    /// is pending, then wait for one event, or nil once SECONDS pass. Lisp
    /// runs timers between reads (rlisp/vm/keyboard.el). Runs only under
    /// `reentrant_load''s discipline.
    pub(crate) fn read_host_event(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() > 1 {
            return Err("wrong primitive arity".into());
        }
        let deadline = match args.first() {
            None => None,
            Some(&v) if v == Value::NIL => None,
            Some(&v) => {
                let seconds = match (v.integer_value(), self.object_if(v)) {
                    (Some(n), _) => n as f64,
                    (None, Some(crate::data::Object::Float(n))) => *n,
                    _ => return Err("romacs--read-host-event: SECONDS is not a number".into()),
                };
                let wait = std::time::Duration::try_from_secs_f64(seconds.clamp(0.0, 1e9)).unwrap_or_default();
                Some(std::time::Instant::now() + wait)
            }
        };
        // Taken, not borrowed: the redisplay below needs `&mut self`, and a
        // read cannot re-enter itself because no Lisp runs inside the wait
        // except the redisplay's own, which cannot reach this primitive with
        // the host absent (it would see end of input, nil).
        let Some(mut host) = self.host_input.take() else {
            return Ok(Value::NIL);
        };
        let outcome = self.read_host_event_with(host.as_mut(), deadline);
        self.host_input = Some(host);
        match outcome? {
            HostEvent::Key(CommandEvent::Character(c)) => Value::integer(c),
            HostEvent::Key(CommandEvent::Named(name)) => Ok(self.intern(&name)),
            HostEvent::Timeout => Ok(Value::NIL),
            HostEvent::Resize { .. } | HostEvent::Background => unreachable!("background events are consumed by the wait"),
            HostEvent::Closed => {
                let quit = self.intern("quit");
                self.propagated_escape = Some(threaded::Escape::Signal(quit, Value::NIL));
                Err("quit: frame closed while reading input".into())
            }
        }
    }

    /// keyboard.c timer_stop_idle: the host read a key for a command.
    pub fn timer_stop_idle(&mut self) { self.idle_since = None; }

    fn redisplay_inhibited(&mut self) -> Result<bool> {
        let slot = self.slot("inhibit-redisplay");
        Ok(self.variable_value(slot)? != Value::NIL)
    }

    fn read_host_event_with(&mut self, host: &mut dyn HostInput, deadline: Option<std::time::Instant>) -> Result<HostEvent> {
        let mut redraw = true;
        loop {
            redraw |= self.poll_terminal_output()?;
            // GNU redisplay_internal does nothing while `inhibit-redisplay'
            // is non-nil, as it is around Lisp that redisplay itself calls.
            if redraw && !host.input_pending() && !self.redisplay_inhibited()? {
                let (width, height) = (self.window_state.width, self.window_state.height);
                let frame = self.frame_snapshot(width, height)?;
                let selected = self.selected_window_value()?;
                let selected = self.window(selected)?.id;
                let ends = host.redisplay(HostFrame { frame, selected })?;
                self.display_apply_layout(&ends)?;
                redraw = false;
            }
            let timeout = deadline.map(|end| end.saturating_duration_since(std::time::Instant::now()));
            match host.next_event(timeout)? {
                HostEvent::Resize { width, height } => {
                    let (line, column) = (self.window_state.line, self.window_state.column);
                    self.set_frame_geometry(width, height, line, column)?;
                    redraw = true;
                }
                HostEvent::Background => {}
                event => return Ok(event),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Keys(Vec<i64>);
    impl HostInput for Keys {
        fn input_pending(&mut self) -> bool { true }
        fn redisplay(&mut self, _: HostFrame) -> std::result::Result<Vec<(u64, usize)>, String> { Ok(Vec::new()) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<HostEvent, String> {
            Ok(HostEvent::Key(CommandEvent::Character(self.0.remove(0))))
        }
    }

    /// The T2 continuation path (`t2_dispatch`) also routes the read to engine
    /// control: a T2 primitive's callback `apply`s it with no arguments.
    #[test]
    fn t2_continuation_reaches_the_read_as_engine_control() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.set_collection_floor(1);
        vm.set_host_input(Some(Box::new(Keys(vec![120]))));
        let got = vm
            .eval_source("(let ((keep (list 5 6))) (list (romacs--t2-assoc nil '((romacs--read-host-event)) #'apply) keep))")
            .map(|v| v.display().unwrap())
            .unwrap_or_else(|e| format!("VM-ERROR {e}"));
        assert_eq!(got, "((romacs--read-host-event) (5 6))");
    }
}
