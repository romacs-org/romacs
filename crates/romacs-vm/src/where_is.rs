//! The reverse keymap lookup: GNU's `Fwhere_is_internal`, `where_is_internal`,
//! `shadow_lookup`, `preferred_sequence_p`, `Faccessible_keymaps` and
//! `get_keyelt` in keymap.c, with its `where_is_cache`.
//!
//! This file is the walk over the keymaps and the cache of what it finds;
//! `query.rs` decides which of the sequences found run a definition.  The
//! walk calls no Lisp, as GNU's cannot: an autoloaded keymap is not loaded
//! and a menu item's `:filter` is not applied.  Lisp runs only where a found
//! key sequence is looked up again, through `lookup-key`, which applies
//! filters: the query then stops with the filter to run (`Stop::Filter`)
//! and goes on from where it was with its result.
mod query;
pub(crate) use query::Query;

use crate::map_keymap::{Advance, Cursor};
use crate::prim::{Fault, Kind, Leaf};
use crate::{HashMap, HashSet, Value};
use std::collections::VecDeque;

/// The internal symbol whose value cell holds the cache, so the collector
/// traces and forwards it like any symbol's value: [MAPS GENERATION TABLE],
/// keymap.c's `where_is_cache_keymaps` and `where_is_cache`.  TABLE maps a
/// binding, menus aside and indirection followed, to its key sequences
/// while the same MAPS are searched and no keymap changed since GENERATION.
const CACHE: &str = "#:vm2-where-is-cache";
/// The internal symbol whose value counts the walks of keymaps for their
/// bindings (`Vm::where_is_walks`), which the cache exists to save.
pub(crate) const WALKS: &str = "#:vm2-where-is-walks";
const META: i64 = 1 << 27;
/// `CHAR_MODIFIER_MASK` without `CHAR_META`.
const NON_META_MODIFIERS: i64 = 0x7c0_0000;

/// Why a walk stopped short of its value.
pub(crate) enum Stop {
    Fault(Fault),
    /// The menu ITEM's FILTER must be applied to OBJECT first.
    Filter { item: Value, filter: Value, object: Value },
}
impl From<Fault> for Stop {
    fn from(fault: Fault) -> Self {
        Stop::Fault(fault)
    }
}
type Faulty<T> = std::result::Result<T, Fault>;
type Walk<T> = std::result::Result<T, Stop>;

fn symbolp(cx: &Leaf<'_>, object: Value) -> bool {
    object == Value::NIL || object == Value::TRUE || cx.kind(object) == Kind::Symbol
}
fn stringp(cx: &Leaf<'_>, object: Value) -> bool {
    matches!(cx.kind(object), Kind::String | Kind::Bytes)
}
fn integer(n: i64) -> Faulty<Value> {
    Value::integer(n).map_err(Fault::Error)
}
fn equal(cx: &Leaf<'_>, a: Value, b: Value) -> Faulty<bool> {
    if a == b { return Ok(true); }
    cx.equal(a, b).map_err(|fault| match fault {
        crate::hash::HashFault::Wrong(predicate, value) => Fault::Wrong(predicate, value),
        crate::hash::HashFault::Error("circular-list", Some(value)) => Fault::Circular(value),
        crate::hash::HashFault::Error(message, _) => Fault::Error(message.into()),
        _ => Fault::Error("equal failed".into()),
    })
}
/// LIST's elements, to its first tail that is no cons.
fn elements(cx: &Leaf<'_>, mut list: Value) -> Faulty<Vec<Value>> {
    let mut items = Vec::new();
    while let Some((item, rest)) = cx.pair(list) {
        if items.len() > cx.list_limit() { return Err(Fault::Circular(list)); }
        items.push(item);
        list = rest;
    }
    Ok(items)
}
fn variable(cx: &mut Leaf<'_>, name: &str) -> Faulty<Value> {
    let symbol = cx.intern(name);
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}

