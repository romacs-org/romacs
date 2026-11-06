//! GNU src/keyboard.c and src/macros.c: the command loop, its key reader,
//! the `read-event' family and keyboard macros.
//!
//! Tier: T2 (PRIMITIVE-DOCTRINE section 3), like prims/minibuf.rs. GNU's
//! `command_loop_1', `read_key_sequence' and `read_char' call Lisp at every
//! step (hooks, keymap lookup, the command itself), so they are Lisp here:
//! rlisp/vm/keyboard.el, the "Lisp half". Each entry point below enters it
//! with one `Step::Call', installing it on first use so a bare `Vm::new()'
//! stays bare. The Rust half keeps what reads no Lisp: the record of this
//! command's keys (`this-command-keys' and its siblings, over the traced
//! variables the Lisp half maintains), `recent-keys' recording, and `ding'.
use super::*;
use crate::prim::leaf::Kind;

/// The Lisp half, evaluated once per VM on first use, with minibuf.c's
/// (prims/minibuf.rs), whose recursive edit is this command loop, and
/// window.c's window change functions, which the loop's redisplays run
/// (host_input.rs).
const LISP_HALF: [&str; 4] = [
    super::HOOKS_LISP,
    include_str!("../../../../rlisp/vm/keyboard.el"),
    include_str!("../../../../rlisp/vm/minibuf.el"),
    include_str!("../../../../rlisp/vm/window-change.el"),
];

/// keyboard.c `this_command_keys' (a vector of the events read for this
/// command), `this_single_command_key_start' and `raw_keybuf', as traced
/// special variables: the Lisp half appends to them as it reads.
pub(crate) const COMMAND_KEYS: &str = "romacs--command-keys";
pub(crate) const SINGLE_KEY_START: &str = "romacs--single-command-key-start";
pub(crate) const RAW_KEYS: &str = "romacs--raw-command-keys";

/// What an entry point does with its helper's value: return it, convert
/// it as Fread_key_sequence does, or resolve it as Fread_char does.
pub(crate) const DONE: usize = 0;
const EVENT_ARRAY: usize = 1;
const CHAR: usize = 2;
const TAILS: [Then; 3] = [done, event_array_done, resolve_char_done];

/// Call HELPER of the Lisp half with ARGS, installing the half first.
pub(crate) fn enter(cx: &mut Leaf<'_>, helper: &str, args: Vec<Value>, tail: usize) -> Next {
    let f = cx.intern(helper);
    if cx.symbol_function(f)? != Value::NIL {
        return Ok(Step::Call { f, args, then: TAILS[tail], saved: vec![] });
    }
    let saved = [Value::integer(tail as i64).unwrap(), f].into_iter().chain(args).collect();
    install(cx, installed, saved)
}

/// Evaluate the Lisp half, then continue with THEN and SAVED.
pub(crate) fn install(cx: &mut Leaf<'_>, then: Then, saved: Vec<Value>) -> Next {
    let mut forms = Value::NIL;
    for half in LISP_HALF.iter().rev() {
        let source = cx.string(half);
        let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
        let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
        forms = cx.cons(form, forms);
    }
    let progn = cx.intern("progn");
    let form = cx.cons(progn, forms);
    let eval = cx.intern("eval");
    Ok(Step::Call { f: eval, args: vec![form, Value::TRUE], then, saved })
}

/// Whether the Lisp half is installed in this VM.
pub(crate) fn is_installed(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let probe = cx.intern("romacs--recursive-edit-1");
    Ok(cx.symbol_function(probe)? != Value::NIL)
}

/// After the one-time install, enter the helper with the saved tail.
fn installed(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let tail = saved[0].integer_value().expect("saved tail") as usize;
    Ok(Step::Call { f: saved[1], args: saved[2..].to_vec(), then: TAILS[tail], saved: vec![] })
}

/// Fread_key_sequence: the events as make_event_array gives them.
fn event_array_done(cx: &mut Leaf<'_>, keys: Value, _: &[Value]) -> Next {
    let keys = cx.vector_values(keys)?;
    Ok(Step::Done(event_array(cx, &keys)))
}

/// Fread_char's tail: a character with its modifier bits resolved where
/// they have a code of their own, or nil.
fn resolve_char_done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(match value.integer_value() {
        Some(c) => Value::integer(resolve_modifier_mask(c)).unwrap(),
        None => Value::NIL,
    }))
}

