//! keyboard.c `read_key_sequence`, as the Lisp port had it (its per-source
//! rules, translation order, fallbacks, fake prefixes, delayed
//! `switch-frame`, prefix help and reader hooks), but following each event
//! through the maps taken at GNU's three points (DESIGN §2.2): when the
//! sequence is replayed, when its first real event arrives, and when the
//! current buffer changed while `read_char` waited.
//!
//! A read's state is its `KeyseqFrame`, in its activation on `Reads`, so a
//! read nested in a filter, a translation or an input method leaves it as it
//! is and the collector traces and moves what it holds. A leaf makes at most
//! one write to a watched variable, as its last effect, in the Lisp port's
//! order, so each watcher runs where the Lisp reader's `setq` ran it.
use super::keyremap::{self, FKEY, KEYTRAN, Scan};
use super::read_char::{self, CharArgs};
use super::{Done, M, MAIN, Store, call, escape, signal, step, store, var};
use crate::brand::Gc;
use crate::gc::RootFrame;
use crate::keymap::{KeyWalk, Walked};
use crate::prim::{Kind, Leaf, ReadCall};
use crate::{Value, Vm, threaded};

/// Where a read is in GNU's loop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// `replay_sequence`: start over from the keys read so far.
    Replay,
    /// Reading events while the sequence is a prefix.
    Read,
    /// The executing keyboard macro ended: no keys.
    End,
    /// An unbound help key after a prefix: `prefix-help-command`.
    Help,
}

/// What a taken prefix help key restores (keyboard.el's checkpoint).
struct Checkpoint {
    n: usize,
    mock: usize,
    binding: Value,
    maps: Value,
    maps_at: Option<u64>,
    first_taken: bool,
    first_unbound: usize,
    scans: [Scan; 3],
    upper: Option<(usize, Value)>,
    shift_translated: bool,
    fake_prefixed: usize,
    nonmenu: Value,
    /// The command keys, kept: a read the help function makes replaces them.
    keys: Vec<Value>,
    single_start: usize,
    raw: usize,
    macro_events: Value,
}

/// One live `read_key_sequence` (DESIGN §2.4).
#[expect(clippy::struct_excessive_bools, reason = "`read_key_sequence`'s own flags, one field each as GNU's locals")]
pub(crate) struct KeyseqFrame {
    /// keybuf: N events read, those before MOCK reread first; it holds
    /// max (N, MOCK) events.
    pub(super) keybuf: Vec<Value>,
    pub(super) n: usize,
    mock: usize,
    /// The stream's key count when the read began.
    keys_start: usize,
    /// GNU's `current_binding`: the composite of the active maps, then what
    /// each event followed to.
    pub(super) binding: Value,
    /// The active maps at point, current while the reader's call count is
    /// `maps_at` (no Lisp ran since they were taken).
    maps: Value,
    maps_at: Option<u64>,
    /// GNU's `first_event` is known: the maps are taken.
    first_taken: bool,
    first_unbound: usize,
    pub(super) scans: [Scan; 3],
    fake_prefixed: Vec<Value>,
    upper: Option<(usize, Value)>,
    shift_translated: bool,
    delayed: Value,
    checkpoint: Option<Box<Checkpoint>>,
    clean: bool,
    raw: Vec<Value>,
    /// The event at hand.
    key: Value,
    help: Value,
    started: bool,
    state: State,
    command: bool,
    dont_downcase_last: bool,
    can_return_switch_frame: bool,
}

impl KeyseqFrame {
    fn new((dont_downcase_last, command, can_return_switch_frame): (bool, bool, bool)) -> Self {
        let scan = Scan::new(Value::NIL);
        Self { keybuf: Vec::new(), n: 0, mock: 0, keys_start: 0, binding: Value::NIL, maps: Value::NIL, maps_at: None,
               first_taken: false, first_unbound: 0, scans: [scan; 3], fake_prefixed: Vec::new(), upper: None,
               shift_translated: false, delayed: Value::NIL, checkpoint: None, clean: false, raw: Vec::new(), key: Value::NIL,
               help: Value::NIL, started: false, state: State::Replay, command, dont_downcase_last, can_return_switch_frame }
    }
    /// Every Value the read holds, for the collector to mark and rewrite.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        self.keybuf.iter_mut().chain(self.fake_prefixed.iter_mut()).chain(self.raw.iter_mut()).for_each(&mut *visit);
        for v in [&mut self.binding, &mut self.maps, &mut self.delayed, &mut self.key, &mut self.help] { visit(v); }
        if let Some((_, event)) = &mut self.upper { visit(event); }
        for scan in &mut self.scans { scan.visit_values(visit); }
        if let Some(c) = &mut self.checkpoint {
            for v in [&mut c.binding, &mut c.maps, &mut c.nonmenu, &mut c.macro_events] { visit(v); }
            c.keys.iter_mut().for_each(&mut *visit);
            if let Some((_, event)) = &mut c.upper { visit(event); }
            for scan in &mut c.scans { scan.visit_values(visit); }
        }
    }
}