/// Brent's cycle detection over the wrappers `get_keyelt` takes off: GNU
/// follows a circular one forever, here it signals.
pub(crate) struct Chain { mark: Value, taken: i64, limit: i64 }
impl Chain {
    pub(crate) fn new(first: Value) -> Self {
        Self { mark: first, taken: 0, limit: 1 }
    }
    pub(crate) fn values(&self) -> Faulty<[Value; 3]> {
        Ok([self.mark, integer(self.taken)?, integer(self.limit)?])
    }
    pub(crate) fn from_values(values: &[Value]) -> Self {
        let count = |v: Value| v.integer_value().unwrap_or(1);
        Self { mark: values[0], taken: count(values[1]), limit: count(values[2]) }
    }
    /// Move on to NEXT, which the object before it wrapped.
    pub(crate) fn advance(&mut self, next: Value) -> Faulty<()> {
        if next == self.mark && next.tag() == 3 { return Err(Fault::Error("Circular menu item".into())); }
        self.taken += 1;
        if self.taken == self.limit {
            (self.mark, self.taken, self.limit) = (next, 0, self.limit.saturating_mul(2));
        }
        Ok(())
    }
}
/// The symbols `get_keyelt` compares against, looked up by name once a walk.
#[derive(Clone, Copy)]
pub(crate) struct Wrappers { menu_item: Option<Value>, filter: Option<Value> }
impl Wrappers {
    pub(crate) fn new(cx: &Leaf<'_>) -> Self {
        Self { menu_item: cx.obarray_lookup("menu-item"), filter: cx.obarray_lookup(":filter") }
    }
}
/// Where `get_keyelt` got to.
pub(crate) enum Keyelt {
    Is(Value),
    /// A menu item's FILTER to apply to OBJECT, whose value the walk
    /// continues from with CHAIN.
    Filter { filter: Value, object: Value, chain: Chain },
}
/// keymap.c `get_keyelt` from OBJECT: the definition under `(STRING . DEFN)`
/// and `(menu-item NAME DEFN . PROPERTIES)` wrappers, stopping at a
/// `:filter` when AUTOLOAD.
pub(crate) fn keyelt_from(cx: &Leaf<'_>, wrappers: Wrappers, mut object: Value, autoload: bool, mut chain: Chain) -> Faulty<Keyelt> {
    while let Some((head, tail)) = cx.pair(object) {
        if Some(head) == wrappers.menu_item {
            let Some((_, rest)) = cx.pair(tail) else { break };
            object = cx.pair(rest).map_or(rest, |(definition, _)| definition);
            if autoload {
                let mut properties = rest;
                for _ in 0..=cx.list_limit() {
                    let Some(((key, next), (filter, _))) = cx.pair(properties).and_then(|p| Some((p, cx.pair(p.1)?))) else { break };
                    if Some(key) == wrappers.filter { return Ok(Keyelt::Filter { filter, object, chain }); }
                    properties = next;
                }
            }
        } else if stringp(cx, head) {
            object = tail;
        } else {
            break;
        }
        chain.advance(object)?;
    }
    Ok(Keyelt::Is(object))
}
/// `get_keyelt` without AUTOLOAD, which calls no Lisp.
fn keyelt(cx: &Leaf<'_>, wrappers: Wrappers, object: Value) -> Faulty<Value> {
    if object.tag() != 3 { return Ok(object); }
    match keyelt_from(cx, wrappers, object, false, Chain::new(object))? {
        Keyelt::Is(definition) => Ok(definition),
        Keyelt::Filter { .. } => unreachable!("no filter without AUTOLOAD"),
    }
}

/// `get_keymap` (OBJECT, 0, 0): the keymap OBJECT is or names, OBJECT itself
/// when it names an autoloaded keymap, which is not loaded.
pub(crate) fn keymap_noload(cx: &Leaf<'_>, object: Value) -> Option<Value> {
    cx.map_resolve(object, false).ok().flatten().map(|(map, _)| map)
}
/// `get_keymap` (OBJECT, 1, 0): OBJECT's keymap, which must be loaded.
fn keymap_loaded(cx: &Leaf<'_>, object: Value) -> Faulty<Value> {
    keymap_noload(cx, object).filter(|map| map.tag() == 3).ok_or(Fault::Wrong("keymapp", object))
}

/// `map_keymap` without AUTOLOAD over a keymap and its parents: each event
/// with its binding, a binding of t as nil (`map_keymap_item`).
struct Bindings(Cursor);
impl Bindings {
    fn of(map: Value) -> Self {
        Self(Cursor::new(map, false))
    }
    fn next(&mut self, cx: &mut Leaf<'_>) -> Faulty<Option<(Value, Value)>> {
        loop {
            match self.0.next(cx)? {
                Advance::Binding(event, binding) => return Ok(Some((event, if binding == Value::TRUE { Value::NIL } else { binding }))),
                Advance::Load(_, symbol) => if self.0.decline_load() { return Err(Fault::Wrong("keymapp", symbol)); },
                Advance::Done(_) => return Ok(None),
            }
        }
    }
}

/// A keymap, or the symbol of an autoloaded one, and the events that reach it.
pub(crate) struct Reached { pub(crate) keys: Vec<Value>, pub(crate) map: Value }

/// KEYS and EVENT as one sequence: a character after the meta prefix (FOLD)
/// is the meta character.
fn append(keys: &[Value], event: Value, fold: bool) -> Faulty<Vec<Value>> {
    let mut keys = keys.to_vec();
    match event.integer_value() {
        Some(c) if fold => *keys.last_mut().expect("a meta prefix") = integer(c | META)?,
        _ => keys.push(event),
    }
    Ok(keys)
}

