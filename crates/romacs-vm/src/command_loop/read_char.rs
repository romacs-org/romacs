//! keyboard.c `read_char`, with the per-source rules of the Lisp port it
//! replaces (DESIGN §2.2's table): `unread-post-input-method-events`,
//! `unread-command-events`, `unread-input-method-events`, the executing
//! keyboard macro, `unread_switch_frame`, then the host keyboard or the
//! batch wait, whose Lisp (`romacs--read-host-input`,
//! `romacs--read-batch-input`) runs timers, processes and special events.
//! Then the input method, the record and the command keys. And
//! `read_filtered_event`, behind `read-event` and `read-char`.
use super::{Done, M, MAIN, Store, call, call_bound, signal, step, store, var};
use crate::brand::Gc;
use crate::gc::RootFrame;
use crate::prim::{Kind, Leaf, ReadCall};
use crate::{Value, Vm};

/// A `read_char` call: wait at most SECONDS (then no event); the input
/// method may see a printable first character (METHOD, GNU's nil
/// `PREV_EVENT`); a key sequence is being read (`KEYMAP_READ`).
#[derive(Clone, Copy)]
pub(super) struct CharArgs {
    pub(super) seconds: Option<f64>,
    pub(super) method: bool,
    pub(super) keymap_read: bool,
}

/// What `read_char` knows of the event it took, between its steps.
#[derive(Clone, Copy, Default)]
#[expect(clippy::struct_excessive_bools, reason = "`read_char`'s own flags, one field each as the Lisp port's locals")]
struct Taken {
    reread: bool,
    recorded: bool,
    post: bool,
    again: bool,
}
impl Taken {
    fn pack(self) -> [bool; 4] {
        [self.reread, self.recorded, self.post, self.again]
    }
    fn unpack([reread, recorded, post, again]: [bool; 4]) -> Self {
        Self { reread, recorded, post, again }
    }
}

/// Where an event comes from, once the queues and the macro had none.
const QUEUED: u8 = 0;
const HOST: u8 = 1;
const BATCH: u8 = 2;

/// `read_char` for the activation AT: the next event in its result (nil
/// when SECONDS passed), recorded as input; whether it was the quit
/// character typed on the host keyboard (keyboard.c `handle_interrupt`'s C-g).
pub(super) fn read_char<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, args: CharArgs) -> Done<bool> {
    // The event and the message before it, rooted across the read's Lisp.
    let own = m.leaf((), |vm, ()| vm.root_frame(&[Value::NIL, Value::NIL]));
    let outcome = read(m, gc.by_ref(), at, &own, args);
    m.leaf((at, own, outcome), |vm, (at, own, outcome)| {
        let [event, _] = vm.release_frame(own);
        if outcome.is_ok() { vm.reads.act(at).result = event; }
        outcome
    })
}

fn read<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame, mut args: CharArgs) -> Done<bool> {
    // Clamped as the host wait clamps it (`host_input.rs`): a huge or
    // infinite SECONDS bounds nothing it could reach.
    let end = args.seconds.and_then(|seconds| std::time::Duration::try_from_secs_f64(seconds.clamp(0.0, 1e9)).ok())
        .map(|wait| crate::host::monotonic() + wait);
    let mut quit = false;
    let taken = loop {
        let (source, packed) = step(m, gc.by_ref(), at, (at, own), take_queued)?;
        let mut taken = Taken::unpack(packed);
        if source == HOST {
            call(m, gc.by_ref(), at, args.seconds, |vm, seconds| Ok(vec![vm.intern("romacs--read-host-input"), seconds_value(vm, seconds)]))?;
            let keyboard_quit;
            (taken, keyboard_quit) = host_event(m, gc.by_ref(), at, own)?;
            quit = keyboard_quit;
        } else if source == BATCH {
            call(m, gc.by_ref(), at, args.seconds, |vm, seconds| Ok(vec![vm.intern("romacs--read-batch-input"), seconds_value(vm, seconds)]))?;
            taken.again = m.leaf((at, own), |vm, (at, own)| {
                let event = vm.reads.act(at).result;
                let again = event == Value::TRUE;
                vm.set_rooted(own, 0, if again { Value::NIL } else { event });
                again
            });
        }
        if !taken.again && m.leaf((own, taken.post, args.method), method_due) {
            let (called, gave) = method(m, gc.by_ref(), at, own, args.keymap_read)?;
            // A C-g the method read was the method's key.
            quit &= !called;
            if !gave {
                taken.again = true;
                // SECONDS bound the whole read, the method's time included.
                args.seconds = end.map(|end| end.saturating_duration_since(crate::host::monotonic()).as_secs_f64());
            }
        }
        if !taken.again { break taken; }
    };
    finish(m, gc, at, own, taken, args.seconds.is_none())?;
    Ok(quit)
}

