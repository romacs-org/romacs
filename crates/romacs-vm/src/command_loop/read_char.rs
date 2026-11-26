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

/// `read_char`: the next event in `Reads::result` (nil when SECONDS passed),
/// recorded as input; whether it was the quit character typed on the host
/// keyboard (keyboard.c `handle_interrupt`'s C-g).
pub(super) fn read_char<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, args: CharArgs) -> Done<bool> {
    // The event and the message before it, rooted across the read's Lisp.
    let own = m.leaf((), |vm, ()| vm.root_frame(&[Value::NIL, Value::NIL]));
    let outcome = read(m, gc.by_ref(), &own, args);
    m.leaf((own, outcome), |vm, (own, outcome)| {
        let [event, _] = vm.release_frame(own);
        if outcome.is_ok() { vm.reads.result = event; }
        outcome
    })
}

fn read<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, own: &RootFrame, mut args: CharArgs) -> Done<bool> {
    let end = args.seconds.map(|seconds| crate::host::monotonic() + std::time::Duration::from_secs_f64(seconds.max(0.0)));
    let mut quit = false;
    let taken = loop {
        let (source, packed) = step(m, gc.by_ref(), own, take_queued)?;
        let mut taken = Taken::unpack(packed);
        if source == HOST {
            call(m, gc.by_ref(), args.seconds, |vm, seconds| Ok(vec![vm.intern("romacs--read-host-input"), seconds_value(vm, seconds)]))?;
            let (packed, keyboard_quit, clear, macro_store) = step(m, gc.by_ref(), own, host_event)?;
            (taken, quit) = (Taken::unpack(packed), keyboard_quit);
            if macro_store { store_macro_char(m, gc.by_ref(), own)?; }
            if clear { call(m, gc.by_ref(), (), |vm, ()| Ok(vec![vm.intern("romacs--clear-echo")]))?; }
        } else if source == BATCH {
            call(m, gc.by_ref(), args.seconds, |vm, seconds| Ok(vec![vm.intern("romacs--read-batch-input"), seconds_value(vm, seconds)]))?;
            taken.again = m.leaf(own, |vm, own| {
                let event = vm.reads.result;
                let again = event == Value::TRUE;
                vm.set_rooted(own, 0, if again { Value::NIL } else { event });
                again
            });
        }
        if !taken.again && m.leaf((own, taken.post, args.method), method_due) {
            let (called, gave) = method(m, gc.by_ref(), own, args.keymap_read)?;
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
    if step(m, gc.by_ref(), (own, taken.pack(), args.seconds.is_none()), finish)? {
        store_macro_char(m, gc, own)?;
    }
    Ok(quit)
}

/// SECONDS as Lisp: a float, or nil.
fn seconds_value(vm: &mut Vm, seconds: Option<f64>) -> Value {
    seconds.map_or(Value::NIL, |seconds| Leaf::new(vm).float(seconds))
}

/// The requeued and macro sources, in `read_char`'s order: an event taken
/// from one of them, or the source to wait on.
fn take_queued(vm: &mut Vm, own: &RootFrame) -> Done<(u8, [bool; 4])> {
    let mut taken = Taken::default();
    let event = if let Some(event) = pop(vm, "unread-post-input-method-events")? {
        (taken.reread, taken.recorded, taken.post) = (true, true, true);
        unwrap_menu_event(vm, event)
    } else if let Some(event) = pop(vm, "unread-command-events")? {
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
    } else if let Some(event) = pop(vm, "unread-input-method-events")? {
        taken.reread = true;
        unwrap_menu_event(vm, event)
    } else if var(vm, "executing-kbd-macro") != Value::NIL && !at_end_of_macro(vm) {
        macro_event(vm)?
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
/// when it is empty.
fn pop(vm: &mut Vm, name: &str) -> Done<Option<Value>> {
    let list = var(vm, name);
    let Some((event, rest)) = Leaf::new(vm).pair(list) else { return Ok(None) };
    store(vm, name, rest)?;
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
fn macro_event(vm: &mut Vm) -> Done<Value> {
    let macro_ = var(vm, "executing-kbd-macro");
    let index = var(vm, "executing-kbd-macro-index").integer_value().unwrap_or(0);
    let at = usize::try_from(index).unwrap_or(0);
    let cx = Leaf::new(vm);
    let event = if matches!(cx.kind(macro_), Kind::String | Kind::Bytes) {
        let c = super::byte_char(i64::from(cx.string_char(macro_, at).ok().flatten().unwrap_or(0)));
        let c = if c & 0x80 != 0 && c <= 0xff { 0x800_0000 | (c & 0x7f) } else { c };
        Value::integer(c).unwrap_or(Value::NIL)
    } else {
        cx.vector_get(macro_, Value::integer(index).unwrap_or(Value::NIL)).map_err(|fault| signal(vm, fault))?
    };
    store(vm, "executing-kbd-macro-index", Value::integer(index + 1).unwrap_or(Value::NIL))?;
    Ok(event)
}

/// Whether `read_char` has a requeued event: in any of its unread queues, or
/// a `switch-frame` a focus change queued.
pub(super) fn unread_p(vm: &mut Vm) -> bool {
    ["unread-post-input-method-events", "unread-command-events", "unread-input-method-events"]
        .iter().any(|name| var(vm, name) != Value::NIL)
        || vm.unread_switch_frame_waits()
}

/// The host wait's answer, `(EVENT RECORDED QUIT)` or nil, in
/// `Reads::result`: the event through `keyboard-translate-table` and
/// recorded. What it took, whether it was the quit character, whether the
/// echo area is to be cleared for it (but for help, frame and window
/// selection events, keyboard.c:3231), and whether the macro being defined
/// stores it.
fn host_event(vm: &mut Vm, own: &RootFrame) -> Done<([bool; 4], bool, bool, bool)> {
    let answer = vm.reads.result;
    let parts = vm.list_values(answer).unwrap_or_default();
    let event = parts.first().copied().unwrap_or(Value::NIL);
    let mut taken = Taken { recorded: parts.get(1).is_some_and(|&v| v != Value::NIL), ..Taken::default() };
    let quit = parts.get(2).is_some_and(|&v| v != Value::NIL);
    let event = if event == Value::NIL { event } else { keyboard_translate(vm, event) };
    vm.set_rooted(own, 0, event);
    if event == Value::NIL {
        taken.again = unread_p(vm);
        return Ok((taken.pack(), quit, false, false));
    }
    // An input method sees what was recorded go in, not its own output.
    let macro_store = !taken.recorded && record_char(vm, event)?;
    taken.recorded = true;
    // What the echo area held, for an input method to refer to.
    let message = vm.current_message().unwrap_or(Value::NIL);
    if var(vm, "input-method-function") != Value::NIL && printable(event) {
        vm.set_rooted(own, 1, message);
        store(vm, "input-method-previous-message", message)?;
    }
    Ok((taken.pack(), quit, message != Value::NIL && !echo_kept(vm, event), macro_store))
}

/// The macro being defined stores the event read (macros.c `store_kbd_macro_char`).
fn store_macro_char<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, own: &RootFrame) -> Done<()> {
    call(m, gc, own, |vm, own| Ok(vec![vm.intern("romacs--store-kbd-macro-char"), vm.rooted(own, 0)]))
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

/// keyboard.c `record_char`: lossage and the count of input events, but not
/// for keyboard macro input. Whether the macro being defined stores it.
fn record_char(vm: &mut Vm, event: Value) -> Done<bool> {
    if var(vm, "executing-kbd-macro") != Value::NIL || var(vm, "inhibit--record-char") != Value::NIL { return Ok(false); }
    vm.record_recent_value(event, false);
    let count = var(vm, "num-nonmacro-input-events").integer_value().unwrap_or(0);
    store(vm, "num-nonmacro-input-events", Value::integer(count + 1).unwrap_or(Value::NIL))?;
    Ok(var(vm, "defining-kbd-macro") != Value::NIL)
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
/// the command's are kept from them; a C-g it read was its key. Whether it
/// was called, and whether it gave an event.
fn method<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, own: &RootFrame, keymap_read: bool) -> Done<(bool, bool)> {
    let (list, message) = m.leaf((), |vm, ()| {
        let list = var(vm, "input-method-function") == Leaf::new(vm).intern("list");
        (list, vm.current_message().unwrap_or(Value::NIL) != Value::NIL)
    });
    if message { call(m, gc.by_ref(), (), |vm, ()| Ok(vec![vm.intern("romacs--clear-echo")]))?; }
    if list { return Ok((false, true)); }
    // The command's keys, kept across the method's own reads.
    let saved = m.leaf((), |vm, ()| {
        let keys = vm.streams().keys(MAIN).to_vec();
        let start = vm.streams().single_key_start(MAIN);
        let keys = Leaf::new(vm).vector(&keys);
        vm.streams_mut().store(MAIN, Store::Keys(&[], 0));
        (vm.root_frame(&[keys]), start)
    });
    let names: &'static [&'static str] = &["input-method-use-echo-area", "romacs--reading-command-keys"];
    let called = call_bound(m, gc.by_ref(), names, (own, keymap_read), |vm, (_, keymap_read)| {
        let echo = var(vm, "input-method-use-echo-area") != Value::NIL || !keymap_read;
        Ok(vec![if echo { Value::TRUE } else { Value::NIL }, Value::NIL])
    }, |vm, (own, _)| Ok(vec![var(vm, "input-method-function"), vm.rooted(own, 0)]));
    let gave = step(m, gc.by_ref(), (saved, called.is_ok(), own), |vm, ((keys, start), ok, own)| {
        let [keys] = vm.release_frame(keys);
        if !ok { return Err(()); }
        let keys = Leaf::new(vm).vector_values(keys).unwrap_or_default();
        vm.streams_mut().store(MAIN, Store::Keys(&keys, start));
        let events = vm.reads.result;
        let Some((first, rest)) = Leaf::new(vm).pair(events) else { return Ok(false) };
        vm.set_rooted(own, 0, first);
        let queued = var(vm, "unread-post-input-method-events");
        let rest = vm.list_values(rest).unwrap_or_default();
        let mut tail = queued;
        for event in rest.into_iter().rev() { tail = Leaf::new(vm).cons(event, tail); }
        store(vm, "unread-post-input-method-events", tail)?;
        Ok(true)
    });
    called?;
    if !gave? {
        // No event: the message comes back.
        let previous = m.leaf(own, |vm, own| vm.rooted(own, 1) != Value::NIL);
        if previous {
            call_bound(m, gc, &["message-log-max"], own, |_, _| Ok(vec![Value::NIL]), |vm, own| {
                let format = Leaf::new(vm).string("%s");
                Ok(vec![vm.intern("message"), format, vm.rooted(own, 1)])
            })?;
        }
        return Ok((true, false));
    }
    Ok((true, true))
}

/// After every source: recorded unless it was, a command key unless the
/// read was timed (UNTIMED false) or a reread with keys already there, and
/// `last-input-event`. Whether the macro being defined stores it.
fn finish(vm: &mut Vm, (own, taken, untimed): (&RootFrame, [bool; 4], bool)) -> Done<bool> {
    let event = vm.rooted(own, 0);
    if event == Value::NIL { return Ok(false); }
    let taken = Taken::unpack(taken);
    let macro_store = !taken.recorded && record_char(vm, event)?;
    if untimed && (!taken.reread || vm.streams().keys(MAIN).is_empty()) {
        vm.streams_mut().store(MAIN, Store::PushKey(event));
    }
    store(vm, "last-input-event", event)?;
    Ok(macro_store)
}

/// keyboard.c `read_filtered_event` behind `read-event` (ASCII nil),
/// `read-char` (ASCII and ERROR-NONASCII) and `read-char-exclusive` (ASCII):
/// PROMPT INHERIT-INPUT-METHOD SECONDS.
pub(crate) fn read_filtered_event(call: ReadCall<'_, '_, '_, '_>, ascii: bool, error_nonascii: bool) -> bool {
    let place = super::place(&call);
    let ReadCall { m, mut gc, out, .. } = call;
    let outcome = super::ensure_lisp_half(m, gc.by_ref()).and_then(|()| filtered(m, gc, place, (ascii, error_nonascii)));
    m.leaf((out, outcome, ascii), |vm, (out, outcome, ascii)| {
        // `Fread_char`: a character's modifier bits resolved, or nil.
        let value = outcome.map(|()| match vm.reads.result {
            event if !ascii => event,
            event => event.integer_value()
                .and_then(|c| Value::integer(crate::prim::prims::keyboard::resolve_modifier_mask(c)).ok())
                .unwrap_or(Value::NIL),
        });
        super::landed(vm, out, value)
    })
}

fn filtered<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, place: (usize, usize), (ascii, error_nonascii): (bool, bool)) -> Done<()> {
    let (prompt, seconds, method) = step(m, gc.by_ref(), place, |vm, place| {
        if var(vm, "inhibit-interaction") != Value::NIL {
            let condition = vm.intern("inhibited-interaction");
            signal(vm, crate::prim::Fault::Signal(condition, Value::NIL));
            return Err(());
        }
        let prompt = super::arg(vm, place, 0);
        let seconds = super::arg(vm, place, 2);
        let seconds = seconds.integer_value().map(|n| i32::try_from(n).map_or(f64::MAX, f64::from)).or_else(|| Leaf::new(vm).float_value(seconds).ok());
        Ok(((prompt != Value::NIL).then(|| vm.root_frame(&[prompt])), seconds, super::arg(vm, place, 1) != Value::NIL))
    })?;
    // `message_with_string`'s echo, which is not logged (keyboard.c:10776).
    if let Some(prompt) = prompt {
        let shown = call_bound(m, gc.by_ref(), &["message-log-max"], &prompt, |_, _| Ok(vec![Value::NIL]), |vm, prompt| {
            let format = Leaf::new(vm).string("%s");
            Ok(vec![vm.intern("message"), format, vm.rooted(prompt, 0)])
        });
        m.leaf(prompt, Vm::drop_frame);
        shown?;
    }
    let delayed = m.leaf((), |vm, ()| vm.root_frame(&[Value::NIL]));
    let outcome = filter_events(m, gc.by_ref(), &delayed, (seconds, method, ascii, error_nonascii));
    m.leaf((delayed, outcome), |vm, (delayed, outcome)| {
        let [event] = vm.release_frame(delayed);
        if outcome.is_ok() && event != Value::NIL { vm.set_unread_switch_frame(event); }
        outcome
    })
}

/// Events read until one passes: with ASCII, a `switch-frame` is put off
/// (DELAYED) and a non-character either skipped or, with ERROR-NONASCII,
/// requeued with an error. The event in `Reads::result`.
fn filter_events<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, delayed: &RootFrame,
    (seconds, method, ascii, error_nonascii): (Option<f64>, bool, bool, bool)) -> Done<()> {
    loop {
        let quit = read_char(m, gc.by_ref(), CharArgs { seconds, method, keymap_read: false })?;
        // `read_char`'s quit: `maybe_quit` acts on a C-g before the event is returned.
        let verdict = step(m, gc.by_ref(), (delayed, quit, seconds.is_some(), (ascii, error_nonascii)),
            |vm, (delayed, quit, timed, (ascii, error_nonascii))| {
            if quit && var(vm, "inhibit-quit") == Value::NIL {
                let condition = vm.intern("quit");
                signal(vm, crate::prim::Fault::Signal(condition, Value::NIL));
                return Err(());
            }
            let event = vm.reads.result;
            if !ascii || (timed && event == Value::NIL) { return Ok(0); }
            if switch_frame_event(vm, event) {
                vm.set_rooted(delayed, 0, event);
                return Ok(1);
            }
            let event = ascii_character(vm, event);
            vm.reads.result = event;
            if event.integer_value().is_some() { return Ok(0); }
            if !error_nonascii { return Ok(1); }
            let queued = Leaf::new(vm).cons(event, Value::NIL);
            store(vm, "unread-command-events", queued)?;
            Ok(2)
        })?;
        match verdict {
            0 => return Ok(()),
            1 => {}
            _ => {
                call(m, gc, (), |vm, ()| {
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