/// character.c char_resolve_modifier_mask.
fn resolve_modifier_mask(mut c: i64) -> i64 {
    const CHAR_SHIFT: i64 = 0x200_0000;
    const CHAR_CTL: i64 = 0x400_0000;
    const MODIFIER_MASK: i64 = 0xfc0_0000;
    if c & !MODIFIER_MASK >= 0x80 {
        return c;
    }
    if c & CHAR_SHIFT != 0 {
        let low = c & 0o377;
        if (b'A' as i64..=b'Z' as i64).contains(&low) {
            c &= !CHAR_SHIFT;
        } else if (b'a' as i64..=b'z' as i64).contains(&low) {
            c = (c & !CHAR_SHIFT) - (b'a' - b'A') as i64;
        } else if c & !MODIFIER_MASK <= 0x20 {
            c &= !CHAR_SHIFT;
        }
    }
    if c & CHAR_CTL != 0 {
        if c & 0o377 == b' ' as i64 {
            c &= !0o177 & !CHAR_CTL;
        } else if c & 0o377 == b'?' as i64 {
            c = 0o177 | (c & !0o177 & !CHAR_CTL);
        } else if (0o101..=0o132).contains(&(c & 0o137)) || (0o100..=0o137).contains(&(c & 0o177)) {
            c &= 0o37 | (!0o177 & !CHAR_CTL);
        }
    }
    c
}

fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

defsubr! { name = "execute-kbd-macro", gnu = "macros.c:276", effect = Global,
fn execute_kbd_macro(cx, macro_: Value; optional count: Value, loopfunc: Value) -> Step {
    enter(cx, "romacs--execute-kbd-macro", vec![macro_, count, loopfunc], DONE)
}}

defsubr! { name = "recursive-edit", gnu = "keyboard.c:781", effect = Global,
fn recursive_edit(cx) -> Step { enter(cx, "romacs--recursive-edit", vec![], DONE) }}

// command_loop_2 one command at a time, for an embedder's loop (the GUI):
// `romacs--host-command' of the Lisp half, which says what ran.
defsubr! { name = "romacs--run-host-command", gnu = "keyboard.c:1158", effect = Global,
fn run_host_command(cx) -> Step { enter(cx, "romacs--host-command", vec![], DONE) }}

// read_char's special events (focus-in, focus-out), for an embedder's loop:
// `romacs--special-event' of the Lisp half runs the event's binding.
defsubr! { name = "romacs--run-special-event", gnu = "keyboard.c:3113", effect = Global,
fn run_special_event(cx, event: Value) -> Step { enter(cx, "romacs--special-event", vec![event], DONE) }}

// cmd_error's default report: `romacs--command-error-default-function' of
// the Lisp half.
defsubr! { name = "command-error-default-function", gnu = "keyboard.c:1049", effect = Global,
fn command_error_default_function(cx, data: Value, context: Value, signal: Value) -> Step {
    enter(cx, "romacs--command-error-default-function", vec![data, context, signal], DONE)
}}

defsubr! { name = "top-level", gnu = "keyboard.c:1199", effect = Global,
fn top_level(cx) -> Step { enter(cx, "romacs--top-level", vec![], DONE) }}

defsubr! { name = "exit-recursive-edit", gnu = "keyboard.c:1223", effect = Global,
fn exit_recursive_edit(cx) -> Step { enter(cx, "romacs--exit-recursive-edit", vec![Value::NIL], DONE) }}

defsubr! { name = "abort-recursive-edit", gnu = "keyboard.c:1234", effect = Global,
fn abort_recursive_edit(cx) -> Step { enter(cx, "romacs--exit-recursive-edit", vec![Value::TRUE], DONE) }}

defsubr! { name = "read-key-sequence", gnu = "keyboard.c:11961", effect = Global,
fn read_key_sequence(cx, prompt: Value; optional continue_echo: Value, dont_downcase_last: Value,
                     can_return_switch_frame: Value, cmd_loop: Value, disable_text_conversion: Value) -> Step {
    let _ = (can_return_switch_frame, disable_text_conversion);
    enter(cx, "romacs--read-key-sequence-vs", vec![prompt, continue_echo, dont_downcase_last, cmd_loop], EVENT_ARRAY)
}}