/// SECONDS as Lisp: a float, or nil.
fn seconds_value(vm: &mut Vm, seconds: Option<f64>) -> Value {
    seconds.map_or(Value::NIL, |seconds| Leaf::new(vm).float(seconds))
}

/// The requeued and macro sources, in `read_char`'s order: an event taken
/// from one of them, or the source to wait on.
fn take_queued(vm: &mut Vm, (at, own): (usize, &RootFrame)) -> Done<(u8, [bool; 4])> {
    let mut taken = Taken::default();
    let event = if let Some(event) = pop(vm, at, "unread-post-input-method-events")? {
        (taken.reread, taken.recorded, taken.post) = (true, true, true);
        unwrap_menu_event(vm, event)
    } else if let Some(event) = pop(vm, at, "unread-command-events")? {
        // (t . EVENT) is read as new input; (no-record . EVENT) was recorded.
        let cx = Leaf::new(vm);
        let (head, tail) = cx.pair(event).unwrap_or((Value::NIL, Value::NIL));
        let mut cx = Leaf::new(vm);
        let event = if cx.pair(event).is_some() && head == Value::TRUE {
            tail
        } else {
            taken.reread = true;
            if cx.pair(event).is_some() && head == cx.intern("no-record") {
                taken.recorded = true;
                tail
            } else {
                event
            }
        };
        let cx = Leaf::new(vm);
        match cx.pair(event) {
            Some((head, tail)) if tail == Leaf::new(vm).intern("disabled") && symbol_or_integer(vm, head) => head,
            _ => event,
        }
    } else if let Some(event) = pop(vm, at, "unread-input-method-events")? {
        taken.reread = true;
        unwrap_menu_event(vm, event)
    } else if var(vm, "executing-kbd-macro") != Value::NIL && !at_end_of_macro(vm) {
        macro_event(vm, at)?
    } else {
        let event = vm.take_unread_switch_frame();
        if event == Value::NIL {
            let host = var(vm, "romacs--host-input") != Value::NIL;
            return Ok((if host { HOST } else { BATCH }, taken.pack()));
        }
        event
    };
    vm.set_rooted(own, 0, event);
    Ok((QUEUED, taken.pack()))
}

/// The head of the list in the variable NAME, which keeps the rest; None
/// when it is empty. The write is the leaf's one: what follows it is the
/// leaf's own state.
fn pop(vm: &mut Vm, at: usize, name: &str) -> Done<Option<Value>> {
    let list = var(vm, name);
    let Some((event, rest)) = Leaf::new(vm).pair(list) else { return Ok(None) };
    store(vm, at, name, rest)?;
    Ok(Some(event))
}

fn symbol_or_integer(vm: &mut Vm, value: Value) -> bool {
    value.integer_value().is_some() || Leaf::new(vm).kind(value) == Kind::Symbol
}

