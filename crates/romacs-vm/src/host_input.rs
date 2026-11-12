//! Keyboard input read in the middle of an evaluation: the wait in GNU's
//! keyboard.c `read_char' (redisplay, then `wait_reading_process_output').
//!
//! GNU prompts by recursion: `read_minibuf' runs a recursive edit whose
//! `read_char' redisplays and blocks for a key while every outer C frame stays
//! suspended on the native stack. VM2 has the same shape. The Lisp half of
//! `read-from-minibuffer' (prims/minibuf.el) calls `romacs--read-host-event'
//! for each key; that primitive is lent from its engine as `load' is
//! (threaded.rs `loader_body'): the waiting engines are suspended, their
//! registers and code traced, so a redisplay that runs Lisp (a mode-line
//! `:eval') executes in its own engine and may collect. The wait is
//! orchestration (`crate::brand`, `wait_for_event'): it holds no `Value', its
//! Lisp runs in requests, and the host sees only plain frame data and answers
//! with plain events.
//!
//! A VM with no host attached keeps batch GNU's behaviour: the primitive
//! returns nil (end of input) and `read-from-minibuffer' still signals
//! `(end-of-file "Error reading from stdin")' outside a keyboard macro.
use crate::brand::{Gc, Mx};
use crate::gc::RootFrame;
use crate::loader::Out;
use crate::{CommandEvent, Result, Value, Vm, quit, threaded};

/// keyboard.c quit_char: C-g. `set-quit-char' changes it only for a text
/// terminal, which Romacs has none of.
pub const QUIT_CHAR: i64 = 7;

/// One redisplay request: the frozen frame, the selected window's id, and
/// how long freezing the frame took (for latency traces).
pub struct HostFrame {
    pub frame: romacs_layout_input::FrameSnapshot,
    pub selected: u64,
    /// The minibuffer window's id: its snapshot is the frame's last
    /// window, or a child frame's when one hosts it.
    pub minibuffer: u64,
    pub snapshot: std::time::Duration,
    /// The text animations drawn over this frame (`Vm::text_animations').
    pub text_animations: Vec<(u64, romacs_display_protocol::text_animation::TextAnimation)>,
}

/// What the host answers a read with.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// A complete input event, GNU's encoding (an integer or a symbol name).
    Key(CommandEvent),
    /// The frame was resized; the read redisplays and keeps waiting.
    Resize { width: u32, height: u32 },
    /// The frame is gone: the read signals `quit' so every level unwinds.
    Closed,
    /// Native background state changed.
    Background,
    /// A descriptor `HostInput::watch_processes' named is ready: the read
    /// ends as a timeout does, for Lisp to read it.
    Process,
    /// The read's time limit passed first.
    Timeout,
    /// The frame gained (true) or lost focus: keyboard.c's FOCUS_IN_EVENT
    /// and FOCUS_OUT_EVENT, read as GNU's special events.
    Focus(bool),
    /// The renderer's event about native model ID, read as the special
    /// event `(romacs-model ID TYPE PLIST)` (`display_host/models.rs`).
    Model { id: u32, event: romacs_display_protocol::model::ModelEvent },
}