/// The key sequence the activation AT reads.
pub(super) fn frame(vm: &mut Vm, at: usize) -> &mut KeyseqFrame {
    vm.reads.act(at).keyseq.as_mut().expect("an activation reading a key sequence")
}

/// `romacs--read-key-sequence` DONT-DOWNCASE-LAST COMMAND
/// CAN-RETURN-SWITCH-FRAME, the command loop's read: the events read, or
/// nil at the end of an executing keyboard macro. It sets
/// `romacs--key-sequence-command` and `romacs--key-sequence-remapped`.
pub(crate) fn read_key_sequence(call: ReadCall<'_, '_, '_, '_>) -> bool {
    let place = super::place(&call);
    let ReadCall { m, gc, out, .. } = call;
    let flags = m.leaf(place, |vm, place| {
        let flag = |i| super::arg(vm, place, i) != Value::NIL;
        (flag(0), flag(1), flag(2))
    });
    super::activation(m, gc, out, |m, mut gc, at| {
        super::ensure_lisp_half(m, gc.by_ref(), at)?;
        read(m, gc, at, flags, out)
    })
}

/// `read-key-sequence` and `read-key-sequence-vector` (keyboard.c
/// `read_key_sequence_vs`): PROMPT CONTINUE-ECHO DONT-DOWNCASE-LAST
/// CAN-RETURN-SWITCH-FRAME CMD-LOOP, the keys as a vector, or with STRING as
/// `make_event_array` gives them.
pub(crate) fn read_key_sequence_vs(call: ReadCall<'_, '_, '_, '_>, string: bool) -> bool {
    let place = super::place(&call);
    let ReadCall { m, gc, out, .. } = call;
    super::activation(m, gc, out, |m, mut gc, at| {
        super::ensure_lisp_half(m, gc.by_ref(), at)?;
        vs(m, gc, at, place, out)?;
        m.leaf((out, string), |vm, (out, string)| {
            let keys = keys_value(vm, vm.rooted(out, 0), string)?;
            vm.set_rooted(out, 0, keys);
            Ok(())
        })
    })
}

/// The keys of a direct read, `[]` at a macro's end, as `make_event_array`
/// gives them when STRING.
fn keys_value(vm: &mut Vm, keys: Value, string: bool) -> Done<Value> {
    let cx = Leaf::new(vm);
    let events = if keys == Value::NIL { vec![] } else { cx.vector_values(keys).map_err(|fault| signal(vm, fault))? };
    let mut cx = Leaf::new(vm);
    Ok(if string { crate::prim::prims::keyboard::event_array(&mut cx, &events) } else { cx.vector(&events) })
}

/// `read_key_sequence_vs`: the prompt shown (interactive reads only) while
/// the caller's keys stand, then the keys reset unless CONTINUE-ECHO, then
/// the read with the input method's two variables bound.
fn vs<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, place: (usize, usize), out: &RootFrame) -> Done<()> {
    let (continue_echo, cmd_loop, flags, prompt) = m.leaf(place, |vm, place| {
        let prompt = super::arg(vm, place, 0);
        let cx = Leaf::new(vm);
        if prompt != Value::NIL && !matches!(cx.kind(prompt), Kind::String | Kind::Bytes) {
            signal(vm, crate::prim::Fault::Wrong("stringp", prompt));
            return Err(());
        }
        let flag = |i| super::arg(vm, place, i) != Value::NIL;
        let (continue_echo, dont_downcase_last, can_return, cmd_loop) = (flag(1), flag(2), flag(3), flag(4));
        // `read_key_sequence` (keyboard.c:10993): an interactive read shows
        // PROMPT now, unlogged.
        let show = prompt != Value::NIL && var(vm, "executing-kbd-macro") == Value::NIL && var(vm, "noninteractive") == Value::NIL;
        Ok((continue_echo, cmd_loop, (dont_downcase_last, false, can_return), show.then(|| vm.root_frame(&[prompt]))))
    })?;
    if let Some(prompt) = prompt {
        let shown = super::call_bound(m, gc.by_ref(), at, &["message-log-max"], &prompt, |_, _| Ok(vec![Value::NIL]), |vm, prompt| {
            let format = vm.make_lisp_string(b"%s".to_vec(), false);
            Ok(vec![vm.intern("message"), format, vm.rooted(prompt, 0)])
        });
        m.leaf(prompt, Vm::drop_frame);
        shown?;
    }
    if !continue_echo {
        m.leaf((), |vm, ()| vm.streams_mut().store(MAIN, Store::Keys(&[], 0)));
    }
    // Only the command loop's own reads give the input method the echo area.
    let values = m.leaf(cmd_loop, |vm, cmd_loop| {
        let flag = if cmd_loop { Value::NIL } else { Value::TRUE };
        vm.root_frame(&[flag, flag])
    });
    let names = &["input-method-exit-on-first-char", "input-method-use-echo-area"];
    super::bound(m, gc, names, values, |m, gc| read(m, gc, at, flags, out))
}

