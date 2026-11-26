//! keymap.c `lookup_key_1` with `access_keymap_1` and `get_keyelt` as one
//! resumable cursor.  A menu item's `:filter` is called where the walk meets
//! it, each time it meets it, and the walk goes on from that exact place
//! with the result: over the live maps, whose tails it reads again after
//! the call, and over the live key sequence, whose events it reads as it
//! reaches them.  Between a filter's call and its result the walk is only
//! its `values`, which its driver keeps rooted (a T2 continuation's saved
//! registers), so the collector may move what it holds.
use super::Decoded;
use crate::{Result, Value, Vm};

/// Where a walk stopped.
#[derive(Clone, Copy)]
pub(crate) enum Walked {
    /// The binding of the key sequence, or the count of its events that
    /// run past a non-prefix (`lookup-key`'s answer).
    Done(Value),
    /// A menu item's FILTER to call on OBJECT; the walk resumes with the
    /// result, or nil when the filter signals (`menu_item_eval_property`).
    Filter { filter: Value, object: Value },
}

/// How a frame's answer reaches the frame below it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Return {
    /// It is the binding of the event (`lookup_key_1`).
    Event,
    /// It is the binding of an embedded keymap, an entry of the map below.
    Embedded,
    /// It is the parent's binding, composed with the prefixes below.
    Parent,
}
const RETURNS: [Return; 3] = [Return::Event, Return::Embedded, Return::Parent];

/// Where a frame's scan stopped short of a binding: at its parent's list
/// (the cons after it), or at an embedded keymap.
#[derive(Clone, Copy)]
enum Nested {
    Parent(Value),
    Embedded(Value),
}

/// What a definition being unwrapped (`get_keyelt`) is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    /// The binding of the top frame's entry at `tail`.
    Binding,
    /// The top frame's default binding, its answer.
    Fallback,
}
const PENDINGS: [Pending; 3] = [Pending::None, Pending::Binding, Pending::Fallback];

/// One `access_keymap_1` over a keymap's list.
struct Frame {
    /// The cons whose entry is at hand.
    tail: Value,
    /// `retval`: the first binding found, which a later one only replaces
    /// when it was nil.
    found: Option<Value>,
    /// The prefix keymaps found, in order, which compose.
    prefixes: Vec<Value>,
    /// `t_binding`: the first default binding met.
    fallback: Option<Value>,
    /// `t_ok`: whether a default binding may still be taken, which the
    /// first one taken ends, and which embedded maps and the parent inherit.
    t_ok: bool,
    inherit: bool,
    returns: Return,
    /// Entries visited, against a circular list.
    steps: usize,
}
const FRAME_VALUES: usize = 10;
const HEADER_VALUES: usize = 9;
/// How deep embedded keymaps and parents nest.
const DEPTH: usize = 64;

/// The key sequence a walk reads: a Lisp string or vector, or for a native
/// caller up to two events held here, off the Lisp heap.
#[derive(Clone, Copy)]
enum Keys {
    Lisp(Value),
    Inline([Value; 2]),
}

/// A meta character being looked up as `access_keymap_1` does: first its
/// ESC in MAP, then BASE in the keymap ESC binds, or else only MAP's
/// default binding.
#[derive(Clone, Copy)]
struct Meta {
    base: Value,
    map: Value,
}

/// A `lookup-key` walk in progress (`Vm::key_walk`).
pub(crate) struct KeyWalk {
    /// The effective key sequence and its length, read once at the start.
    keys: Keys,
    length: usize,
    /// The index of the key's next event to read and, in a string, that
    /// event's byte while no Lisp has run since the last read (an executable
    /// image's string, which never changes, keeps it across callbacks).
    next: usize,
    byte: Option<usize>,
    /// The meta character the frames are looking up, its ESC first.
    meta: Option<Meta>,
    /// The event the frames look up.
    event: Value,
    defaults: bool,
    frames: Vec<Frame>,
    pending: Pending,
    /// The symbol `keymap` and the list limit, read again with the values.
    keymap: Value,
    limit: usize,
}

/// Where a step leaves the walk: `Some` when it stops.
type Step = Result<Option<Walked>>;