/// `read_char`'s undoing of `read_char_x_menu_prompt`: (EVENT) is EVENT.
fn unwrap_menu_event(vm: &mut Vm, event: Value) -> Value {
    match Leaf::new(vm).pair(event) {
        Some((head, Value::NIL)) if symbol_or_integer(vm, head) => head,
        _ => event,
    }
}

/// macros.c `at_end_of_macro_p`, while `executing-kbd-macro` is non-nil.
pub(super) fn at_end_of_macro(vm: &mut Vm) -> bool {
    let macro_ = var(vm, "executing-kbd-macro");
    let index = var(vm, "executing-kbd-macro-index").integer_value().unwrap_or(0);
    macro_ == Value::TRUE || usize::try_from(index).is_ok_and(|index| index >= macro_length(vm, macro_))
}

fn macro_length(vm: &mut Vm, macro_: Value) -> usize {
    let cx = Leaf::new(vm);
    match cx.kind(macro_) {
        Kind::String | Kind::Bytes => cx.string_shape(macro_).map_or(0, |(chars, _)| chars),
        Kind::Vector => cx.vector_len(macro_).unwrap_or(0),
        _ => 0,
    }
}

/// The executing macro's next event; a string's 0x80 bit is meta.
fn macro_event(vm: &mut Vm, at: usize) -> Done<Value> {
    let macro_ = var(vm, "executing-kbd-macro");
    let index = var(vm, "executing-kbd-macro-index").integer_value().unwrap_or(0);
    let i = usize::try_from(index).unwrap_or(0);
    let cx = Leaf::new(vm);
    let event = if matches!(cx.kind(macro_), Kind::String | Kind::Bytes) {
        let c = super::byte_char(i64::from(cx.string_char(macro_, i).ok().flatten().unwrap_or(0)));
        let c = if c & 0x80 != 0 && c <= 0xff { 0x800_0000 | (c & 0x7f) } else { c };
        Value::integer(c).unwrap_or(Value::NIL)
    } else {
        cx.vector_get(macro_, Value::integer(index).unwrap_or(Value::NIL)).map_err(|fault| signal(vm, fault))?
    };
    store(vm, at, "executing-kbd-macro-index", Value::integer(index + 1).unwrap_or(Value::NIL))?;
    Ok(event)
}

/// Whether `read_char` has a requeued event: in any of its unread queues, or
/// a `switch-frame` a focus change queued.
pub(super) fn unread_p(vm: &mut Vm) -> bool {
    ["unread-post-input-method-events", "unread-command-events", "unread-input-method-events"]
        .iter().any(|name| var(vm, name) != Value::NIL)
        || vm.unread_switch_frame_waits()
}

/// The host wait's answer, `(EVENT RECORDED QUIT)` or nil, in the
/// activation's result, in the Lisp port's order: the event through
/// `keyboard-translate-table` and recorded (`record`), then what the echo
/// area held for an input method, then the echo area cleared for it but for
/// help, frame and window selection events (keyboard.c:3231). What it took,
/// and whether it was the quit character.
fn host_event<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame) -> Done<(Taken, bool)> {
    let (packed, quit) = m.leaf((at, own), |vm, (at, own)| {
        let answer = vm.reads.act(at).result;
        let parts = vm.list_values(answer).unwrap_or_default();
        let event = parts.first().copied().unwrap_or(Value::NIL);
        let mut taken = Taken { recorded: parts.get(1).is_some_and(|&v| v != Value::NIL), ..Taken::default() };
        let quit = parts.get(2).is_some_and(|&v| v != Value::NIL);
        let event = if event == Value::NIL { event } else { keyboard_translate(vm, event) };
        vm.set_rooted(own, 0, event);
        if event == Value::NIL { taken.again = unread_p(vm); }
        (taken.pack(), quit)
    });
    let mut taken = Taken::unpack(packed);
    if m.leaf(own, |vm, own| vm.rooted(own, 0) == Value::NIL) { return Ok((taken, quit)); }
    // An input method sees what was recorded go in, not its own output.
    if !taken.recorded { record(m, gc.by_ref(), at, own)?; }
    taken.recorded = true;
    // What the echo area held, for an input method to refer to.
    step(m, gc.by_ref(), at, (at, own), |vm, (at, own)| {
        let event = vm.rooted(own, 0);
        if var(vm, "input-method-function") == Value::NIL || !printable(event) { return Ok(()); }
        let message = vm.current_message().unwrap_or(Value::NIL);
        vm.set_rooted(own, 1, message);
        store(vm, at, "input-method-previous-message", message)
    })?;
    // Then the echo area as it is now, which that write's watcher may have changed.
    let clear = m.leaf(own, |vm, own| {
        let event = vm.rooted(own, 0);
        vm.current_message().unwrap_or(Value::NIL) != Value::NIL && !echo_kept(vm, event)
    });
    if clear { call(m, gc, at, (), |vm, ()| Ok(vec![vm.intern("romacs--clear-echo")]))?; }
    Ok((taken, quit))
}