/// The activation AT reads a key sequence, its exit on every exit: the end
/// hooks once if its sequence started (the escape kept across them unless
/// they make their own, which replaces it), then its frame dropped. The keys
/// (or nil) land in OUT.
fn read<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, flags: (bool, bool, bool), out: &RootFrame) -> Done<()> {
    m.leaf((at, flags), |vm, (at, flags)| vm.reads.act(at).keyseq = Some(KeyseqFrame::new(flags)));
    let outcome = body(m, gc.by_ref(), at, out);
    // A walk an escape left at its filter, and a write it left, are dropped.
    let started = m.leaf(at, |vm, at| {
        let act = vm.reads.act(at);
        act.walk.clear();
        act.write = None;
        act.keyseq.take().is_some_and(|frame| frame.started)
    });
    let ended = if started { end_hooks(m, gc, outcome.is_err()) } else { Ok(()) };
    outcome.and(ended)
}

/// `romacs--sequence-end-functions`, with an escape pending (ESCAPING)
/// kept across them.
fn end_hooks<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, escaping: bool) -> Done<()> {
    if !m.leaf((), |vm, ()| var(vm, "romacs--sequence-end-functions") != Value::NIL) { return Ok(()); }
    let frame = m.leaf((), |vm, ()| {
        let hooks = [vm.intern("run-hooks"), vm.intern("romacs--sequence-end-functions")];
        vm.root_frame(&hooks)
    });
    let then = |vm: &mut Vm, (): (), value: crate::Result<Value>| value.map(|_| ()).map_err(|error| escape(vm, error));
    let ended = if escaping {
        threaded::call_frame_keeping_escape(m, gc, &frame, (), then)
    } else {
        threaded::call_frame(m, gc, &frame, (), then)
    };
    m.leaf(frame, |vm, frame| {
        vm.drop_frame(frame);
        vm.reads.callouts += 1;
    });
    ended
}

/// The read itself: replays until a sequence is complete.
fn body<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize, out: &RootFrame) -> Done<()> {
    m.leaf(depth, begin);
    loop {
        step(m, gc.by_ref(), depth, depth, replay)?;
        m.leaf(depth, replay_maps)?;
        while m.leaf(depth, reading) {
            if m.leaf(depth, drop_unbound) { break; }
            // An event that ended the read (a macro's end) or replays it
            // (a buffer change in the wait) is not looked up.
            if !event(m, gc.by_ref(), depth)? || !m.leaf(depth, |vm, depth| frame(vm, depth).state == State::Read) { break; }
            lookup(m, gc.by_ref(), depth)?;
        }
        if m.leaf(depth, |vm, depth| frame(vm, depth).state != State::Replay) { break; }
    }
    finish(m, gc, depth, out)
}

/// The read's start: its scans over the translation maps.
fn begin(vm: &mut Vm, depth: usize) {
    let indec = var(vm, "input-decode-map");
    let fkey = if var_bound(vm, "local-function-key-map") { var(vm, "local-function-key-map") } else { var(vm, "function-key-map") };
    let keytran = var(vm, "key-translation-map");
    let keys_start = vm.streams().keys(MAIN).len();
    vm.streams_mut().store(MAIN, Store::SingleStart(keys_start));
    let f = frame(vm, depth);
    f.keys_start = keys_start;
    f.scans = [Scan::new(indec), Scan::new(fkey), Scan::new(keytran)];
}

/// Whether the variable NAME is bound.
fn var_bound(vm: &mut Vm, name: &str) -> bool {
    let slot = vm.slot(name);
    vm.variable_value(slot).is_ok_and(|value| value != Value::UNBOUND)
}

/// `replay_sequence`: the record starts over (`last-nonmenu-event`, then
/// the command keys, in the Lisp port's order).
fn replay(vm: &mut Vm, depth: usize) -> Done<()> {
    let f = frame(vm, depth);
    f.state = State::Read;
    f.n = 0;
    f.first_unbound = usize::MAX;
    f.checkpoint = None;
    store(vm, depth, "last-nonmenu-event", Value::NIL)
}

/// After `replay`: the command keys cut back to the read's start, and with
/// events to reread the maps taken for them (GNU's first point); without,
/// the first event takes them (`event`).
fn replay_maps(vm: &mut Vm, depth: usize) -> Done<()> {
    let f = frame(vm, depth);
    let (mock, keys_start) = (f.mock, f.keys_start);
    let (first, second) = (f.keybuf.first().copied().filter(|_| mock > 0), f.keybuf.get(1).copied().filter(|_| mock > 1));
    vm.streams_mut().store(MAIN, Store::TruncateKeys(keys_start));
    if let Some(first) = first {
        return take_maps(vm, depth, first, second.unwrap_or(Value::NIL));
    }
    // Until the first event: a keymap, so the loop reads it.
    let keymap = vm.intern("keymap");
    let binding = Leaf::new(vm).cons(keymap, Value::NIL);
    let f = frame(vm, depth);
    f.binding = binding;
    f.first_taken = false;
    Ok(())
}