fn flag(value: bool) -> Value {
    if value { Value::TRUE } else { Value::NIL }
}
fn count(n: usize) -> Value {
    Value::integer(i64::try_from(n).unwrap_or(i64::MAX)).unwrap_or(Value::NIL)
}
fn counted(value: Value) -> usize {
    value.integer_value().and_then(|n| usize::try_from(n).ok()).unwrap_or(0)
}

impl Vm {
    /// The walk of KEY in MAP with default bindings when DEFAULTS, run to its
    /// answer or to the first filter it must call.
    pub(crate) fn key_walk(&mut self, map: Value, key: Value, defaults: bool) -> Result<(KeyWalk, Walked)> {
        let length = self.key_length(key)?;
        self.walk_keys(map, Keys::Lisp(key), length, defaults)
    }
    fn walk_keys(&mut self, map: Value, keys: Keys, length: usize, defaults: bool) -> Result<(KeyWalk, Walked)> {
        let mut walk = KeyWalk::new(self, keys, length, defaults);
        // lookup_key_1: a cons is searched as it is, so a list of keymaps
        // acts as their composition; nil is an empty keymap.
        let is_list = map == Value::NIL || map.tag_is(3);
        if length == 0 && is_list { return Ok((walk, Walked::Done(map))); }
        let map = match self.resolve_keymap(map) {
            Some(map) => map,
            None if is_list => self.keymap_cons(walk.keymap, map),
            None => return Err("keymap required".into()),
        };
        if length == 0 { return Ok((walk, Walked::Done(map))); }
        walk.begin_event(self, map)?;
        let walked = walk.run(self)?;
        Ok((walk, walked))
    }
    /// `key_walk` of the EVENTS (one or two) held off the heap, as the key
    /// reader follows one event or looks up `[remap COMMAND]`.
    pub(crate) fn key_walk_events(&mut self, map: Value, events: &[Value], defaults: bool) -> Result<(KeyWalk, Walked)> {
        let held = [events.first().copied().unwrap_or(Value::NIL), events.get(1).copied().unwrap_or(Value::NIL)];
        self.walk_keys(map, Keys::Inline(held), events.len().min(2), defaults)
    }
    /// WALK, stopped at a filter, resumed with the filter's RESULT.
    pub(crate) fn key_walk_resume(&mut self, walk: &mut KeyWalk, result: Value) -> Result<Walked> {
        let pending = std::mem::replace(&mut walk.pending, Pending::None);
        if pending == Pending::None { return Err("keymap walk resumed with no filter called".into()); }
        match walk.unwrap(self, result, pending)? {
            Some(walked) => Ok(walked),
            None => walk.run(self),
        }
    }
    /// `access_keymap` for `define-key`, which calls no Lisp: EVENT's
    /// binding in MAP, nil when unbound, with MAP's parents when INHERIT and
    /// default bindings when DEFAULTS; an error at a menu item's filter.
    pub(super) fn keymap_access(&mut self, map: Value, event: Value, inherit: bool, defaults: bool) -> Result<Value> {
        let mut walk = KeyWalk::new(self, Keys::Inline([Value::NIL; 2]), 0, defaults);
        walk.event = event;
        walk.push(self, map, inherit, defaults, Return::Event)?;
        match walk.run(self)? {
            Walked::Done(binding) => Ok(binding),
            Walked::Filter { .. } => Err("keymap menu filter must be evaluated (VM2 define-key does not)".into()),
        }
    }
    /// KEY's binding in MAP for a caller that must not run Lisp: an error
    /// when a menu item's filter would have to run.
    pub(crate) fn key_lookup_native(&mut self, map: Value, key: Value, defaults: bool) -> Result<Value> {
        let length = self.key_length(key)?;
        self.walk_native(map, Keys::Lisp(key), length, defaults)
    }
    /// `key_lookup_native` of the EVENTS (one or two), held off the heap.
    pub(super) fn key_lookup_events(&mut self, map: Value, events: &[Value], defaults: bool) -> Result<Value> {
        let held = [events.first().copied().unwrap_or(Value::NIL), events.get(1).copied().unwrap_or(Value::NIL)];
        self.walk_native(map, Keys::Inline(held), events.len().min(2), defaults)
    }
    fn walk_native(&mut self, map: Value, keys: Keys, length: usize, defaults: bool) -> Result<Value> {
        match self.walk_keys(map, keys, length, defaults)?.1 {
            Walked::Done(binding) => Ok(binding),
            Walked::Filter { .. } => Err("keymap menu filter must be evaluated".into()),
        }
    }
}