/// keyboard.c `record_char` of the event, in the Lisp port's order: lossage,
/// the count of input events, then the macro being defined stores it
/// (macros.c `store_kbd_macro_char`); none of it for keyboard macro input.
fn record<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame) -> Done<()> {
    let recorded = step(m, gc.by_ref(), at, (at, own), |vm, (at, own)| {
        if var(vm, "executing-kbd-macro") != Value::NIL || var(vm, "inhibit--record-char") != Value::NIL { return Ok(false); }
        vm.record_recent_value(vm.rooted(own, 0), false);
        let count = var(vm, "num-nonmacro-input-events").integer_value().unwrap_or(0);
        store(vm, at, "num-nonmacro-input-events", Value::integer(count + 1).unwrap_or(Value::NIL))?;
        Ok(true)
    })?;
    if recorded && m.leaf((), |vm, ()| var(vm, "defining-kbd-macro") != Value::NIL) {
        call(m, gc, at, own, |vm, own| Ok(vec![vm.intern("romacs--store-kbd-macro-char"), vm.rooted(own, 0)]))?;
    }
    Ok(())
}

/// A help, frame or window selection event, which keeps the echo area.
fn echo_kept(vm: &mut Vm, event: Value) -> bool {
    let Some((head, _)) = Leaf::new(vm).pair(event) else { return false };
    let mut cx = Leaf::new(vm);
    ["help-echo", "switch-frame", "select-window"].iter().any(|name| head == cx.intern(name))
}

/// keyboard.c `read_char`: a host character through
/// `keyboard-translate-table`; a nil entry, or one beyond a string or
/// vector table, is no translation.
fn keyboard_translate(vm: &mut Vm, event: Value) -> Value {
    let table = var(vm, "keyboard-translate-table");
    let Some(c) = event.integer_value().filter(|&c| c >= 0) else { return event };
    let cx = Leaf::new(vm);
    let entry = match cx.kind(table) {
        Kind::CharTable if c <= i64::from(crate::lstring::MAX_CHAR) => {
            cx.char_table_element(table, usize::try_from(c).unwrap_or(0)).unwrap_or(Value::NIL)
        }
        Kind::String | Kind::Bytes => usize::try_from(c).ok().and_then(|at| cx.string_char(table, at).ok().flatten())
            .map_or(Value::NIL, |c| Value::integer(i64::from(c)).unwrap_or(Value::NIL)),
        Kind::Vector => cx.vector_get(table, event).unwrap_or(Value::NIL),
        _ => Value::NIL,
    };
    if entry == Value::NIL { event } else { entry }
}

/// A printable one-byte character, which an input method may translate.
fn printable(event: Value) -> bool {
    event.integer_value().is_some_and(|c| (32..256).contains(&c) && c != 127)
}