/// The bindings a walk collects besides the keymaps it reaches: for each
/// SELECT gives a value, that value and the binding's key sequence, in the
/// order of the walk.  NOMENUS leaves out the keymaps under a mouse or menu
/// prefix.
struct Rows<'a> {
    nomenus: bool,
    select: &'a mut dyn FnMut(&Leaf<'_>, Value) -> Faulty<Option<Value>>,
    found: Vec<(Value, Value)>,
}

/// `Faccessible_keymaps` from START, the keymap of a prefix of `PREFIX_LENGTH`
/// events: breadth first, each keymap bound under a sequence no prefix of
/// which reached it already.  A meta character's sequence is as long as
/// its ESC prefix's, so it goes right after it, and the keymaps are walked
/// in the order of the value: ROWS, `where_is_internal`'s pass over each of
/// them, is collected by the same walk.
fn accessible_rows(cx: &mut Leaf<'_>, start: Option<Reached>, prefix_length: usize, mut rows: Option<&mut Rows<'_>>) -> Faulty<Vec<Reached>> {
    let (meta_prefix, wrappers) = (variable(cx, "meta-prefix-char")?, Wrappers::new(cx));
    let bits = |keys: &[Value]| -> Vec<u64> { keys.iter().map(|key| key.0).collect() };
    // The sequences that reached each keymap, so a cycle is found without
    // a scan of all that were reached.
    let mut reached_by: HashMap<u64, HashSet<Vec<u64>>> = start.iter().map(|first| (first.map.0, std::iter::once(bits(&first.keys)).collect())).collect();
    // Those still to walk, in order, and those walked.
    let mut queue: VecDeque<Reached> = start.into_iter().collect();
    let mut reached = Vec::new();
    while let Some(Reached { keys, map }) = queue.pop_front() {
        let here = bits(&keys);
        // Not the last event of the prefix itself.
        let metized = keys.len() > prefix_length && keys.last() == Some(&meta_prefix);
        let wanted = match &rows {
            Some(rows) => !(rows.nomenus && mouse_prefix(cx, &keys)?),
            None => false,
        };
        // An autoloaded keymap cannot be walked without loading it.
        let mut bindings = (map.tag() == 3).then(|| Bindings::of(map));
        while let Some((event, binding)) = bindings.as_mut().map_or(Ok(None), |b| b.next(cx))? {
            let fold = metized && event.integer_value().is_some();
            if let Some(rows) = rows.as_deref_mut().filter(|_| wanted)
                && let Some(selected) = (rows.select)(cx, binding)?
            {
                let sequence = cx.vector_from(append(&keys, event, fold)?);
                rows.found.push((selected, sequence));
            }
            let Some(child) = keymap_noload(cx, keyelt(cx, wrappers, binding)?) else { continue };
            let sequences = reached_by.entry(child.0).or_default();
            if (0..=here.len()).any(|length| sequences.contains(&here[..length])) { continue; }
            let entry = Reached { keys: append(&keys, event, fold)?, map: child };
            sequences.insert(bits(&entry.keys));
            if fold { queue.push_front(entry) } else { queue.push_back(entry) }
        }
        reached.push(Reached { keys, map });
    }
    Ok(reached)
}
pub(crate) fn accessible(cx: &mut Leaf<'_>, start: Option<Reached>, prefix_length: usize) -> Faulty<Vec<Reached>> {
    accessible_rows(cx, start, prefix_length, None)
}

/// PREFIX's events as `accessible-keymaps` lists them: a string's 8-bit
/// characters are meta characters.
pub(crate) fn prefix_events(cx: &Leaf<'_>, prefix: Value) -> Faulty<Vec<Value>> {
    if !stringp(cx, prefix) { return cx.vector_values(prefix); }
    let (mut events, mut at) = (Vec::new(), 0);
    while let Some((c, width)) = cx.string_char_at(prefix, at) {
        let c = i64::from(c);
        events.push(integer(if c < 256 && c & 0x80 != 0 { (c ^ 0x80) | META } else { c })?);
        at += width;
    }
    Ok(events)
}

/// Whether EVENTS start with a mouse or menu prefix: keymap.c's `Vmouse_events`
/// under any modifiers.
fn mouse_prefix(cx: &Leaf<'_>, events: &[Value]) -> Faulty<bool> {
    const MODIFIERS: [&str; 12] = ["A", "C", "H", "M", "S", "s", "up", "down", "drag", "click", "double", "triple"];
    const EVENTS: [&str; 11] = ["menu-bar", "tab-bar", "tool-bar", "tab-line", "header-line", "mode-line", "mouse-1", "mouse-2", "mouse-3", "mouse-4", "mouse-5"];
    let Some(&first) = events.first().filter(|&&event| symbolp(cx, event)) else { return Ok(false) };
    let name = cx.symbol_name(first)?;
    let mut base = name.as_str();
    while let Some((modifier, rest)) = base.split_once('-') {
        if !MODIFIERS.contains(&modifier) { break; }
        base = rest;
    }
    Ok(EVENTS.contains(&base))
}