impl KeyWalk {
    fn new(vm: &mut Vm, keys: Keys, length: usize, defaults: bool) -> Self {
        Self { keys, length, next: 0, byte: None, meta: None, event: Value::NIL, defaults, frames: Vec::new(), pending: Pending::None,
               keymap: vm.keymap_symbol("keymap"), limit: vm.list_limit() }
    }
    /// The walk as values for its driver to root, its prefixes as lists.
    pub(crate) fn values(&self, vm: &mut Vm) -> Vec<Value> {
        let pending = PENDINGS.iter().position(|p| *p == self.pending).unwrap_or(0);
        // A native caller's events never wait on a filter; were they to, they
        // would wait as the vector they stand for.
        let key = match self.keys {
            Keys::Lisp(key) => key,
            Keys::Inline(events) => vm.allocate_object(crate::data::Object::Vector(events[..self.length].to_vec())),
        };
        let (meta_base, meta_map) = self.meta.map_or((Value::NIL, Value::NIL), |meta| (meta.base, meta.map));
        let mut values = vec![key, count(self.length), count(self.next), meta_base, meta_map,
                              self.event, flag(self.defaults), count(pending), self.byte.map_or(Value::NIL, count)];
        for frame in &self.frames {
            let returns = RETURNS.iter().position(|r| *r == frame.returns).unwrap_or(0);
            let prefixes = vm.list_value(&frame.prefixes);
            values.extend([frame.tail, frame.found.unwrap_or(Value::NIL), flag(frame.found.is_some()),
                           frame.fallback.unwrap_or(Value::NIL), flag(frame.fallback.is_some()), prefixes,
                           flag(frame.inherit), count(returns), count(frame.steps), flag(frame.t_ok)]);
        }
        values
    }
    /// The walk `values` gave, read again after the collector ran.
    pub(crate) fn from_values(vm: &mut Vm, values: &[Value]) -> Result<Self> {
        let (header, frames) = values.split_at(HEADER_VALUES);
        // A base is a character, never nil.
        let meta = (header[3] != Value::NIL).then_some(Meta { base: header[3], map: header[4] });
        let frames = frames.chunks_exact(FRAME_VALUES).map(|f| Ok(Frame {
            tail: f[0], found: (f[2] != Value::NIL).then_some(f[1]), fallback: (f[4] != Value::NIL).then_some(f[3]),
            prefixes: vm.list_values(f[5])?, inherit: f[6] != Value::NIL, returns: RETURNS[counted(f[7]).min(2)], steps: counted(f[8]),
            t_ok: f[9] != Value::NIL,
        })).collect::<Result<_>>()?;
        // A callback may have changed a heap string: its byte is found again.
        let byte = (header[8] != Value::NIL && Vm::image_string(header[0])).then(|| counted(header[8]));
        Ok(Self { keys: Keys::Lisp(header[0]), length: counted(header[1]), next: counted(header[2]), byte, meta,
                  event: header[5], defaults: header[6] != Value::NIL, frames, pending: PENDINGS[counted(header[7]).min(2)],
                  keymap: vm.keymap_symbol("keymap"), limit: vm.list_limit() })
    }