/// `reread_for_input_method`: whether the input method sees the event.
fn method_due(vm: &mut Vm, (own, post, method): (&RootFrame, bool, bool)) -> bool {
    method && !post && var(vm, "input-method-function") != Value::NIL && printable(vm.rooted(own, 0))
}

/// The input method on the event: `list` only wipes a message; another
/// gives the events it returns, the first read now and the rest next
/// (`unread-post-input-method-events`), or none, and the message comes
/// back and the read goes on. Its own reads make up keys of their own, so
/// the command's are saved and emptied first, before the echo area is
/// cleared for it, and put back after; a C-g it read was its key. Whether it
/// was called, and whether it gave an event.
fn method<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame, keymap_read: bool) -> Done<(bool, bool)> {
    let list = m.leaf((), |vm, ()| var(vm, "input-method-function") == Leaf::new(vm).intern("list"));
    if list {
        if m.leaf((), |vm, ()| vm.current_message().unwrap_or(Value::NIL) != Value::NIL) {
            call(m, gc, at, (), |vm, ()| Ok(vec![vm.intern("romacs--clear-echo")]))?;
        }
        return Ok((false, true));
    }
    // The command's keys, kept across the method's own reads.
    let saved = m.leaf((), |vm, ()| {
        let keys = vm.streams().keys(MAIN).to_vec();
        let start = vm.streams().single_key_start(MAIN);
        let keys = Leaf::new(vm).vector(&keys);
        vm.streams_mut().store(MAIN, Store::Keys(&[], 0));
        (vm.root_frame(&[keys]), start)
    });
    let called = method_call(m, gc.by_ref(), at, own, keymap_read);
    let gave = step(m, gc.by_ref(), at, (at, saved, called.is_ok(), own), |vm, (at, (keys, start), ok, own)| {
        let [keys] = vm.release_frame(keys);
        if !ok { return Err(()); }
        let keys = Leaf::new(vm).vector_values(keys).unwrap_or_default();
        vm.streams_mut().store(MAIN, Store::Keys(&keys, start));
        queue_rest(vm, at, own)
    });
    called?;
    if !gave? {
        // No event: the message comes back.
        let previous = m.leaf(own, |vm, own| vm.rooted(own, 1) != Value::NIL);
        if previous {
            call_bound(m, gc, at, &["message-log-max"], own, |_, _| Ok(vec![Value::NIL]), |vm, own| {
                let format = Leaf::new(vm).string("%s");
                Ok(vec![vm.intern("message"), format, vm.rooted(own, 1)])
            })?;
        }
        return Ok((true, false));
    }
    Ok((true, true))
}

/// The echo area cleared for the method when a message shows, then the
/// method called with its two variables bound: its events in the
/// activation's result.
fn method_call<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame, keymap_read: bool) -> Done<()> {
    if m.leaf((), |vm, ()| vm.current_message().unwrap_or(Value::NIL) != Value::NIL) {
        call(m, gc.by_ref(), at, (), |vm, ()| Ok(vec![vm.intern("romacs--clear-echo")]))?;
    }
    let names: &'static [&'static str] = &["input-method-use-echo-area", "romacs--reading-command-keys"];
    call_bound(m, gc, at, names, (own, keymap_read), |vm, (_, keymap_read)| {
        let echo = var(vm, "input-method-use-echo-area") != Value::NIL || !keymap_read;
        Ok(vec![if echo { Value::TRUE } else { Value::NIL }, Value::NIL])
    }, |vm, (own, _)| Ok(vec![var(vm, "input-method-function"), vm.rooted(own, 0)]))
}