/// GNU `active_maps (FIRST, SECOND)`: the active maps where a click among
/// them happened, else at point, as the binding to follow.
fn take_maps(vm: &mut Vm, depth: usize, first: Value, second: Value) -> Done<()> {
    let mut cx = Leaf::new(vm);
    let position = match click_start(&mut cx, first) {
        Value::NIL if cx.kind(first) == Kind::Symbol && first != Value::NIL && second != Value::NIL => click_start(&mut cx, second),
        position => position,
    };
    let maps = vm.current_active_maps(true, position).map_err(|fault| signal(vm, fault))?;
    let keymap = vm.intern("keymap");
    let binding = Leaf::new(vm).cons(keymap, maps);
    let callouts = vm.reads.callouts;
    let f = frame(vm, depth);
    f.binding = binding;
    f.first_taken = true;
    if position == Value::NIL {
        f.maps = maps;
        f.maps_at = Some(callouts);
    } else {
        f.maps_at = None;
    }
    Ok(())
}

/// `EVENT_START` of EVENT if it is a mouse click with a position, else nil
/// (as `key-binding` finds the click it looks a key up at).
fn click_start(cx: &mut Leaf<'_>, event: Value) -> Value {
    let Some((head, Some((start, _)))) = cx.pair(event).map(|(head, rest)| (head, cx.pair(rest))) else { return Value::NIL };
    let kind = cx.intern("event-kind");
    let click = cx.intern("mouse-click");
    if cx.symbol_property(head, kind).is_ok_and(|kind| kind == click) { start } else { Value::NIL }
}

/// Whether to read another event: while the sequence is a prefix, or
/// unbound with a translation pending.
fn reading(vm: &mut Vm, depth: usize) -> bool {
    let f = frame(vm, depth);
    if f.state != State::Read { return false; }
    let (binding, n, keytran) = (f.binding, f.n, f.scans[KEYTRAN].start);
    if binding == Value::NIL { keytran < n } else { keymapp(vm, binding) }
}

/// `keymapp`.
pub(super) fn keymapp(vm: &Vm, object: Value) -> bool {
    vm.resolve_keymap(object).is_some() || vm.autoloaded_keymap(object)
}
/// MAP as a walk follows it: a keymap, or a list of keymaps; a keymap still
/// to autoload is nothing yet, so its events are unbound, as the Lisp
/// reader's `key-binding` found them (VM2-AUTOLOAD1: GNU loads it).
pub(super) fn followable(vm: &Vm, map: Value) -> Value {
    if map.tag_is(3) || vm.resolve_keymap(map).is_some() { map } else { Value::NIL }
}

/// The prefix up to `first_unbound` is unbound with no translation left
/// to try: drop it and replay what follows it. Whether it did.
fn drop_unbound(vm: &mut Vm, depth: usize) -> bool {
    let f = frame(vm, depth);
    if f.first_unbound >= f.scans[KEYTRAN].start { return false; }
    let drop = f.first_unbound + 1;
    let n = f.n;
    f.keybuf = f.keybuf.get(drop..n).map(<[Value]>::to_vec).unwrap_or_default();
    f.mock = n - drop;
    f.state = State::Replay;
    for scan in &mut f.scans {
        let start = scan.start.saturating_sub(drop);
        scan.reset(start);
    }
    true
}

/// One event of the sequence (`replay_key`): reread, or read from
/// `read_char`, again after a delayed `switch-frame` or a fake prefix.
/// False when the executing macro ended.
fn event<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize) -> Done<bool> {
    let keys = m.leaf((), |vm, ()| vm.streams().keys(MAIN).len());
    loop {
        let fresh = m.leaf((depth, keys), |vm, (depth, keys)| {
            vm.streams_mut().store(MAIN, Store::TruncateKeys(keys));
            reread(vm, depth)
        });
        match fresh {
            Some(false) => return Ok(false),
            Some(true) => {
                let before = m.leaf(depth, prepare_read);
                if !new_event(m, gc.by_ref(), depth, before)? { continue; }
            }
            None => {}
        }
        if !m.leaf(depth, fake_prefix) { return Ok(true); }
    }
}

/// The event at N from the keys to reread (None), or whether to read a new
/// one: false at the end of an executing macro with nothing requeued.
fn reread(vm: &mut Vm, depth: usize) -> Option<bool> {
    let f = frame(vm, depth);
    if f.n < f.mock {
        let key = f.keybuf[f.n];
        f.key = key;
        f.checkpoint = None;
        vm.streams_mut().store(MAIN, Store::PushKey(key));
        return None;
    }
    if var(vm, "executing-kbd-macro") != Value::NIL && read_char::at_end_of_macro(vm) && !read_char::unread_p(vm) {
        frame(vm, depth).state = State::End;
        return Some(false);
    }
    Some(true)
}

/// Before reading a new event: a command read keeps what the event would
/// undo, should a prefix help key be taken. The current buffer, rooted, to
/// see a change in the wait.
fn prepare_read(vm: &mut Vm, depth: usize) -> RootFrame {
    let f = frame(vm, depth);
    if f.command && f.n > 0 && var(vm, "romacs--prefix-help-functions") != Value::NIL {
        let checkpoint = checkpoint(vm, depth);
        let clean = var(vm, "unread-command-events") == Value::NIL && var(vm, "executing-kbd-macro") == Value::NIL
            && var(vm, "inhibit--record-char") == Value::NIL;
        let f = frame(vm, depth);
        f.checkpoint = Some(Box::new(checkpoint));
        f.clean = clean;
    }
    let buffer = vm.slots[vm.buffer_slot].value;
    vm.root_frame(&[buffer])
}