/// The embedder's keyboard, screen, and optional native clipboard.
pub trait HostInput {
    /// True when an event is already queued. GNU's `read_char' does not
    /// redisplay while input is pending (keyboard.c, `detect_input_pending').
    fn input_pending(&mut self) -> bool;
    /// True when a key press is queued: what ends an idle collection while
    /// it is still marking (`Vm::collect_garbage_until_key'). A host that
    /// cannot tell keys from other events answers whether any is queued.
    fn key_pending(&mut self) -> bool {
        self.input_pending()
    }
    /// Paint `frame`; return the layout's window ends to commit
    /// (`Vm::display_apply_layout').
    fn redisplay(&mut self, frame: HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String>;
    /// Block until the next event, or until TIMEOUT passes (`Timeout').
    fn next_event(&mut self, timeout: Option<std::time::Duration>) -> std::result::Result<HostEvent, String>;

    /// The process descriptors the next `next_event' also waits on, as
    /// wait_reading_process_output selects on them with the keyboard:
    /// one ready ends it with `Process'.  A host that cannot wait on them
    /// ignores them, and process output waits for Lisp's own waits.
    fn watch_processes(&mut self, _fds: &[(std::os::fd::RawFd, i16)]) {}
    /// Whether the last paint left images to send and was drawn: paint
    /// again before waiting for input.
    fn images_behind(&mut self) -> bool { false }

    /// Read CLIPBOARD text with a bounded wait and without running Lisp.
    /// No text (`None`) is distinct from an unavailable host or backend error.
    fn clipboard_text(&mut self) -> std::result::Result<Option<String>, String> {
        Err("native clipboard is unavailable".into())
    }

    /// Publish CLIPBOARD text with a bounded wait and without running Lisp.
    fn set_clipboard_text(&mut self, _text: &str) -> std::result::Result<(), String> {
        Err("native clipboard is unavailable".into())
    }

    /// Whether the display shows frame-by-frame animation
    /// (`romacs--display-animates'): None until it knows its adapter.  A
    /// host with no window, every test's, shows none.
    fn display_animates(&mut self) -> Option<bool> {
        Some(false)
    }

    /// Ask about or change the host frame's native window
    /// (`romacs--native-window'): whether one shows after the request is
    /// taken, None when the host has no native windows. A server session
    /// hides and shows it; a session that is its window only reports it.
    fn native_window(&mut self, _action: NativeWindow) -> Option<bool> {
        None
    }

    /// Where the pointer is over the host frame, in pixels: GNU's
    /// `mouse_position_hook'.  None with no pointer there, or no display.
    fn pointer(&mut self) -> Option<(i32, i32)> {
        None
    }
}

/// What `romacs--native-window' asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeWindow {
    /// Whether the window shows.
    Query,
    /// Show it, made anew when it was closed, and ask for the focus.
    Show,
    /// Close it and keep the session.
    Hide,
}

/// The Lisp variable that tells the minibuffer a host keyboard exists.
pub(crate) const HOST_INPUT_VARIABLE: &str = "romacs--host-input";

impl Vm {
    /// Attach (Some) or detach (None) the host keyboard. While attached,
    /// `read-from-minibuffer' outside a keyboard macro reads its keys here.
    pub fn set_host_input(&mut self, host: Option<Box<dyn HostInput>>) {
        let attached = if host.is_some() { Value::TRUE } else { Value::NIL };
        self.host_input = host.map(|host| Box::new(crate::host_calls::Counted(host)) as Box<dyn HostInput>);
        self.declare_special_raw(HOST_INPUT_VARIABLE, attached);
    }