/// The method's events: the first is the event now, and the rest go before
/// `unread-post-input-method-events` as the Lisp port put them, with `nconc`
/// itself (`fns_seq::nconc_values`): the method's own list extended, a
/// non-list tail or a circular one refused, quit polled. Whether it gave an
/// event.
fn queue_rest(vm: &mut Vm, at: usize, own: &RootFrame) -> Done<bool> {
    let events = vm.reads.act(at).result;
    let cx = Leaf::new(vm);
    let Some((first, rest)) = cx.pair(events) else { return Ok(false) };
    vm.set_rooted(own, 0, first);
    let queued = var(vm, "unread-post-input-method-events");
    let tail = if rest == Value::NIL {
        queued
    } else {
        crate::prim::prims::fns_seq::nconc_values(&mut Leaf::new(vm), &[rest, queued]).map_err(|fault| signal(vm, fault))?
    };
    store(vm, at, "unread-post-input-method-events", tail)?;
    Ok(true)
}

/// After every source, in the Lisp port's order: recorded unless it was
/// (`record`), then a command key unless the read was timed (UNTIMED false)
/// or a reread with keys already there, then `last-input-event`.
fn finish<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, own: &RootFrame, taken: Taken, untimed: bool) -> Done<()> {
    if m.leaf(own, |vm, own| vm.rooted(own, 0) == Value::NIL) { return Ok(()); }
    if !taken.recorded { record(m, gc.by_ref(), at, own)?; }
    step(m, gc, at, (at, own, taken.reread, untimed), |vm, (at, own, reread, untimed)| {
        let event = vm.rooted(own, 0);
        if untimed && (!reread || vm.streams().keys(MAIN).is_empty()) {
            vm.streams_mut().store(MAIN, Store::PushKey(event));
        }
        store(vm, at, "last-input-event", event)
    })
}

/// keyboard.c `read_filtered_event` behind `read-event` (ASCII nil),
/// `read-char` (ASCII and ERROR-NONASCII) and `read-char-exclusive` (ASCII):
/// PROMPT INHERIT-INPUT-METHOD SECONDS.
pub(crate) fn read_filtered_event(call: ReadCall<'_, '_, '_, '_>, ascii: bool, error_nonascii: bool) -> bool {
    let place = super::place(&call);
    let ReadCall { m, gc, out, .. } = call;
    super::activation(m, gc, out, |m, mut gc, at| {
        super::ensure_lisp_half(m, gc.by_ref(), at)?;
        filtered(m, gc, at, place, (ascii, error_nonascii))?;
        m.leaf((at, out, ascii), |vm, (at, out, ascii)| {
            // `Fread_char`: a character's modifier bits resolved, or nil.
            let value = match vm.reads.act(at).result {
                event if !ascii => event,
                event => event.integer_value()
                    .and_then(|c| Value::integer(crate::prim::prims::keyboard::resolve_modifier_mask(c)).ok())
                    .unwrap_or(Value::NIL),
            };
            vm.set_rooted(out, 0, value);
        });
        Ok(())
    })
}

fn filtered<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, place: (usize, usize), (ascii, error_nonascii): (bool, bool)) -> Done<()> {
    let (prompt, seconds, method) = m.leaf(place, |vm, place| {
        if var(vm, "inhibit-interaction") != Value::NIL {
            let condition = vm.intern("inhibited-interaction");
            signal(vm, crate::prim::Fault::Signal(condition, Value::NIL));
            return Err(());
        }
        let prompt = super::arg(vm, place, 0);
        let seconds = super::arg(vm, place, 2);
        let seconds = seconds.integer_value().map(|n| if n <= 0 { 0.0 } else { i32::try_from(n).map_or(1e9, f64::from) }).or_else(|| Leaf::new(vm).float_value(seconds).ok());
        Ok(((prompt != Value::NIL).then(|| vm.root_frame(&[prompt])), seconds, super::arg(vm, place, 1) != Value::NIL))
    })?;
    // `message_with_string`'s echo, which is not logged (keyboard.c:10776).
    if let Some(prompt) = prompt {
        let shown = call_bound(m, gc.by_ref(), at, &["message-log-max"], &prompt, |_, _| Ok(vec![Value::NIL]), |vm, prompt| {
            let format = Leaf::new(vm).string("%s");
            Ok(vec![vm.intern("message"), format, vm.rooted(prompt, 0)])
        });
        m.leaf(prompt, Vm::drop_frame);
        shown?;
    }
    let delayed = m.leaf((), |vm, ()| vm.root_frame(&[Value::NIL]));
    let outcome = filter_events(m, gc.by_ref(), at, &delayed, (seconds, method, ascii, error_nonascii));
    m.leaf((delayed, outcome), |vm, (delayed, outcome)| {
        let [event] = vm.release_frame(delayed);
        if outcome.is_ok() && event != Value::NIL { vm.set_unread_switch_frame(event); }
        outcome
    })
}