fn checkpoint(vm: &mut Vm, depth: usize) -> Checkpoint {
    let nonmenu = var(vm, "last-nonmenu-event");
    let macro_events = if var(vm, "defining-kbd-macro") == Value::NIL { Value::NIL } else { var(vm, "romacs--kbd-macro-events") };
    let (keys, single_start) = (vm.streams().keys(MAIN).to_vec(), vm.streams().single_key_start(MAIN));
    let f = frame(vm, depth);
    Checkpoint { n: f.n, mock: f.mock, binding: f.binding, maps: f.maps, maps_at: f.maps_at, first_taken: f.first_taken,
                 first_unbound: f.first_unbound, scans: f.scans, upper: f.upper, shift_translated: f.shift_translated,
                 fake_prefixed: f.fake_prefixed.len(), nonmenu, keys, single_start, raw: f.raw.len(), macro_events }
}

/// A new event from `read_char`, then the reader hooks and, for the first,
/// its maps (GNU's second point) or, after a buffer change in the wait, a
/// replay (the third). False to read again (a delayed `switch-frame`).
fn new_event<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize, before: RootFrame) -> Done<bool> {
    let method = m.leaf((), |vm, ()| var(vm, "last-nonmenu-event") == Value::NIL);
    let read = read_char::read_char(m, gc.by_ref(), depth, CharArgs { seconds: None, method, keymap_read: true });
    let kept = step(m, gc.by_ref(), depth, (depth, read.is_ok()), |vm, (depth, ok)| {
        if !ok { return Ok(false); }
        let key = vm.reads.act(depth).result;
        let delay = (frame(vm, depth).n > 0 || !frame(vm, depth).can_return_switch_frame) && switch_frame_p(vm, key);
        let f = frame(vm, depth);
        f.key = key;
        if delay {
            f.delayed = key;
        } else {
            f.raw.push(key);
            f.keybuf.push(key);
        }
        store(vm, depth, "quit-flag", Value::NIL)?;
        Ok(!delay)
    });
    let changed = m.leaf(&before, |vm, before| vm.rooted(before, 0) != vm.slots[vm.buffer_slot].value);
    m.leaf(before, Vm::drop_frame);
    read?;
    if !kept? { return Ok(false); }
    if m.leaf((), |vm, ()| var(vm, "romacs--key-read-functions") != Value::NIL) {
        call(m, gc.by_ref(), depth, depth, |vm, depth| {
            let key = frame(vm, depth).key;
            Ok(vec![vm.intern("run-hook-with-args"), vm.intern("romacs--key-read-functions"), key])
        })?;
    }
    // Once per sequence, after its first event and before it is looked up:
    // a choice of keymaps made here holds for the whole sequence.
    if !m.leaf(depth, |vm, depth| std::mem::replace(&mut frame(vm, depth).started, true))
        && m.leaf((), |vm, ()| var(vm, "romacs--sequence-start-functions") != Value::NIL) {
        call(m, gc.by_ref(), depth, (), |vm, ()| Ok(vec![vm.intern("run-hooks"), vm.intern("romacs--sequence-start-functions")]))?;
    }
    m.leaf((depth, changed), |vm, (depth, changed)| {
        let f = frame(vm, depth);
        if !f.first_taken {
            let key = f.key;
            return take_maps(vm, depth, key, Value::NIL);
        }
        if changed {
            f.mock = f.n + 1;
            f.state = State::Replay;
        }
        Ok(())
    })?;
    Ok(true)
}

/// Whether KEY is a `switch-frame` or `select-window` event.
fn switch_frame_p(vm: &mut Vm, key: Value) -> bool {
    let cx = Leaf::new(vm);
    let Some((head, _)) = cx.pair(key) else { return false };
    let mut cx = Leaf::new(vm);
    head == cx.intern("switch-frame") || head == cx.intern("select-window")
}

/// A click in a mode line, header line or margin is looked up after its
/// area, a fake prefix key: whether the event is read again as one.
fn fake_prefix(vm: &mut Vm, depth: usize) -> bool {
    let f = frame(vm, depth);
    let (key, n) = (f.key, f.n);
    if f.state != State::Read || f.fake_prefixed.contains(&key) { return false; }
    let cx = Leaf::new(vm);
    let Some(position) = cx.pair(key).and_then(|(_, rest)| cx.pair(rest)).map(|(position, _)| position) else { return false };
    let Some((_, rest)) = cx.pair(position) else { return false };
    let area = cx.pair(rest).map_or(Value::NIL, |(area, _)| area);
    let area = cx.pair(area).map_or(area, |(area, _)| area);
    if area == Value::NIL || cx.kind(area) != Kind::Symbol { return false; }
    let f = frame(vm, depth);
    f.keybuf.truncate(n);
    f.keybuf.extend([area, key]);
    f.mock = n + 2;
    f.fake_prefixed.push(key);
    true
}