defsubr! { name = "read-key-sequence-vector", gnu = "keyboard.c:12022", effect = Global,
fn read_key_sequence_vector(cx, prompt: Value; optional continue_echo: Value, dont_downcase_last: Value,
                            can_return_switch_frame: Value, cmd_loop: Value, disable_text_conversion: Value) -> Step {
    let _ = (can_return_switch_frame, disable_text_conversion);
    enter(cx, "romacs--read-key-sequence-vs", vec![prompt, continue_echo, dont_downcase_last, cmd_loop], DONE)
}}

// keyboard.c read_filtered_event (NO-SWITCH-FRAME, ASCII-REQUIRED, ERROR-NONASCII).
defsubr! { name = "read-event", gnu = "keyboard.c:10749", effect = Global,
fn read_event(cx; optional prompt: Value, inherit_input_method: Value, seconds: Value) -> Step {
    let _ = inherit_input_method;
    enter(cx, "romacs--read-filtered-event", vec![prompt, seconds, Value::NIL, Value::NIL], DONE)
}}

defsubr! { name = "read-char", gnu = "keyboard.c:10700", effect = Global,
fn read_char(cx; optional prompt: Value, inherit_input_method: Value, seconds: Value) -> Step {
    let _ = inherit_input_method;
    enter(cx, "romacs--read-filtered-event", vec![prompt, seconds, Value::TRUE, Value::TRUE], CHAR)
}}

defsubr! { name = "read-char-exclusive", gnu = "keyboard.c:10784", effect = Global,
fn read_char_exclusive(cx; optional prompt: Value, inherit_input_method: Value, seconds: Value) -> Step {
    let _ = inherit_input_method;
    enter(cx, "romacs--read-filtered-event", vec![prompt, seconds, Value::TRUE, Value::NIL], CHAR)
}}

/// This command's keys, `this_command_keys[0 .. this_command_key_count]'.
fn command_keys(cx: &mut Leaf<'_>) -> std::result::Result<Vec<Value>, Fault> {
    let symbol = cx.intern(COMMAND_KEYS);
    let keys = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
    if cx.kind(keys) == Kind::Vector { cx.vector_values(keys) } else { Ok(vec![]) }
}

const CHAR_META: i64 = 0x800_0000;

/// alloc.c make_event_array: a unibyte string when every event is a
/// character that fits after discarding the meta bit (which moves to 0x80),
/// else a vector.
fn event_array(cx: &mut Leaf<'_>, events: &[Value]) -> Value {
    let bytes: Option<Vec<u8>> = events.iter().map(|e| {
        let c = e.integer_value()?;
        (c & (CHAR_META - 1) < 0o200).then_some((c as u8 & 0x7f) | if c & CHAR_META != 0 { 0x80 } else { 0 })
    }).collect();
    match bytes {
        Some(bytes) => cx.make_string(bytes, false),
        None => cx.vector(events),
    }
}

defsubr! { name = "this-command-keys", gnu = "keyboard.c:12244", effect = Reads,
fn this_command_keys(cx) {
    let keys = command_keys(cx)?;
    Ok(event_array(cx, &keys))
}}

defsubr! { name = "this-command-keys-vector", gnu = "keyboard.c:12290", effect = Reads,
fn this_command_keys_vector(cx) {
    let keys = command_keys(cx)?;
    Ok(cx.vector(&keys))
}}

defsubr! { name = "this-single-command-keys", gnu = "keyboard.c:12302", effect = Reads,
fn this_single_command_keys(cx) {
    let keys = command_keys(cx)?;
    let symbol = cx.intern(SINGLE_KEY_START);
    let start = cx.variable_value(symbol)?.and_then(|v| v.integer_value()).unwrap_or(0);
    let start = usize::try_from(start).unwrap_or(0).min(keys.len());
    Ok(cx.vector(&keys[start..]))
}}

defsubr! { name = "this-single-command-raw-keys", gnu = "keyboard.c:12316", effect = Reads,
fn this_single_command_raw_keys(cx) {
    let symbol = cx.intern(RAW_KEYS);
    let keys = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
    let keys = if cx.kind(keys) == Kind::Vector { cx.vector_values(keys)? } else { vec![] };
    Ok(cx.vector(&keys))
}}

defsubr! { name = "clear-this-command-keys", gnu = "keyboard.c:12329", effect = Global,
fn clear_this_command_keys(cx; optional keep_record: Value) {
    let symbol = cx.intern(COMMAND_KEYS);
    cx.variable_set(symbol, Value::NIL)?;
    if keep_record == Value::NIL {
        cx.clear_recent_keys();
    }
    Ok(Value::NIL)
}}