    /// redisplay_internal's run_window_change_functions, which every
    /// redisplay's frame snapshot runs first (display.rs): the command
    /// loop's Lisp half (lisp/romacs/vm/window-change.el) once it is installed.
    /// Lisp runs only when the windows differ from what the last run saw,
    /// so a redisplay that changed no window enters no Lisp.
    pub(crate) fn run_window_change_functions(&mut self) -> Result<()> {
        if self.window_change_signature()? == self.window_change_seen { return Ok(()); }
        let run = "(lambda () (and (fboundp 'romacs--run-window-change-functions)
                     (condition-case err (romacs--run-window-change-functions)
                       (error (message \"Error in window change functions: %S\" err)))))";
        self.call_prepared_source_propagating(run, &[])?;
        self.window_change_seen = self.window_change_signature()?;
        Ok(())
    }
    /// prepare_menu_bars's call of `pre-redisplay-function', with the windows
    /// to redisplay. Lisp runs only when it would do something: never for
    /// nil or `ignore', nor for `simple.el''s default while no window's
    /// buffer has work on `pre-redisplay-functions'.
    pub(crate) fn run_pre_redisplay(&mut self) -> Result<()> {
        let Some(windows) = self.pre_redisplay_call()? else { return Ok(()) };
        self.call_prepared_source_propagating("(lambda (windows) (and (fboundp 'romacs--pre-redisplay) (romacs--pre-redisplay windows)))", &[windows])?;
        Ok(())
    }
    /// The windows argument `romacs--pre-redisplay' takes now, when it has
    /// anything to do.
    pub(crate) fn pre_redisplay_call(&mut self) -> Result<Option<Value>> {
        let slot = self.slot("pre-redisplay-function");
        let function = self.variable_value(slot)?;
        if matches!(function, Value::NIL | Value::UNBOUND) || function == self.display_symbol("ignore") { return Ok(None); }
        if function == self.display_symbol("redisplay--pre-redisplay-functions") && !self.pre_redisplay_has_work()? { return Ok(None); }
        self.pre_redisplay_windows().map(Some)
    }
    /// Whether `pre-redisplay-functions', in any shown buffer, holds more
    /// than GNU's region overlay, which `romacs--native-region' leaves to
    /// the native highlight unless a package wraps it (rect.el).
    fn pre_redisplay_has_work(&mut self) -> Result<bool> {
        let (hook, mark) = (self.slot("pre-redisplay-functions"), self.slot("mark-active"));
        let (region, global) = (self.display_symbol("redisplay--update-region-highlight"), self.default_value(hook)?);
        let highlight = self.slot("redisplay-highlight-region-function");
        let native = self.variable_value(highlight)? == self.display_symbol("romacs--native-region");
        let overlay = self.display_symbol("internal-region-overlay");
        let saved = self.slots[self.buffer_slot].value;
        let mut work = false;
        for window in self.displayed_windows() {
            let w = self.window(window)?;
            let (buffer, shown) = (w.buffer, w.parameters.iter().any(|&(k, v)| k == overlay && v != Value::NIL));
            self.slots[self.buffer_slot].value = buffer;
            let (value, marked) = (self.variable_value(hook), self.variable_value(mark));
            self.slots[self.buffer_slot].value = saved;
            let (value, marked) = (value?, marked? != Value::NIL);
            let functions = |vm: &Self, list: Value| vm.list_values(list).unwrap_or_else(|_| vec![list]);
            let mut members = functions(self, value);
            if members.contains(&Value::TRUE) { members.extend(functions(self, global)); }
            work |= members.iter().any(|&f| match f {
                Value::NIL | Value::TRUE | Value::UNBOUND => false,
                f if f == region => !native && (marked || shown),
                _ => true,
            });
        }
        Ok(work)
    }
    /// Every leaf window of every frame, the mini-windows too: what any
    /// redisplay may show.
    pub(crate) fn displayed_windows(&self) -> Vec<Value> {
        let mut out = vec![];
        for frame in self.frames() {
            let Ok(f) = self.frame(frame) else { continue };
            let mut pending = vec![f.root, f.minibuffer];
            while let Some(window) = pending.pop() {
                let Ok(w) = self.window(window) else { continue };
                if w.children.is_empty() { if !out.contains(&window) { out.push(window); } } else { pending.extend(w.children.iter().copied()); }
            }
        }
        out
    }
    /// What window_change_record keeps, for the run's gate: the selected
    /// frame, and each frame with its selected window and its windows'
    /// buffers, pixel boxes and what their body sizes also depend on (their
    /// parameters, whether their buffers show a mode or header line, the
    /// frame's divider widths, the faces' revision and the frame's line and
    /// column). Its words are heap
    /// indices, which a collection may reuse, so the count of collections
    /// is part of it.
    fn window_change_signature(&mut self) -> Result<Vec<u64>> {
        let mut out = vec![self.gc_stats.collections, self.frame_value_peek().0, self.display_state.face_revision,
                           u64::from(self.window_state.line), u64::from(self.window_state.column)];
        let lines = [self.slot("mode-line-format"), self.slot("header-line-format")];
        for frame in self.frames() {
            let Ok(f) = self.frame(frame) else { continue };
            out.extend([frame.0, f.selected.0, u64::from(f.dividers[0]), u64::from(f.dividers[1])]);
            let mut pending = vec![f.root, f.minibuffer];
            while let Some(window) = pending.pop() {
                let Ok(w) = self.window(window) else { continue };
                let buffer = w.buffer;
                out.extend([window.0, buffer.0, u64::from(w.x), u64::from(w.y), u64::from(w.width), u64::from(w.height),
                            u64::from(w.left_margin), u64::from(w.right_margin)]);
                out.extend(w.parameters.iter().flat_map(|(k, v)| [k.0, v.0]));
                pending.extend(w.children.iter().copied());
                if buffer != Value::NIL {
                    for slot in lines {
                        let format = self.value_in_buffer(slot, buffer)?;
                        out.push(u64::from(format != Value::NIL && format != Value::UNBOUND));
                    }
                }
            }
        }
        Ok(out)
    }

    /// A redisplay took TOOK: the first after `romacs--redisplay-since-mark's
    /// mark is what the command that marked it cost the display.
    pub fn redisplay_done(&mut self, took: std::time::Duration) {
        if self.redisplay_since_mark == Some(None) {
            self.redisplay_since_mark = Some(Some(u64::try_from(took.as_micros()).unwrap_or(u64::MAX)));
        }
    }

    /// keyboard.c timer_stop_idle: the host read a key for a command.
    pub fn timer_stop_idle(&mut self) {
        self.idle_since = None;
        crate::compiled_lisp::note_idle(None);
    }

    pub(crate) fn redisplay_inhibited(&mut self) -> Result<bool> {
        let slot = self.slot("inhibit-redisplay");
        Ok(self.variable_value(slot)? != Value::NIL)
    }

    /// The event a keyboard read returns at once for a quit already
    /// pending (`Vm::take_read_quit'), if any.
    fn read_quit_event(&mut self) -> Option<HostEvent> {
        match self.take_read_quit() {
            quit::ReadQuit::Key => Some(HostEvent::Key(CommandEvent::Character(QUIT_CHAR))),
            quit::ReadQuit::Thrown => Some(HostEvent::Timeout),
            quit::ReadQuit::None => None,
        }
    }

}

/// A host wait in progress (`wait_for_event`): the host, moved out of
/// `host_input` so a nested read and the collector see none, as before; the
/// read's time limit; and, once it came, the event that ends the wait.
pub(crate) struct HostWait {
    host: Box<dyn HostInput>,
    deadline: Option<std::time::Instant>,
    event: Option<HostEvent>,
}

/// The waiting host (`HostWait`).
fn host(vm: &mut Vm) -> &mut dyn HostInput {
    vm.host_wait.as_mut().expect("a host wait in progress").host.as_mut()
}

/// What a host wait's deadline depends on besides the clock, as
/// `romacs--wait-state` (`lisp/romacs/vm/keyboard.el`) prints it: empty before the
/// Lisp half is installed.
const WAIT_STATE: &str = "(lambda () (and (fboundp 'romacs--wait-state) (romacs--wait-state)))";
/// The echo area brought up to date (`romacs--echo-sync`): whether it changed.
const ECHO_SYNC: &str = "(lambda () (and (fboundp 'romacs--echo-sync) (romacs--echo-sync)))";

/// `romacs--read-host-event` &optional SECONDS NODISPLAY, lent from its
/// engine as `load` is (`Nested::Loader`): redisplay unless NODISPLAY or
/// input is pending, then wait for one event, or nil once SECONDS pass; its
/// value goes to OUT. Lisp runs timers between reads (`lisp/romacs/vm/keyboard.el`).
/// Orchestration: the Lisp the wait runs (the wait-state probe, echo-sync,
/// image errors' log lines, redisplay, a resize) is requests, the rest leaves.
pub(crate) fn wait_for_event<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, args: &RootFrame, out: Out<'_>) -> Result<()> {
    let Some(display) = m.leaf(args, take_host)? else { return Ok(()) };
    let waited = wait(m, gc.by_ref(), display);
    m.leaf((waited, out), finish)
}

/// The read's arguments; with a host attached, the host moved to the wait
/// (`HostWait`) and whether to redisplay, else None and the read is nil.
/// Moved, not borrowed: a read cannot re-enter itself, because no Lisp runs
/// inside the wait but its own (a redisplay's), which sees no host and so
/// end of input, nil.
fn take_host(vm: &mut Vm, args: &RootFrame) -> Result<Option<bool>> {
    let args = vm.rooted_values(args).to_vec();
    if args.len() > 2 {
        return Err("wrong primitive arity".into());
    }
    let display = args.get(1).is_none_or(|&v| v == Value::NIL);
    let deadline = match args.first() {
        None => None,
        Some(&v) if v == Value::NIL => None,
        Some(&v) => {
            let seconds = match (v.integer_value(), vm.object_if(v)) {
                (Some(n), _) => n as f64,
                (None, Some(crate::data::Object::Float(n))) => *n,
                _ => return Err("romacs--read-host-event: SECONDS is not a number".into()),
            };
            let wait = std::time::Duration::try_from_secs_f64(seconds.clamp(0.0, 1e9)).unwrap_or_default();
            Some(crate::host::monotonic() + wait)
        }
    };
    let Some(host) = vm.host_input.take() else { return Ok(None) };
    vm.host_wait = Some(HostWait { host, deadline, event: None });
    Ok(Some(display))
}

/// The wait (GNU `read_char`'s redisplay, then `wait_reading_process_output`),
/// done once `HostWait::event` holds what ends it.
fn wait<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, display: bool) -> Result<()> {
    // A C-g that came before the read is the quit character at once, as
    // read_char returns it.
    if m.leaf((), |vm, ()| vm.read_quit_event().is_some_and(|event| ended(vm, event))) {
        return Ok(());
    }
    let mut redraw = display;
    loop {
        redraw |= m.leaf((), |vm, ()| vm.poll_terminal_output())?;
        // An image decoded off the VM landed: the frame shows it.
        redraw |= land_images(m, gc.by_ref())?;
        // So did the parse a redisplay left a window plain for.
        redraw |= m.leaf((), |vm, ()| vm.land_treesit());
        // GNU redisplay_internal does nothing while `inhibit-redisplay'
        // is non-nil, as it is around Lisp that redisplay itself calls,
        // and waits for pending input; but GNU's `message' displays the
        // echo area at once, input pending or not (xdisp.c
        // message3_nolog, echo_area_display): a message left during
        // this wait (a timer's) is shown, and painted, before the input
        // that may clear it is read.
        if redraw && !m.leaf((), |vm, ()| vm.redisplay_inhibited())? {
            // The deadline was computed from the timers and the unread
            // queues; the echo area's functions (a toast's expiry) or
            // a redisplay's Lisp (a mode-line :eval) may change them,
            // and then the wait ends for Lisp to compute it again.
            let before = wait_state(m, gc.by_ref())?;
            let echoed = threaded::call_source(m, gc.by_ref(), ECHO_SYNC, None, (), |_, (), echoed| echoed.map(|v| v != Value::NIL))?;
            if echoed || !m.leaf((), |vm, ()| host(vm).input_pending()) {
                threaded::redisplay(m, gc.by_ref(), (), |vm, (), (started, frame)| paint(vm, started, frame))?;
                redraw = false;
            }
            if wait_state(m, gc.by_ref())? != before {
                m.leaf((), |vm, ()| ended(vm, HostEvent::Timeout));
                return Ok(());
            }
        }
        // The paint sent a bounded share of new images: paint the rest
        // before waiting, while no input waits and redisplay may run.
        if m.leaf((), |vm, ()| {
            let behind = { let host = host(vm); host.images_behind() && !host.input_pending() };
            Ok::<_, String>(behind && !vm.redisplay_inhibited()?)
        })? {
            redraw = true;
            continue;
        }
        // A user signal `debug-on-event' names, which woke this wait:
        // the command reading quits into the debugger now, as GNU's
        // enters it at once, not at the read's next key.
        m.leaf((), |vm, ()| if vm.take_debug_event() { vm.poll_quit() } else { Ok(()) })?;
        let (resize, done) = m.leaf((), next_event)?;
        if let Some(size) = resize {
            threaded::resize(m, gc.by_ref(), size)?;
            redraw = true;
        }
        if done {
            return Ok(());
        }
    }
}

/// The wait-state probe's printed value (`WAIT_STATE`).
fn wait_state<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>) -> Result<String> {
    threaded::call_source(m, gc, WAIT_STATE, None, (), |vm, (), state| state.and_then(|v| vm.display(v)))
}