/// The event looked up: followed in the binding, the translations tried,
/// then the fallbacks: an upper-case or shifted event without its shift,
/// and an unbound help key after a prefix.
fn lookup<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize) -> Done<()> {
    walk(m, gc.by_ref(), depth, depth, |vm, depth| {
        let f = frame(vm, depth);
        let (binding, key) = (f.binding, f.key);
        Ok((followable(vm, binding), [lookup_event(vm, key), Value::NIL], 1, true))
    })?;
    // The Lisp port's order: `last-nonmenu-event`, then the single command's start.
    step(m, gc.by_ref(), depth, depth, |vm, depth| {
        let binding = vm.reads.act(depth).result;
        let f = frame(vm, depth);
        f.binding = binding;
        f.first_unbound = if binding == Value::NIL { f.first_unbound.min(f.n) } else { f.first_unbound.max(f.n + 1) };
        f.n += 1;
        let key = f.key;
        store(vm, depth, "last-nonmenu-event", key)
    })?;
    m.leaf(depth, |vm, depth| {
        let (keys, n) = (vm.streams().keys(MAIN).len(), frame(vm, depth).n);
        vm.streams_mut().store(MAIN, Store::SingleStart(keys.saturating_sub(n)));
    });
    if keyremap::translate(m, gc.by_ref(), depth)? {
        m.leaf(depth, |vm, depth| {
            let f = frame(vm, depth);
            f.mock = f.keybuf.len();
            f.state = State::Replay;
        });
        return Ok(());
    }
    fallbacks(m, gc, depth)
}

/// EVENT as the lookup reads it: one with parameters by its head, unless
/// it carries a position.
fn lookup_event(vm: &mut Vm, event: Value) -> Value {
    let cx = Leaf::new(vm);
    match cx.pair(event) {
        Some((head, rest)) if cx.pair(rest).is_none_or(|(arg, _)| cx.pair(arg).is_none()) => head,
        _ => event,
    }
}
/// EVENT as `lookup_event` reads it, from the keys being read.
pub(super) fn lookup_event_at(vm: &mut Vm, depth: usize, i: usize) -> Value {
    let event = frame(vm, depth).keybuf.get(i).copied().unwrap_or(Value::NIL);
    lookup_event(vm, event)
}

/// `read_key_sequence`'s fallbacks for an unbound event.
fn fallbacks<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, depth: usize) -> Done<()> {
    let help = m.leaf(depth, |vm, depth| {
        let f = frame(vm, depth);
        let (binding, key, n, settled) = (f.binding, f.key, f.n, f.scans[KEYTRAN].start >= f.n);
        if binding != Value::NIL { return false; }
        let lower = if var(vm, "translate-upper-case-key-bindings") == Value::NIL { None } else { unshifted(vm, key) };
        let integer = key.integer_value().is_some();
        // keyboard.c's integer upcase or shift fallback comes first.
        if let (true, Some(lower)) = (settled && integer, lower) {
            shift_down(vm, depth, key, lower, false);
            return false;
        }
        if n > 1 && help_event_p(vm, key) { return true; }
        if let (true, Some(lower)) = (settled, lower) {
            shift_down(vm, depth, key, lower, !integer);
        }
        false
    });
    if help { prefix_help(m, gc, depth) } else { Ok(()) }
}

/// The event at N-1 looked up without its shift, the original kept
/// (`upper`); a shifted function key goes through the function key map
/// again (S-backspace reaches DEL through backspace) when RESCAN.
fn shift_down(vm: &mut Vm, depth: usize, key: Value, lower: Value, rescan: bool) {
    let f = frame(vm, depth);
    let i = f.n - 1;
    f.keybuf[i] = lower;
    f.upper = Some((i, key));
    f.mock = f.n.max(f.mock);
    f.shift_translated = true;
    f.state = State::Replay;
    if rescan {
        f.scans[FKEY].reset(0);
        f.scans[KEYTRAN].reset(0);
    }
}

/// EVENT without its shift: an upper-case character's lower case, a
/// shifted one's unshifted, or a symbol's name without `S-`. None for
/// another event.
fn unshifted(vm: &mut Vm, event: Value) -> Option<Value> {
    let mut cx = Leaf::new(vm);
    if let Some(c) = event.integer_value() {
        let down = |c: i64| cx.case_convert(Value::integer(c).unwrap_or(Value::NIL), true).ok().and_then(Value::integer_value).unwrap_or(c);
        return keyremap::unshifted_char(c, down).and_then(|c| Value::integer(c).ok());
    }
    if cx.kind(event) != Kind::Symbol || event == Value::NIL { return None; }
    let name = cx.symbol_name(event).ok()?;
    keyremap::unshifted_name(&name).map(|name| cx.intern(&name))
}

/// keyboard.c `help_char_p` of the event's head: `help-char` or a member of
/// `help-event-list`.
fn help_event_p(vm: &mut Vm, event: Value) -> bool {
    let head = Leaf::new(vm).pair(event).map_or(event, |(head, _)| head);
    let (help_char, list) = (var(vm, "help-char"), var(vm, "help-event-list"));
    let cx = Leaf::new(vm);
    if cx.eql(head, help_char) { return true; }
    let mut tail = list;
    while let Some((item, rest)) = cx.pair(tail) {
        if item == head { return true; }
        tail = rest;
    }
    false
}