defsubr! { name = "set--this-command-keys", gnu = "keyboard.c:12257", effect = Global,
fn set_this_command_keys(cx, keys: Value) {
    // Internal use (simple.el execute-extended-command): KEYS's characters
    // become this command's keys, a leading \370 spelled M-x.
    let text = cx.string_view(keys).map_err(|_| Fault::Wrong("stringp", keys))?;
    let mut events: Vec<Value> = text.chars().enumerate().map(|(i, c)| {
        let c = c as i64;
        // CHAR_BYTE8_P: a raw byte is the byte itself.
        let c = if (0x3fff80..=0x3fffff).contains(&c) { c - 0x3fff00 } else { c };
        Value::integer(if i == 0 && c == 248 { 'x' as i64 | CHAR_META } else { c }).unwrap()
    }).collect();
    if events.is_empty() {
        // GNU reads the first character unconditionally: an empty string's
        // terminating NUL.
        events.push(Value::integer(0).unwrap());
    }
    let vector = cx.vector_from(std::mem::take(&mut events));
    let symbol = cx.intern(COMMAND_KEYS);
    cx.variable_set(symbol, vector)?;
    let start = cx.intern(SINGLE_KEY_START);
    cx.variable_set(start, Value::integer(0).unwrap())?;
    Ok(Value::NIL)
}}

// keyboard.c record_char: the Lisp half records each event read, and the
// command loop each command as (nil . COMMAND).
defsubr! { name = "romacs--record-input", gnu = "keyboard.c:3476", effect = Global,
fn record_input(cx, event: Value; optional command: Value) {
    cx.record_recent_key(event, command != Value::NIL);
    Ok(Value::NIL)
}}

defsubr! { name = "ding", gnu = "dispnew.c:6854", effect = Global,
fn ding(cx; optional arg: Value) -> Step {
    // dispnew.c Fding and bitch_at_user: batch writes BEL to stdout. A
    // display with a keyboard macro running ends the macro with a
    // `user-error', unless ARG.
    let noninteractive = cx.intern("noninteractive");
    if cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) != Value::NIL {
        use std::io::Write;
        std::io::stdout().write_all(b"\x07").map_err(|e| Fault::Error(e.to_string()))?;
        return Ok(Step::Done(Value::NIL));
    }
    let macro_ = cx.intern("executing-kbd-macro");
    if arg == Value::NIL && cx.variable_value(macro_)?.unwrap_or(Value::NIL) != Value::NIL {
        let message = cx.string("Keyboard macro terminated by a command ringing the bell");
        let data = cx.cons(message, Value::NIL);
        return Err(Fault::Signal(cx.intern("user-error"), data));
    }
    // ring_bell: `ring-bell-function' when set; VM2's displays have no
    // audible or visible bell of their own.
    let function = cx.intern("ring-bell-function");
    let f = cx.variable_value(function)?.unwrap_or(Value::NIL);
    if f == Value::NIL {
        return Ok(Step::Done(Value::NIL));
    }
    Ok(Step::Call { f, args: vec![], then: done_nil, saved: vec![] })
}}

fn done_nil(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}

// VM2 queues no host input of its own: the Lisp half ends a macro being
// defined and drops `unread-command-events'.
defsubr! { name = "discard-input", gnu = "keyboard.c:12400", effect = Global,
fn discard_input(cx) -> Step { enter(cx, "romacs--discard-input", vec![], DONE) }}

// macros.c: defining a keyboard macro, whose events read_char stores
// (rlisp/vm/keyboard.el `romacs--store-kbd-macro-char').
defsubr! { name = "start-kbd-macro", gnu = "macros.c:43", effect = Global,
fn start_kbd_macro(cx, append: Value; optional no_exec: Value) -> Step {
    enter(cx, "romacs--start-kbd-macro", vec![append, no_exec], DONE)
}}

defsubr! { name = "end-kbd-macro", gnu = "macros.c:137", effect = Global,
fn end_kbd_macro(cx; optional repeat: Value, loopfunc: Value) -> Step {
    enter(cx, "romacs--end-kbd-macro-command", vec![repeat, loopfunc], DONE)
}}