/// `Vm::land_images` within the wait: the decodes that finished landed, and
/// each loader's error logged to *Messages* unshown, its Lisp a request.
/// Whether any decode finished.
fn land_images<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>) -> Result<bool> {
    let (messages, landed) = m.leaf((), |vm, ()| vm.image_completions());
    for message in messages {
        let frame = m.leaf(message.as_str(), |vm, message| {
            let text = vm.allocate_object(crate::data::Object::String(Box::new(crate::data::LispString::new(message))));
            vm.root_frame(&[text])
        });
        let logged = threaded::call_source(m, gc.by_ref(), crate::prim::IMAGE_ERROR_LOG, Some(&frame), (), |_, (), logged| logged.map(|_| ()));
        m.leaf(frame, Vm::drop_frame);
        logged?;
    }
    Ok(landed)
}

/// The frame frozen at START (`threaded::redisplay`), handed to the host to
/// paint, and the layout it reports applied.
fn paint(vm: &mut Vm, started: std::time::Instant, frame: Result<romacs_layout_input::FrameSnapshot>) -> Result<()> {
    let frame = frame?;
    let snapshot = crate::host::monotonic().saturating_duration_since(started);
    let selected = vm.selected_window_value()?;
    let selected = vm.window(selected)?.id;
    let minibuffer = vm.window(vm.mini_window())?.id;
    let text_animations = vm.text_animations()?;
    let ends = host(vm).redisplay(HostFrame { frame, selected, minibuffer, snapshot, text_animations })?;
    vm.display_apply_layout(&ends)?;
    vm.redisplay_done(crate::host::monotonic().saturating_duration_since(started));
    Ok(())
}

