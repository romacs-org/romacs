//! keyboard.c's translation scans (`keyremap`, `keyremap_step`) and the
//! order `read_key_sequence` runs them in: `input-decode-map` over every
//! sequence, the function key map over an undefined one, and
//! `key-translation-map` over any, each scanning only what the map before it
//! has finished with. The scans are a read's (`KeyseqFrame::scans`); a
//! lookup that meets a filter and a translation function are Lisp calls.
use super::keyseq::{self, frame, keymapp};
use super::{Done, M, call_bound, signal, step};
use crate::brand::Gc;
use crate::prim::{Kind, Leaf};
use crate::{Value, Vm};

pub(super) const INDEC: usize = 0;
pub(super) const FKEY: usize = 1;
pub(super) const KEYTRAN: usize = 2;

/// A keyboard.c `keyremap`: a scan through the translation map PARENT, at
/// MAP, the submap the events from START to END lead to.
#[derive(Clone, Copy)]
pub(super) struct Scan {
    pub(super) parent: Value,
    pub(super) map: Value,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Scan {
    pub(super) fn new(map: Value) -> Self {
        Self { parent: map, map, start: 0, end: 0 }
    }
    /// The scan starts again at START, from its whole map.
    pub(super) fn reset(&mut self, start: usize) {
        self.map = self.parent;
        self.start = start;
        self.end = start;
    }
    /// The scan moves by DIFF events, after a translation before it.
    pub(super) fn shift(&mut self, diff: isize) {
        self.start = self.start.saturating_add_signed(diff);
        self.end = self.end.saturating_add_signed(diff);
    }
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.parent);
        visit(&mut self.map);
    }
}

/// The translations tried once N events are read (`read_key_sequence`'s
/// three loops). True when one was made: the keys replaced, and the scans
/// of the maps before it moved by its change of length.
pub(super) fn translate<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize) -> Done<bool> {
    while m.leaf(depth, |vm, depth| { let f = frame(vm, depth); f.scans[INDEC].end < f.n }) {
        if remap_step(m, gc.by_ref(), depth, INDEC, true)?.is_some() { return Ok(true); }
    }
    // A complete binding with no decoding pending holds no function key.
    let complete = !m.leaf(depth, |vm, depth| { let binding = frame(vm, depth).binding; keymapp(vm, binding) })
        && !keyseq::undefined(m, gc.by_ref(), depth)?
        && m.leaf(depth, |vm, depth| { let f = frame(vm, depth); f.scans[INDEC].start >= f.n });
    if complete {
        m.leaf(depth, |vm, depth| {
            let f = frame(vm, depth);
            if f.scans[FKEY].start < f.n { let n = f.n; f.scans[FKEY].reset(n); }
        });
    } else {
        while m.leaf(depth, |vm, depth| { let f = frame(vm, depth); f.scans[FKEY].end < f.scans[INDEC].start }) {
            let last = m.leaf(depth, |vm, depth| { let f = frame(vm, depth); f.scans[FKEY].end + 1 == f.n });
            let doit = last && keyseq::undefined(m, gc.by_ref(), depth)?;
            if let Some(diff) = remap_step(m, gc.by_ref(), depth, FKEY, doit)? {
                m.leaf((depth, diff), |vm, (depth, diff)| frame(vm, depth).scans[INDEC].shift(diff));
                return Ok(true);
            }
        }
    }
    while m.leaf(depth, |vm, depth| { let f = frame(vm, depth); f.scans[KEYTRAN].end < f.scans[FKEY].start }) {
        if let Some(diff) = remap_step(m, gc.by_ref(), depth, KEYTRAN, true)? {
            m.leaf((depth, diff), |vm, (depth, diff)| {
                let f = frame(vm, depth);
                f.scans[INDEC].shift(diff);
                f.scans[FKEY].shift(diff);
            });
            return Ok(true);
        }
    }
    Ok(false)
}