defsubr! { name = "call-last-kbd-macro", gnu = "macros.c:228", effect = Global,
fn call_last_kbd_macro(cx; optional prefix: Value, loopfunc: Value) -> Step {
    enter(cx, "romacs--call-last-kbd-macro", vec![prefix, loopfunc], DONE)
}}

defsubr! { name = "store-kbd-macro-event", gnu = "macros.c:219", effect = Global,
fn store_kbd_macro_event(cx, event: Value) -> Step {
    enter(cx, "romacs--store-kbd-macro-char", vec![event], DONE)
}}

defsubr! { name = "cancel-kbd-macro-events", gnu = "macros.c:210", effect = Global,
fn cancel_kbd_macro_events(cx) -> Step { enter(cx, "romacs--cancel-kbd-macro-events", vec![], DONE) }}

defsubr! { name = "current-idle-time", gnu = "keyboard.c:4949", effect = Reads,
fn current_idle_time(cx) {
    // timer_idleness_start_time is valid only while the command loop waits
    // idle for a key (`romacs--timer-start-idle'); a running command sees nil.
    match cx.idle_since() {
        Some(since) => super::time::lisp_time(cx, since.elapsed().as_nanos() as i128),
        None => Ok(Value::NIL),
    }
}}

/// keyboard.c decode_timer: when TIMER, a timer.el vector, fires, in
/// nanoseconds since the epoch; None for anything else, or one triggered.
fn decode_timer(cx: &Leaf<'_>, timer: Value) -> Option<i128> {
    let fields = cx.vector_values(timer).ok().filter(|v| v.len() == 10)?;
    if fields[0] != Value::NIL {
        return None;
    }
    // list4_to_timespec: (HIGH LOW USEC PSEC) in picoseconds, floored to ns.
    let [high, low, usec, psec] = [fields[1], fields[2], fields[3], fields[8]].map(|v| v.integer_value().map(i128::from));
    let ps = ((high? * 65536 + low?) * 1_000_000 + usec?) * 1_000_000 + psec?;
    Some(ps.div_euclid(1000))
}

/// The nanoseconds since the epoch, as `current-time' reads them.
fn now_nanos() -> i128 {
    // The host's clock, which a world build pins.
    match crate::host::clock().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    }
}

/// The valid timer of TIMERS that fires first (the first of equals), and
/// when. timer.el keeps the lists ordered, so it is GNU's first valid timer;
/// but the copy timer_check walks keeps a repeating timer that just ran at
/// its old place with its new time, where GNU's head-only walk lets it hide
/// the ripe timers behind it until the next wait round.
fn earliest(cx: &Leaf<'_>, mut timers: Value) -> Option<(Value, i128)> {
    let mut first: Option<(Value, i128)> = None;
    while let Some((timer, rest)) = cx.pair(timers) {
        if let Some(time) = decode_timer(cx, timer)
            && first.is_none_or(|(_, earliest)| time < earliest)
        {
            first = Some((timer, time));
        }
        timers = rest;
    }
    first
}

defsubr! { name = "romacs--timer-time", gnu = "keyboard.c:4735", effect = Reads,
fn timer_time(cx, timer: Value) {
    // decode_timer, as seconds: when TIMER fires, or nil.
    Ok(match decode_timer(cx, timer) {
        Some(nanos) => cx.float(nanos as f64 / 1e9),
        None => Value::NIL,
    })
}}

defsubr! { name = "romacs--timer-next", gnu = "keyboard.c:4763", effect = Reads,
fn timer_next(cx, timers: Value, idle_timers: Value) {
    // timer_check_2's choice between the earliest of the ordinary TIMERS
    // and of the IDLE-TIMERS: the timer to run, a ripe one, else the
    // seconds until the next becomes ripe, or nil if none is valid. Running it is the Lisp caller's (subprocess.el
    // `romacs--timer-check-2'). An idle timer's time counts from the start
    // of idleness, zero when Emacs is not idle.
    let now = now_nanos();
    let idleness = cx.idle_since().map_or(0, |since| since.elapsed().as_nanos() as i128);
    // (TIMER, ripe, distance from its clock) of each list's earliest timer.
    let head = |list, clock: i128| earliest(cx, list).map(|(timer, time)| (timer, time <= clock, (clock - time).abs()));
    let chosen = match (head(timers, now), head(idle_timers, idleness)) {
        (None, None) => return Ok(Value::NIL),
        (Some(timer), None) | (None, Some(timer)) => timer,
        (Some(ordinary), Some(idle)) => {
            let ordinary_first = idle.1 < ordinary.1
                || (idle.1 == ordinary.1 && if ordinary.1 { idle.2 < ordinary.2 } else { ordinary.2 < idle.2 });
            if ordinary_first { ordinary } else { idle }
        }
    };
    Ok(match chosen {
        (timer, true, _) => timer,
        (_, false, distance) => cx.float(distance as f64 / 1e9),
    })
}}