/// An unbound help key after a prefix: offered to
/// `romacs--prefix-help-functions` in the clean case, which take it by
/// restoring the read as it was before the key; else the sequence ends
/// with `prefix-help-command`.
fn prefix_help<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize) -> Done<()> {
    let clean = m.leaf(depth, |vm, depth| {
        let f = frame(vm, depth);
        let n = f.n;
        f.clean && f.checkpoint.as_ref().is_some_and(|c| c.n + 1 == n) && f.mock <= n && f.scans.iter().all(|scan| scan.start >= n)
    });
    let taken = clean && {
        call(m, gc.by_ref(), depth, depth, |vm, depth| {
            let f = frame(vm, depth);
            let (prefix, key) = (f.keybuf[..f.n - 1].to_vec(), f.key);
            let mut cx = Leaf::new(vm);
            Ok(vec![cx.intern("run-hook-with-args-until-success"), cx.intern("romacs--prefix-help-functions"), cx.vector(&prefix), key])
        })?;
        m.leaf(depth, |vm, depth| vm.reads.act(depth).result != Value::NIL)
    };
    if taken { return restore(m, gc, depth); }
    m.leaf(depth, |vm, depth| {
        let help = var(vm, "prefix-help-command");
        let f = frame(vm, depth);
        f.help = help;
        f.state = State::Help;
    });
    Ok(())
}

/// A taken help key: the read goes on as before it came, in the Lisp
/// port's order: the macro being defined forgets the entry this read
/// stored, the read's own state comes back, then `last-nonmenu-event`, then
/// the command keys and their start, as they were.
fn restore<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize) -> Done<()> {
    step(m, gc.by_ref(), depth, depth, |vm, depth| {
        let f = frame(vm, depth);
        let (Some(macro_events), key) = (f.checkpoint.as_ref().map(|c| c.macro_events), f.key) else { return Ok(()) };
        if var(vm, "defining-kbd-macro") == Value::NIL { return Ok(()); }
        let events = var(vm, "romacs--kbd-macro-events");
        let unstored = Leaf::new(vm).pair(events).is_some_and(|(head, rest)| head == key && rest == macro_events);
        if unstored { store(vm, depth, "romacs--kbd-macro-events", macro_events)?; }
        Ok(())
    })?;
    step(m, gc.by_ref(), depth, depth, |vm, depth| {
        let f = frame(vm, depth);
        let Some(c) = f.checkpoint.take() else { return Ok(()) };
        f.keybuf.truncate(c.n);
        f.fake_prefixed.truncate(c.fake_prefixed);
        f.raw.truncate(c.raw);
        (f.n, f.mock, f.binding, f.maps, f.maps_at, f.first_taken) = (c.n, c.mock, c.binding, c.maps, c.maps_at, c.first_taken);
        (f.first_unbound, f.scans, f.upper, f.shift_translated) = (c.first_unbound, c.scans, c.upper, c.shift_translated);
        let nonmenu = c.nonmenu;
        // The keys come back last (`restore`'s third leaf).
        f.checkpoint = Some(c);
        store(vm, depth, "last-nonmenu-event", nonmenu)
    })?;
    m.leaf(depth, |vm, depth| {
        let Some(c) = frame(vm, depth).checkpoint.take() else { return };
        vm.streams_mut().store(MAIN, Store::Keys(&c.keys, c.single_start));
    });
    Ok(())
}

/// The sequence complete, in the Lisp port's order: the shift a binding
/// did not need put back and `this-command-keys-shift-translated`; a
/// translation's events past the binding added to the keys; the delayed
/// `switch-frame` left for the next read; the raw keys, the binding and its
/// remapping for the command loop. The keys (or nil) land in OUT.
fn finish<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize, out: &RootFrame) -> Done<()> {
    step(m, gc.by_ref(), depth, depth, |vm, depth| {
        let f = frame(vm, depth);
        if f.state == State::End { return Ok(()); }
        let (n, upper, unbound) = (f.n, f.upper, f.binding == Value::NIL);
        if let Some((at, event)) = upper.filter(|&(at, _)| at + 1 == n && (f.dont_downcase_last || unbound)) {
            f.keybuf[at] = event;
            f.shift_translated = false;
        }
        if f.shift_translated { store(vm, depth, "this-command-keys-shift-translated", Value::TRUE)?; }
        Ok(())
    })?;
    let command = step(m, gc.by_ref(), depth, depth, |vm, depth| {
        let f = frame(vm, depth);
        let ended = f.state == State::End;
        if !ended {
            let rest = f.keybuf.get(f.n..f.mock).map(<[Value]>::to_vec).unwrap_or_default();
            f.n = f.n.max(f.mock);
            f.keybuf.truncate(f.n);
            for event in rest { vm.streams_mut().store(MAIN, Store::PushKey(event)); }
        }
        let f = frame(vm, depth);
        let (delayed, raw) = (f.delayed, std::mem::take(&mut f.raw));
        let command = if ended { Value::NIL } else if f.state == State::Help { f.help } else { f.binding };
        vm.set_unread_switch_frame(delayed);
        vm.streams_mut().store(MAIN, Store::RawKeys(&raw));
        vm.reads.act(depth).result = command;
        store(vm, depth, "romacs--key-sequence-command", command)?;
        Ok(command != Value::NIL && Leaf::new(vm).kind(command) == Kind::Symbol)
    })?;
    if command { remap(m, gc.by_ref(), depth)? } else { m.leaf(depth, |vm, depth| vm.reads.act(depth).result = Value::NIL) }
    step(m, gc, depth, depth, |vm, depth| {
        let remapped = vm.reads.act(depth).result;
        store(vm, depth, "romacs--key-sequence-remapped", remapped)
    })?;
    m.leaf((depth, out), |vm, (depth, out)| {
        let f = frame(vm, depth);
        let keys = (f.state != State::End).then(|| f.keybuf.clone());
        let keys = keys.map_or(Value::NIL, |keys| Leaf::new(vm).vector(&keys));
        vm.set_rooted(out, 0, keys);
    });
    Ok(())
}