/// The host's next event, waited for with the evaluator idle: a C-g typed now
/// is this read's key, not a quit (quit.rs), while the Lisp around the wait
/// (the redisplay's) stays quittable, as GNU's is. A resize to apply, and
/// whether the wait is over, its event in `HostWait::event`.
fn next_event(vm: &mut Vm, (): ()) -> Result<(Option<(u32, u32)>, bool)> {
    let deadline = vm.host_wait.as_ref().expect("a host wait in progress").deadline;
    let timeout = deadline.map(|end| end.saturating_duration_since(crate::host::monotonic()));
    let fds = vm.process_wait_fds();
    host(vm).watch_processes(&fds);
    let busy = vm.quit.busy();
    for _ in 0..vm.quit.set_busy(false) {
        vm.handle_interrupt();
    }
    let event = match vm.read_quit_event() {
        Some(event) => Ok(event),
        None => host(vm).next_event(timeout),
    };
    vm.quit.set_busy(busy);
    Ok(match event? {
        HostEvent::Resize { width, height } => (Some((width, height)), false),
        // A completed background command ends the wait as a timeout does:
        // Lisp may run its callback between commands
        // (lisp/romacs/vm/keyboard.el `romacs--read-host-char').
        HostEvent::Background if vm.background_processes_ready() => (None, ended(vm, HostEvent::Timeout)),
        HostEvent::Background => (None, false),
        // Lisp reads it (lisp/romacs/vm/keyboard.el `romacs--read-host-char').
        HostEvent::Process => (None, ended(vm, HostEvent::Timeout)),
        event => (None, ended(vm, event)),
    })
}