/// Events read until one passes: with ASCII, a `switch-frame` is put off
/// (DELAYED) and a non-character either skipped or, with ERROR-NONASCII,
/// requeued with an error. The event in the activation's result.
fn filter_events<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, delayed: &RootFrame,
    (seconds, method, ascii, error_nonascii): (Option<f64>, bool, bool, bool)) -> Done<()> {
    loop {
        let quit = read_char(m, gc.by_ref(), at, CharArgs { seconds, method, keymap_read: false })?;
        // `read_char`'s quit: `maybe_quit` acts on a C-g before the event is returned.
        let verdict = step(m, gc.by_ref(), at, ((at, delayed), quit, seconds.is_some(), (ascii, error_nonascii)),
            |vm, ((at, delayed), quit, timed, (ascii, error_nonascii))| {
            if quit && var(vm, "inhibit-quit") == Value::NIL {
                let condition = vm.intern("quit");
                signal(vm, crate::prim::Fault::Signal(condition, Value::NIL));
                return Err(());
            }
            let event = vm.reads.act(at).result;
            if !ascii || (timed && event == Value::NIL) { return Ok(0); }
            if switch_frame_event(vm, event) {
                vm.set_rooted(delayed, 0, event);
                return Ok(1);
            }
            let event = ascii_character(vm, event);
            vm.reads.act(at).result = event;
            if event.integer_value().is_some() { return Ok(0); }
            if !error_nonascii { return Ok(1); }
            let queued = Leaf::new(vm).cons(event, Value::NIL);
            store(vm, at, "unread-command-events", queued)?;
            Ok(2)
        })?;
        match verdict {
            0 => return Ok(()),
            1 => {}
            _ => {
                call(m, gc, at, (), |vm, ()| {
                    let mut cx = Leaf::new(vm);
                    Ok(vec![cx.intern("error"), cx.string("Non-character input-event")])
                })?;
                return Err(());
            }
        }
    }
}

fn switch_frame_event(vm: &mut Vm, event: Value) -> bool {
    let Some((head, _)) = Leaf::new(vm).pair(event) else { return false };
    head == Leaf::new(vm).intern("switch-frame")
}

/// A function key with an ASCII equivalent is that character, with the
/// symbol's modifiers; a GUI key (`C-M-m`) names its whole character.
fn ascii_character(vm: &mut Vm, event: Value) -> Value {
    let mut cx = Leaf::new(vm);
    if cx.kind(event) != Kind::Symbol || event == Value::NIL { return event; }
    let ascii = cx.intern("ascii-character");
    let own = cx.symbol_property(event, ascii).unwrap_or(Value::NIL);
    if own.integer_value().is_some() { return own; }
    let mask_key = cx.intern("event-symbol-element-mask");
    let mask = cx.symbol_property(event, mask_key).unwrap_or(Value::NIL);
    let Some((base, rest)) = cx.pair(mask) else { return event };
    let modifiers = cx.pair(rest).map_or(Value::NIL, |(modifiers, _)| modifiers);
    let character = cx.symbol_property(base, ascii).unwrap_or(Value::NIL);
    match (character.integer_value(), modifiers.integer_value()) {
        (Some(c), Some(bits)) => Value::integer(c | bits).unwrap_or(event),
        _ => event,
    }
}