defsubr! { name = "romacs--set-idle", gnu = "keyboard.c:4695", effect = Global,
fn set_idle(cx, idle: Value) {
    // The state timer_start_idle (IDLE non-nil) and timer_stop_idle set;
    // rlisp/vm/subprocess.el `romacs--timer-start-idle' is the rest.
    cx.set_idle_since((idle != Value::NIL).then(crate::host::monotonic));
    Ok(Value::NIL)
}}

// kbd_buffer_get_event's USER_SIGNAL_EVENT: a pending SIGUSR1 or SIGUSR2
// as the event read_char reads next, or nil. One `debug-on-event' names is
// no event: as GNU's handle_user_signal does, it sets `quit-flag' with
// `debug-on-quit', so the read returns the quit into the debugger.
defsubr! { name = "romacs--take-user-signal", gnu = "keyboard.c:8561", effect = Global,
fn take_user_signal(cx) {
    match crate::host::take_user_signal() {
        Some(event) if cx.debugs_on_event(event) => { cx.debug_event(); Ok(Value::NIL) }
        Some(event) => Ok(cx.intern(event)),
        None => Ok(Value::NIL),
    }
}}

defsubr! { name = "input-pending-p", gnu = "keyboard.c:12076", effect = Reads,
fn input_pending_p(cx; optional check_timers: Value) {
    // requeued_events_pending_p, then the host keyboard's readable events.
    let _ = check_timers;
    let unread = cx.intern("unread-command-events");
    let pending = cx.variable_value(unread)?.unwrap_or(Value::NIL) != Value::NIL || cx.host_input_pending();
    Ok(if pending { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "current-input-mode", gnu = "keyboard.c:12967", effect = Reads,
fn current_input_mode(cx) {
    // A frame that is not a tty's, as every VM2 frame is (VM2-WIN5):
    // interrupt-driven input, no flow control, 8-bit Meta, C-g to quit.
    let items = [Value::TRUE, Value::NIL, Value::TRUE, Value::integer(7).map_err(Fault::Error)?];
    Ok(items.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail)))
}}

defsubr! { name = "set-input-meta-mode", gnu = "keyboard.c:12852", effect = Reads,
fn set_input_meta_mode(_cx, meta: Value; optional terminal: Value) {
    // decode_tty_terminal finds no tty in VM2, so nothing changes.
    let _ = (meta, terminal);
    Ok(Value::NIL)
}}
#[cfg(test)]
mod tests {
    use crate::Vm;

    fn printed(vm: &mut Vm, source: &str) -> String {
        let value = vm.eval_source_raw(source).unwrap();
        vm.display(value).unwrap()
    }

    /// timer_check_2's choice: invalid and triggered timers are skipped, a
    /// ripe one is returned to run, an ordinary timer and an idle one are
    /// compared by ripeness and distance, and otherwise the seconds until
    /// the next one fires.
    #[test]
    fn timer_next_chooses_as_timer_check_2() {
        let mut vm = Vm::new();
        // A timer.el vector TRIGGERED-P HIGH LOW USEC REPEAT FUNCTION ARGS IDLE PSECS, firing at SECONDS (relative to NOW).
        vm.eval_source_raw("(defun at (seconds name &optional now) (let* ((s (+ (or now (float-time)) seconds)) (whole (floor s))) (vector nil (/ whole 65536) (% whole 65536) (floor (* 1e6 (- s whole))) nil name nil nil 0 nil)))").unwrap();
        assert_eq!(printed(&mut vm, "(list (romacs--timer-next nil nil) (romacs--timer-next '(junk [1 2]) nil))"), "(nil nil)");
        assert_eq!(printed(&mut vm, "(aref (romacs--timer-next (list 'junk (at -5 'past) (at 5 'later)) nil) 5)"), "past");
        assert_eq!(printed(&mut vm, "(let ((ripe (at -5 'triggered))) (aset ripe 0 t) (aref (romacs--timer-next (list ripe (at -1 'next)) nil) 5))"), "next");
        assert_eq!(printed(&mut vm, "(let ((wait (romacs--timer-next (list (at 30 'later)) nil))) (and (floatp wait) (< 29 wait 30.01)))"), "t");
        // A repeating timer that ran is re-armed in place in timer_check's
        // copy of the list; it must not hide the ripe timer behind it.
        assert_eq!(printed(&mut vm, "(aref (romacs--timer-next (list (at 0.4 'rearmed) (at -0.1 'ripe)) nil) 5)"), "ripe");
        // Not idle: idle timers count from zero idleness, so one due at
        // 0 s of idleness is ripe and one due at 10 s is not.
        assert_eq!(printed(&mut vm, "(aref (romacs--timer-next (list (at 60 'ordinary)) (list (vector nil 0 0 0 nil 'idle nil t 0 nil))) 5)"), "idle");
        assert_eq!(printed(&mut vm, "(let ((wait (romacs--timer-next (list (at 60 'ordinary)) (list (vector nil 0 10 0 nil 'idle nil t 0 nil))))) (and (floatp wait) (< 9.9 wait 10.01)))"), "t");
        // Both ripe: the one ripe longest runs first.
        assert_eq!(printed(&mut vm, "(aref (romacs--timer-next (list (at -1 'ordinary)) (list (vector nil 0 0 0 nil 'idle nil t 0 nil))) 5)"), "ordinary");
        assert_eq!(printed(&mut vm, "(romacs--timer-time (at 0 'x 1000.5))"), "1000.5");
        assert_eq!(printed(&mut vm, "(romacs--timer-time [t 0 1 0 nil x nil nil 0 nil])"), "nil");
    }
}

/// keyboard.c modifier_names: the symbol for each modifier bit.
const MODIFIER_NAMES: [(u32, &str); 12] = [(0, "up"), (1, "down"), (2, "drag"), (3, "click"), (4, "double"), (5, "triple"),
    (22, "alt"), (23, "super"), (24, "hyper"), (25, "shift"), (26, "control"), (27, "meta")];

// keyboard.c parse_modifiers: parse SYMBOL's name once, caching
// (BASE MASK) on its `event-symbol-element-mask' and (BASE MODIFIERS...)
// on its `event-symbol-elements', and return the latter.
defsubr! { name = "internal-event-symbol-parse-modifiers", gnu = "keyboard.c:7609", effect = Global,
fn internal_event_symbol_parse_modifiers(cx, symbol: Value) {
    let (mask_key, elements_key) = (cx.intern("event-symbol-element-mask"), cx.intern("event-symbol-elements"));
    let cached = cx.symbol_property(symbol, mask_key)?;
    if cx.pair(cached).is_none() {
        let name = cx.symbol_name(symbol)?;
        let (modifiers, start) = crate::keymap::parse_event_modifiers(name.as_bytes());
        let base = cx.intern(&name[start..]);
        let mask = Value::integer(modifiers).expect("modifier bits are a fixnum");
        let mask = cx.cons(mask, Value::NIL);
        let mask = cx.cons(base, mask);
        cx.put_symbol_property(symbol, mask_key, mask)?;
        // lispy_modifier_list: consed from the lowest bit, so highest first.
        let elements = MODIFIER_NAMES.iter().filter(|(bit, _)| modifiers & 1 << bit != 0)
            .fold(Value::NIL, |list, (_, name)| { let name = cx.intern(name); cx.cons(name, list) });
        let elements = cx.cons(base, elements);
        cx.put_symbol_property(symbol, elements_key, elements)?;
    }
    cx.symbol_property(symbol, elements_key)
}}

// A deadline on one call the host admits, an agent's evaluation
// (rlisp/romacs-agent-tools.el): MILLISECONDS after it is armed the call is
// quit at its next safepoint, as by C-g, unless it ended first. GENERATION
// names the call; nil disarms.
defsubr! { name = "romacs--call-deadline", gnu = "romacs:keyboard", effect = Global,
fn call_deadline(cx, generation: Value; optional milliseconds: Value) { cx.call_deadline(generation, milliseconds) }}

defsubr! { name = "char-resolve-modifiers", gnu = "character.c:883", effect = Pure,
fn char_resolve_modifiers(_cx, character: Value) {
    let c = character.integer_value().ok_or(Fault::Wrong("fixnump", character))?;
    Ok(Value::integer(resolve_modifier_mask(c)).unwrap())
}}