    fn run(&mut self, vm: &mut Vm) -> Result<Walked> {
        loop {
            if let Some(walked) = self.step(vm)? { return Ok(walked); }
        }
    }
    /// Read the key's next event and look it up in MAP.
    fn begin_event(&mut self, vm: &mut Vm, map: Value) -> Result<()> {
        let decoded = match self.keys {
            Keys::Lisp(key) => vm.key_event_at(key, self.next, &mut self.byte)?,
            Keys::Inline(events) => vm.decode_key_event(events[self.next.min(1)], false)?,
        };
        self.next += 1;
        self.event = match decoded {
            Decoded::One(event) => event,
            Decoded::Meta(prefix, base) => {
                self.meta = Some(Meta { base, map });
                prefix
            }
            Decoded::Range(range) => vm.event_head(range),
        };
        self.push(vm, map, true, self.defaults, Return::Event)
    }
    /// A frame over MAP, a keymap.
    fn push(&mut self, vm: &mut Vm, map: Value, inherit: bool, t_ok: bool, returns: Return) -> Result<()> {
        if self.frames.len() > DEPTH { return Err("keymap nesting limit exceeded".into()); }
        let (_, tail) = vm.cons_pair(map)?;
        self.frames.push(Frame { tail, found: None, prefixes: Vec::new(), fallback: None, t_ok, inherit, returns, steps: 0 });
        Ok(())
    }
    fn top(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("a walk in progress has a frame")
    }
    /// The top frame's entries, scanned to the next that binds the event,
    /// nests a keymap or ends the frame.  Nothing calls Lisp in the scan, so
    /// it reads each entry's cdr with the entry.
    fn step(&mut self, vm: &mut Vm) -> Step {
        let (keymap, limit, event) = (self.keymap, self.limit, self.event);
        let frame = self.top();
        let mut steps = frame.steps;
        let nested = loop {
            steps += 1;
            if steps > limit + 1 { return Err("circular keymap".into()); }
            if frame.tail == Value::NIL { return self.finish(vm); }
            if !frame.tail.tag_is(3) {
                frame.tail = vm.resolve_keymap_as(frame.tail, keymap).ok_or("keymap parent required")?;
            }
            let (entry, next) = vm.cons_pair(frame.tail)?;
            // An entry's pair is read once: whether it is a keymap, and what it binds.
            let element = if entry == keymap {
                break Nested::Parent(next);
            } else if entry.tag_is(3) {
                let (key, value) = vm.cons_pair(entry)?;
                if key == keymap { break Nested::Embedded(entry); }
                vm.keymap_pair_binding(key, value, event, frame.found.is_none())
            } else if let Some(submap) = vm.resolve_keymap_as(entry, keymap) {
                break Nested::Embedded(submap);
            } else {
                vm.keymap_object_binding(entry, event)?
            };
            match element {
                Some(super::Element::Binding(value)) => {
                    frame.steps = steps;
                    let value = if value == Value::TRUE { Value::NIL } else { value };
                    return self.unwrap(vm, value, Pending::Binding);
                }
                Some(super::Element::Default(value)) if frame.t_ok => {
                    frame.fallback = Some(value);
                    frame.t_ok = false;
                }
                _ => {}
            }
            frame.tail = next;
        };
        frame.steps = steps;
        self.nest(vm, nested)
    }
    /// The top frame's scan reached NESTED: its parent's list, which it goes
    /// on into unless a binding already decides (a prefix found composes
    /// with the parent's), or an embedded keymap, which is scanned with what
    /// the frame left of its defaults.
    fn nest(&mut self, vm: &mut Vm, nested: Nested) -> Step {
        let frame = self.top();
        match nested {
            Nested::Parent(next) => {
                if !frame.inherit || frame.found == Some(Value::NIL) { return self.finish(vm); }
                if frame.found.is_none() {
                    frame.tail = next;
                    return Ok(None);
                }
                let (parent, t_ok) = (frame.tail, frame.t_ok);
                self.push(vm, parent, true, t_ok, Return::Parent)?;
            }
            Nested::Embedded(submap) => {
                let (inherit, t_ok) = (frame.inherit, frame.t_ok);
                self.push(vm, submap, inherit, t_ok, Return::Embedded)?;
            }
        }
        Ok(None)
    }
    /// Go on to the top frame's next entry, read from the live list.
    fn advance(&mut self, vm: &mut Vm) -> Step {
        let frame = self.top();
        frame.tail = vm.cons_pair(frame.tail)?.1;
        Ok(None)
    }
    /// `get_keyelt` of VALUE, which is for PENDING: stopped at a filter, or
    /// given where it belongs.
    fn unwrap(&mut self, vm: &mut Vm, value: Value, pending: Pending) -> Step {
        match vm.keymap_definition(value)? {
            Err((filter, object)) => {
                self.pending = pending;
                Ok(Some(Walked::Filter { filter, object }))
            }
            Ok(definition) if pending == Pending::Binding => self.bound(vm, definition),
            Ok(definition) => self.returned(vm, Some(definition)),
        }
    }
    /// The top frame found DEFINITION for the event.
    fn bound(&mut self, vm: &mut Vm, definition: Value) -> Step {
        let prefix = vm.resolve_keymap(definition).is_some();
        let frame = self.top();
        if prefix {
            if frame.prefixes.is_empty() { frame.found = Some(definition); }
            frame.prefixes.push(definition);
        } else {
            if frame.found.is_none_or(|found| found == Value::NIL) { frame.found = Some(definition); }
            // A binding that is no prefix shadows everything after it.
            if definition != Value::NIL { return self.finish_with(vm); }
        }
        self.advance(vm)
    }
    /// The top frame's list ended: its answer, or its default's.
    fn finish(&mut self, vm: &mut Vm) -> Step {
        let frame = self.top();
        if frame.found.is_some() { return self.finish_with(vm); }
        match frame.fallback {
            Some(fallback) => self.unwrap(vm, fallback, Pending::Fallback),
            None => self.returned(vm, None),
        }
    }
    /// The top frame answers what it found: its prefixes composed.
    fn finish_with(&mut self, vm: &mut Vm) -> Step {
        let frame = self.top();
        let (found, prefixes) = (frame.found, std::mem::take(&mut frame.prefixes));
        let answer = vm.keymap_prefixes(found, &prefixes);
        self.returned(vm, answer)
    }
    /// The top frame answered ANSWER: give it to the frame below.
    fn returned(&mut self, vm: &mut Vm, answer: Option<Value>) -> Step {
        let frame = self.frames.pop().expect("a frame answered");
        match frame.returns {
            Return::Event => self.event_bound(vm, answer.unwrap_or(Value::NIL)),
            // access_keymap_1: an embedded map's t is an explicit nil, as any entry's.
            Return::Embedded => match answer {
                Some(definition) => self.bound(vm, if definition == Value::TRUE { Value::NIL } else { definition }),
                None => self.advance(vm),
            },
            Return::Parent => {
                let keymap = self.keymap;
                let below = self.top();
                let (found, prefixes) = (below.found, std::mem::take(&mut below.prefixes));
                let Some(parent) = answer.and_then(|value| vm.resolve_keymap_as(value, keymap)) else {
                    let answer = vm.keymap_prefixes(found, &prefixes);
                    return self.returned(vm, answer);
                };
                // The prefixes found, then the parent's as their tail.
                let prefixes = if prefixes.is_empty() { found.into_iter().collect() } else { prefixes };
                let composed = prefixes.iter().rev().fold(parent, |tail, &prefix| vm.keymap_cons(prefix, tail));
                let composed = vm.keymap_cons(keymap, composed);
                self.returned(vm, Some(composed))
            }
        }
    }
    /// The frames' answer is BINDING: for a meta character's ESC, where its
    /// base is looked up next; else the event's binding, the walk's answer at
    /// the key's end, or the map for the next event.
    fn event_bound(&mut self, vm: &mut Vm, binding: Value) -> Step {
        let binding = match self.meta.take() {
            Some(meta) => match self.meta_map(vm, meta, binding)? {
                Some(binding) => binding,
                None => return Ok(None),
            },
            None => binding,
        };
        if self.next == self.length { return Ok(Some(Walked::Done(binding))); }
        let Some(prefix) = vm.resolve_keymap(binding) else {
            // Past a non-prefix: the count of the key's events read.
            return Ok(Some(Walked::Done(count(self.next))));
        };
        self.begin_event(vm, prefix)?;
        Ok(None)
    }
    /// access_keymap_1's meta character, its ESC bound to ESC_BINDING: the
    /// base is looked up in the keymap ESC binds; else, with defaults, the
    /// meta character's map is searched for its `t' binding; else it is
    /// unbound.  None while frames look further.
    fn meta_map(&mut self, vm: &mut Vm, meta: Meta, esc_binding: Value) -> Result<Option<Value>> {
        let (event, map) = match vm.resolve_keymap(esc_binding) {
            Some(esc_map) => (meta.base, esc_map),
            None if self.defaults => (Value::TRUE, meta.map),
            None => return Ok(Some(Value::NIL)),
        };
        self.event = event;
        self.push(vm, map, true, self.defaults, Return::Event)?;
        Ok(None)
    }
}