/// EVENT ends the wait.
fn ended(vm: &mut Vm, event: HostEvent) -> bool {
    vm.host_wait.as_mut().expect("a host wait in progress").event = Some(event);
    true
}

/// The wait's end, whatever ended it: the host back in `host_input`, then
/// the event as the read's value in OUT.
fn finish(vm: &mut Vm, (waited, out): (Result<()>, Out<'_>)) -> Result<()> {
    let HostWait { host, event, .. } = vm.host_wait.take().expect("a host wait in progress");
    vm.host_input = Some(host);
    waited?;
    let event = event.expect("a wait ends with its event");
    // kbd_buffer_store_event: input inside `while-no-input' sets
    // `quit-flag' to `throw-on-input', for the next safepoint to throw.
    if let HostEvent::Key(_) = event {
        vm.note_input_for_throw();
    }
    let value = match event {
        HostEvent::Key(CommandEvent::Character(c)) => Value::integer(c)?,
        HostEvent::Key(CommandEvent::Named(name)) => vm.intern(&name),
        HostEvent::Timeout => Value::NIL,
        // make_lispy_focus_in / make_lispy_focus_out: (focus-in FRAME).
        HostEvent::Focus(focused) => {
            let kind = vm.intern(if focused { "focus-in" } else { "focus-out" });
            let frame = vm.frame_value_peek();
            vm.list_value(&[kind, frame])
        }
        // A native model's event: the special event `(romacs-model ID TYPE PLIST)`.
        HostEvent::Model { id, event } => vm.model_event_value(id, &event),
        HostEvent::Resize { .. } | HostEvent::Background | HostEvent::Process => unreachable!("background events are consumed by the wait"),
        HostEvent::Closed => {
            // Its data tells it from a C-g's (`romacs--host-event').
            let (quit, closed) = (vm.intern("quit"), vm.intern("frame-closed"));
            vm.propagated_escape = Some(threaded::Escape::Signal(quit, closed));
            return Err("quit: frame closed while reading input".into());
        }
    };
    vm.set_rooted(out.0, out.1, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Keys(Vec<i64>);
    impl HostInput for Keys {
        fn input_pending(&mut self) -> bool { true }
        fn redisplay(&mut self, _: HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(Vec::new()) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<HostEvent, String> {
            Ok(HostEvent::Key(CommandEvent::Character(self.0.remove(0))))
        }
    }

    /// Keys whose every redisplay takes the next of its durations.
    struct SlowPaint(Vec<i64>, Vec<u64>);
    impl HostInput for SlowPaint {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> {
            std::thread::sleep(std::time::Duration::from_millis(self.1.remove(0)));
            Ok(Vec::new())
        }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<HostEvent, String> {
            Ok(HostEvent::Key(CommandEvent::Character(self.0.remove(0))))
        }
    }

    /// `romacs--redisplay-since-mark' answers the first redisplay after its
    /// mark, not a later one, and nil after a new mark until one runs.
    #[test]
    fn the_first_redisplay_after_a_mark_is_timed() {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.set_host_input(Some(Box::new(SlowPaint(vec![120, 121], vec![5, 40]))));
        let got = vm
            .eval_source("(list (romacs--redisplay-since-mark t) (romacs--redisplay-since-mark)
                                (progn (romacs--read-host-event) (romacs--read-host-event) (romacs--redisplay-since-mark t))
                                (romacs--redisplay-since-mark))")
            .map(|v| v.display().unwrap())
            .unwrap_or_else(|e| format!("VM-ERROR {e}"));
        let words: Vec<&str> = got.trim_matches(|c| c == '(' || c == ')').split(' ').collect();
        assert_eq!((words[0], words[1], words[3]), ("nil", "nil", "nil"), "{got}");
        let first: u64 = words[2].parse().expect(&got);
        assert!((5_000..40_000).contains(&first), "{got}");
    }

    /// A collection in a prompt's redisplay runs with the host taken for the
    /// read, so no key can be seen: its record says so rather than nothing.
    #[test]
    fn a_collection_in_a_prompts_redisplay_is_recorded_as_met() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.set_host_input(Some(Box::new(SlowPaint(vec![120], vec![0]))));
        vm.eval_source("(setq mode-line-format '(:eval (progn (garbage-collect) \"m\")))").unwrap();
        vm.take_collections();
        vm.idle_since = Some(crate::host::monotonic());
        vm.eval_source("(romacs--read-host-event)").unwrap();
        let taken = vm.take_collections();
        let got = vm.display(taken).unwrap();
        assert!(got.contains(" redisplay "), "{got}");
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

    /// A host that answers with EVENTS in turn, no input ever pending, so
    /// every read redisplays first.
    struct Events(std::collections::VecDeque<HostEvent>);
    impl HostInput for Events {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(Vec::new()) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<HostEvent, String> {
            self.0.pop_front().ok_or_else(|| "host script exhausted".into())
        }
    }
    fn waiting(events: Vec<HostEvent>) -> Vm {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.set_host_input(Some(Box::new(Events(events.into()))));
        vm
    }
    fn shown(vm: &mut Vm, source: &str) -> String {
        vm.eval_source(source).map_or_else(|e| format!("VM-ERROR {e}"), |v| v.display().unwrap())
    }

    /// The wait's own Lisp (the wait-state probe, echo-sync) and a
    /// redisplay's run as requests and may collect: the read still answers.
    #[test]
    fn the_waits_lisp_may_collect() {
        let mut vm = waiting(vec![HostEvent::Key(CommandEvent::Character(120))]);
        shown(&mut vm, "(progn (defun romacs--wait-state () (garbage-collect) (format \"%S\" (list 1 2)))
                               (defun romacs--echo-sync () (garbage-collect) nil)
                               (setq mode-line-format '(:eval (progn (garbage-collect) \"m\"))))");
        vm.set_gc_stress(0);
        let before = vm.heap_stats().collections;
        assert_eq!(shown(&mut vm, "(list (romacs--read-host-event) (make-string 2 ?k))"), "(120 \"kk\")");
        assert!(vm.heap_stats().collections > before, "the wait's Lisp collected");
        assert!(vm.host_input.is_some(), "the host is back after the read");
    }

    /// A resize while waiting resizes the root window (Lisp, a request) and
    /// the frame, then redisplays and goes on waiting.
    #[test]
    fn a_resize_during_the_wait_resizes_and_waits_on() {
        let mut vm = waiting(vec![HostEvent::Resize { width: 1000, height: 700 }, HostEvent::Key(CommandEvent::Character(120))]);
        vm.set_gc_stress(0);
        assert_eq!(shown(&mut vm, "(romacs--read-host-event)"), "120");
        assert_eq!((vm.window_state.width, vm.window_state.height), (1000, 700));
    }

    /// A closed frame ends the wait with `quit`, its data telling it from a
    /// C-g's, and the host is back afterwards.
    #[test]
    fn a_closed_frame_quits_the_wait() {
        let mut vm = waiting(vec![HostEvent::Closed]);
        assert_eq!(shown(&mut vm, "(condition-case e (romacs--read-host-event) (quit e))"), "(quit . frame-closed)");
        assert!(vm.host_input.is_some(), "the host is back after the quit");
    }

    /// A read from a redisplay's Lisp, inside a wait, sees no host and reads
    /// nil; the outer read still gets its key.
    #[test]
    fn a_nested_read_sees_no_host() {
        let mut vm = waiting(vec![HostEvent::Key(CommandEvent::Character(120))]);
        shown(&mut vm, "(setq nested 'unread mode-line-format '(:eval (progn (setq nested (list (romacs--read-host-event))) \"m\")))");
        assert_eq!(shown(&mut vm, "(list (romacs--read-host-event) nested)"), "(120 (nil))");
        assert!(vm.host_input.is_some(), "the host is back after the read");
    }
}