/// `keyremap_step`: scan WHICH extended by the event at its end. With DOIT,
/// when the events from its start through that one are bound to an array
/// (or to a function returning one), they are replaced by it and the scan
/// starts again after the replacement: the change of the keys' length.
/// Otherwise None: a prefix of a longer translation keeps the start, and
/// anything else moves it on by one event.
fn remap_step<'b>(m: &mut M<'_, 'b>, mut gc: Gc<'_, 'b>, depth: usize, which: usize, doit: bool) -> Done<Option<isize>> {
    keyseq::walk(m, gc.by_ref(), (depth, which), |vm, (depth, which)| {
        let scan = frame(vm, depth).scans[which];
        if !keymapp(vm, scan.parent) {
            // No map to scan: the walk of nothing.
            return Ok((Value::NIL, [Value::NIL; 2], 0, true));
        }
        Ok((keyseq::followable(vm, scan.map), [keyseq::lookup_event_at(vm, depth, scan.end), Value::NIL], 1, true))
    })?;
    let function = step(m, gc.by_ref(), (depth, which, doit), |vm, (depth, which, doit)| {
        let mut next = vm.reads.result;
        let f = frame(vm, depth);
        f.scans[which].end += 1;
        // `access_keymap_keyremap`: a symbol whose function is an array or keymap.
        let cx = Leaf::new(vm);
        if next != Value::NIL && cx.kind(next) == Kind::Symbol {
            let function = cx.symbol_function(next).unwrap_or(Value::NIL);
            if matches!(cx.kind(function), Kind::Vector | Kind::String | Kind::Bytes) || keymapp(vm, function) { next = function; }
        }
        vm.reads.result = next;
        let mut cx = Leaf::new(vm);
        Ok(doit && crate::prim::prims::vars::function_p(&mut cx, next).unwrap_or(false))
    })?;
    if function {
        // Called with `current-key-remap-sequence` the events it translates.
        let names: &'static [&'static str] = &["current-key-remap-sequence"];
        let bind = |vm: &mut Vm, (depth, which): (usize, usize)| {
            let f = frame(vm, depth);
            let scan = f.scans[which];
            let events = f.keybuf.get(scan.start..scan.end).map(<[Value]>::to_vec).unwrap_or_default();
            Ok(vec![Leaf::new(vm).vector(&events)])
        };
        let args = |vm: &mut Vm, _: (usize, usize)| Ok(vec![vm.reads.result, Value::NIL]);
        // The function is rooted in the call's frame, its value in `result`.
        call_bound(m, gc.by_ref(), names, (depth, which), bind, args)?;
        let valid = m.leaf((), |vm, ()| {
            let next = vm.reads.result;
            next == Value::NIL || matches!(Leaf::new(vm).kind(next), Kind::Vector | Kind::String | Kind::Bytes)
        });
        if !valid {
            super::call(m, gc.by_ref(), (), |vm, ()| {
                let next = vm.reads.result;
                let mut cx = Leaf::new(vm);
                Ok(vec![cx.intern("error"), cx.string("Function %s returns invalid key sequence"), next])
            })?;
            return Err(());
        }
    }
    m.leaf((depth, which, doit), |vm, (depth, which, doit)| apply(vm, depth, which, doit))
}

/// The scan's outcome once its lookup's answer is in `Reads::result`.
fn apply(vm: &mut Vm, depth: usize, which: usize, doit: bool) -> Done<Option<isize>> {
    let next = vm.reads.result;
    let cx = Leaf::new(vm);
    let replacement = match cx.kind(next) {
        Kind::Vector if doit => Some(cx.vector_values(next).map_err(|fault| signal(vm, fault))?),
        Kind::String | Kind::Bytes if doit => Some(string_events(&cx, next)),
        _ => None,
    };
    let is_map = replacement.is_none() && keymapp(vm, next);
    let f = frame(vm, depth);
    let scan = f.scans[which];
    match replacement {
        Some(events) => {
            let before = f.keybuf.len();
            let tail = f.keybuf.split_off(scan.end.min(before));
            f.keybuf.truncate(scan.start);
            f.keybuf.extend(events.iter().copied().chain(tail));
            f.scans[which].reset(scan.start + events.len());
            Ok(Some(isize::try_from(f.keybuf.len()).unwrap_or(0) - isize::try_from(before).unwrap_or(0)))
        }
        None if is_map => {
            f.scans[which].map = next;
            Ok(None)
        }
        None => {
            f.scans[which].reset(scan.start + 1);
            Ok(None)
        }
    }
}

/// A string translation's events, as `vconcat` gives them.
fn string_events(cx: &Leaf<'_>, string: Value) -> Vec<Value> {
    let mut events = Vec::new();
    let mut i = 0;
    while let Ok(Some(c)) = cx.string_char(string, i) {
        events.push(Value::integer(super::byte_char(i64::from(c))).unwrap_or(Value::NIL));
        i += 1;
    }
    events
}

const CHAR_SHIFT: i64 = 0x200_0000;
const CHAR_BITS: i64 = 0x3f_ffff;

/// An integer event C without its shift: its shift bit cleared, or an
/// upper-case character's lower case (DOWN) with its modifiers kept. None
/// when it has neither.
pub(super) fn unshifted_char(c: i64, down: impl FnOnce(i64) -> i64) -> Option<i64> {
    if c & CHAR_SHIFT != 0 { return Some(c & !CHAR_SHIFT); }
    let base = c & CHAR_BITS;
    let lower = down(base);
    (lower != base).then_some(lower | (c & !CHAR_BITS))
}

/// A symbol event's NAME without its `S-` modifier, GNU's modifiers being
/// spelled in the order A-C-H-M-S-s-: `C-S-f` is `C-f`. None without one.
pub(super) fn unshifted_name(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut at = 0;
    while bytes.get(at + 1) == Some(&b'-') && matches!(bytes.get(at), Some(b'A' | b'C' | b'H' | b'M')) { at += 2; }
    (bytes.get(at) == Some(&b'S') && bytes.get(at + 1) == Some(&b'-')).then(|| format!("{}{}", &name[..at], &name[at + 2..]))
}