/// `where_is_internal`'s walk: for each binding in the keymaps accessible
/// from MAPS, in order, the key sequence of those SELECT gives a value,
/// with that value.  NOMENUS skips the keymaps under a mouse or menu prefix.
fn bound(cx: &mut Leaf<'_>, maps: &[Value], nomenus: bool, mut select: impl FnMut(&Leaf<'_>, Value) -> Faulty<Option<Value>>) -> Faulty<Vec<(Value, Value)>> {
    let walks = cx.internal_value(WALKS).and_then(Value::integer_value).unwrap_or(0);
    cx.set_internal_value(WALKS, integer(walks + 1)?);
    let mut rows = Rows { nomenus, select: &mut select, found: Vec::new() };
    for &map in maps {
        let top = Reached { keys: Vec::new(), map: keymap_loaded(cx, map)? };
        accessible_rows(cx, Some(top), 0, Some(&mut rows))?;
    }
    Ok(rows.found)
}

/// A binding as the cache's `eql` table tells bindings apart: numbers by
/// value, anything else by identity.
#[derive(PartialEq, Eq, Hash)]
enum Eql { Is(u64), Float(u64), Integer(num_bigint::BigInt) }
fn eql_key(cx: &Leaf<'_>, value: Value) -> Eql {
    // The tag first: only an object can be a float or a bignum.
    if value.tag() != 0 { return Eql::Is(value.0); }
    match cx.kind(value) {
        Kind::Float => cx.float_value(value).map_or(Eql::Is(value.0), |float| Eql::Float(float.to_bits())),
        Kind::Bignum => cx.bigint_value(value).map_or(Eql::Is(value.0), Eql::Integer),
        _ => Eql::Is(value.0),
    }
}

/// The cache's table for MAPS, filled by one walk when MAPS or any keymap
/// changed since the last.
fn cached(cx: &mut Leaf<'_>, maps: &[Value]) -> Faulty<Value> {
    let generation = integer(i64::try_from(cx.keymap_generation() & 0x0fff_ffff_ffff_ffff).unwrap_or(0))?;
    if let Some(Ok(cache)) = cx.internal_value(CACHE).map(|cache| cx.vector_values(cache))
        && let [of, at, table] = cache[..]
        && at == generation
        && cx.vector_values(of)? == maps
    {
        return Ok(table);
    }
    // Each binding's sequences, the bindings in the order first met.
    let mut slots: HashMap<Eql, usize> = HashMap::default();
    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
    let wrappers = Wrappers::new(cx);
    for (binding, sequence) in bound(cx, maps, true, |cx, binding| keyelt(cx, wrappers, binding).map(Some))? {
        let slot = *slots.entry(eql_key(cx, binding)).or_insert_with(|| { groups.push((binding, Vec::new())); groups.len() - 1 });
        groups[slot].1.push(sequence);
    }
    let entries: Vec<(Value, Value)> = groups.into_iter().map(|(binding, sequences)| (binding, cx.vector_from(sequences))).collect();
    let table = cx.eql_table(&entries);
    let of = cx.vector(maps);
    let cache = cx.vector(&[of, generation, table]);
    cx.set_internal_value(CACHE, cache);
    Ok(table)
}

/// `where_is_internal`: the key sequences bound to DEFINITION in MAPS, before
/// shadowing, as a Lisp vector.  The cache serves the lookups that ignore
/// menus and follow indirection; any other walks the keymaps and, as in
/// GNU, drops the cache.
fn candidates(cx: &mut Leaf<'_>, maps: &[Value], definition: Value, noindirect: bool, nomenus: bool) -> Faulty<Value> {
    if nomenus && !noindirect {
        let table = cached(cx, maps)?;
        return Ok(match cx.completion_hash_find(table, definition)? {
            Some((_, sequences)) => sequences,
            None => cx.vector(&[]),
        });
    }
    cx.set_internal_value(CACHE, Value::NIL);
    let (structural, wrappers) = (definition.tag() == 3, Wrappers::new(cx));
    let found = bound(cx, maps, nomenus, |cx, binding| {
        let binding = if noindirect { binding } else { keyelt(cx, wrappers, binding)? };
        Ok((binding == definition || structural && equal(cx, binding, definition)?).then_some(Value::NIL))
    })?;
    Ok(cx.vector_from(found.into_iter().map(|(_, sequence)| sequence).collect()))
}