/// keyboard.c `test_undefined` of the binding: nil, `undefined`, or a
/// command remapped to `undefined`.
pub(super) fn undefined<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, depth: usize) -> Done<bool> {
    let known = m.leaf(depth, |vm, depth| {
        let binding = frame(vm, depth).binding;
        let mut cx = Leaf::new(vm);
        if binding == Value::NIL || binding == cx.intern("undefined") { return Some(true); }
        if cx.kind(binding) != Kind::Symbol { return Some(false); }
        vm.reads.act(depth).result = binding;
        None
    });
    if let Some(known) = known { return Ok(known); }
    remap(m, gc, depth)?;
    Ok(m.leaf(depth, |vm, depth| { let undefined = vm.intern("undefined"); vm.reads.act(depth).result == undefined }))
}

/// `command-remapping` of the command in the activation's result, at point:
/// the remapping, or nil, in its result. The maps taken at point are
/// used again while no Lisp ran since (they would be the same).
fn remap<'b>(m: &mut M<'_, 'b>, gc: Gc<'_, 'b>, depth: usize) -> Done<()> {
    walk(m, gc, depth, depth, |vm, depth| {
        let command = vm.reads.act(depth).result;
        let callouts = vm.reads.callouts;
        let f = frame(vm, depth);
        let maps = if f.maps_at == Some(callouts) {
            f.maps
        } else {
            let maps = vm.current_active_maps(true, Value::NIL).map_err(|fault| signal(vm, fault))?;
            let f = frame(vm, depth);
            (f.maps, f.maps_at) = (maps, Some(callouts));
            maps
        };
        Ok((maps, [vm.intern("remap"), command], 2, false))
    })?;
    m.leaf(depth, |vm, depth| {
        let act = vm.reads.act(depth);
        if act.result.integer_value().is_some() { act.result = Value::NIL; }
    });
    Ok(())
}

/// The keymap walk SETUP gives (its map, one or two events and whether
/// default bindings count) run to its answer, each filter it meets called
/// where it meets it (C1w's `KeyWalk`): the binding in the activation AT's
/// result.
pub(super) fn walk<'b, A: crate::brand::Plain>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, at: usize, a: A,
    setup: impl FnOnce(&mut Vm, A) -> Done<(Value, [Value; 2], usize, bool)> + Copy + 'static) -> Done<()> {
    let mut stop = m.leaf((at, a), move |vm, (at, a)| {
        let (map, events, length, defaults) = setup(vm, a)?;
        let walked = vm.key_walk_events(map, &events[..length], defaults);
        walked_to(vm, at, walked)
    })?;
    while let Some(filter) = stop {
        super::called(m, gc.by_ref(), at, filter)?;
        stop = m.leaf(at, |vm, at| {
            let values = std::mem::take(&mut vm.reads.act(at).walk);
            let walked = KeyWalk::from_values(vm, &values).and_then(|mut walk| {
                let result = vm.reads.act(at).result;
                vm.key_walk_resume(&mut walk, result).map(|walked| (walk, walked))
            });
            walked_to(vm, at, walked)
        })?;
    }
    Ok(())
}

/// Where a walk stopped: done, its binding in the activation AT's result,
/// or at a filter, the walk kept in AT and the filter's call returned.
fn walked_to(vm: &mut Vm, at: usize, walked: crate::Result<(KeyWalk, Walked)>) -> Done<Option<RootFrame>> {
    match walked {
        Err(error) => {
            escape(vm, error);
            Err(())
        }
        Ok((_, Walked::Done(binding))) => {
            vm.reads.act(at).result = binding;
            Ok(None)
        }
        Ok((walk, Walked::Filter { filter, object })) => {
            let values = walk.values(vm);
            vm.reads.act(at).walk = values;
            let call = vm.intern("romacs--menu-filter");
            Ok(Some(vm.root_frame(&[call, filter, object])))
        }
    }
}
