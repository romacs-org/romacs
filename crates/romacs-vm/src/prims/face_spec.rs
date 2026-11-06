//! faces.el's recalculation of a face from its specs, in Rust. The same
//! writes in the same order as `face-spec-recalc', on the live lface
//! vector, so intermediate states, frame parameters, re-entry and errors
//! land where GNU's do; what goes is the Lisp around each write
//! (DIVERGENCES.md VM2-FACE-ENGINE).
use super::*;
use super::face::{carries_effects, display_supports, graphic_record, make_lisp_face, note_height, resolve, shows_changes, signal, slot, slot_effects, validate};
use crate::prim::{FaceKeys, FaceProps, Kind};
use romacs_face::registry::LFACE_VECTOR_SIZE;

/// `face--attributes-unspecified': every attribute of
/// `face-attribute-name-alist'. `:font', `:fontset' and
/// `:distant-foreground' are not among them, so a reset leaves them.
const RESET_SLOTS: [usize; 16] = [slot::FAMILY, slot::FOUNDRY, slot::WIDTH, slot::HEIGHT, slot::WEIGHT, slot::SLANT, slot::UNDERLINE,
    slot::OVERLINE, slot::EXTEND, slot::STRIKE, slot::BOX, slot::INVERSE, slot::FOREGROUND, slot::BACKGROUND, slot::STIPPLE, slot::INHERIT];

fn slot_value(index: usize) -> Value { Value::integer(index as i64).unwrap() }

/// `car' and `cdr' at once: nil of nil, GNU's error on anything but a list.
fn uncons(cx: &Leaf<'_>, value: Value) -> std::result::Result<(Value, Value), Fault> {
    if value == Value::NIL { return Ok((Value::NIL, Value::NIL)); }
    cx.pair(value).ok_or(Fault::Wrong("listp", value))
}

fn memq(cx: &Leaf<'_>, item: Value, mut list: Value) -> bool {
    while let Some((car, rest)) = cx.pair(list) {
        if car == item { return true; }
        list = rest;
    }
    false
}

fn featurep(cx: &mut Leaf<'_>, name: &str) -> std::result::Result<bool, Fault> {
    let (feature, features) = (cx.intern(name), cx.intern("features"));
    let list = cx.variable_value(features)?.unwrap_or(Value::NIL);
    Ok(memq(cx, feature, list))
}

/// `display-color-cells': the `:color-cells' of the display record.
fn color_cells(cx: &mut Leaf<'_>) -> std::result::Result<Value, Fault> {
    let (record, key) = (cx.intern("romacs-display"), cx.intern(":color-cells"));
    let mut tail = cx.variable_value(record)?.unwrap_or(Value::NIL);
    while let Some((name, rest)) = cx.pair(tail) {
        let Some((value, next)) = cx.pair(rest) else { break };
        if name == key { return Ok(value); }
        tail = next;
    }
    Ok(Value::NIL)
}

fn number(cx: &Leaf<'_>, value: Value) -> std::result::Result<f64, Fault> {
    if let Some(n) = value.integer_value() { return Ok(n as f64); }
    if cx.kind(value) == Kind::Float { return cx.float_value(value); }
    Err(Fault::Wrong("number-or-marker-p", value))
}

fn window_system_matches(cx: &mut Leaf<'_>, keys: &FaceKeys, options: Value, system: Value) -> std::result::Result<bool, Fault> {
    if memq(cx, system, options) { return Ok(true); }
    let listed = ["x", "w32", "ns", "pgtk"].iter().any(|name| cx.intern(name) == system);
    if memq(cx, keys.graphic, options) && listed { return Ok(true); }
    if system == Value::NIL { let tty = cx.intern("tty"); return Ok(memq(cx, tty, options)); }
    let asks = |cx: &mut Leaf<'_>, name: &str| { let option = cx.intern(name); memq(cx, option, options) };
    Ok((asks(cx, "motif") && featurep(cx, "motif")?)
        || (asks(cx, "gtk") && featurep(cx, "gtk")?)
        || (asks(cx, "lucid") && featurep(cx, "x-toolkit")? && !featurep(cx, "motif")? && !featurep(cx, "gtk")?)
        || (asks(cx, "x-toolkit") && featurep(cx, "x-toolkit")?))
}

/// What `face-spec-set-match-display' reads of a frame, once for a pass:
/// only a write to an effect-carrying face can change it during one.
#[derive(Clone, Copy)]
pub(super) struct Class {
    bare: bool,
    system: Value,
    cells: Value,
    display_type: Value,
    background: Value,
}

fn frame_class(cx: &mut Leaf<'_>, keys: &FaceKeys, frame: Value) -> std::result::Result<Class, Fault> {
    // Without romacs-frame.el's display record there is one display to
    // answer for: the graphical truecolour one GNU's specs call pgtk, dark.
    if cx.display_record_missing() {
        let (system, display_type, background) = (cx.intern("pgtk"), cx.intern("color"), cx.intern("dark"));
        return Ok(Class { bare: true, system, cells: Value::integer(1 << 24).unwrap(), display_type, background });
    }
    Ok(Class {
        bare: false,
        system: cx.windows().window_system(frame)?,
        cells: color_cells(cx)?,
        display_type: cx.stored_frame_parameter(frame, keys.display_type),
        background: cx.stored_frame_parameter(frame, keys.background_mode),
    })
}

/// `face-spec-set-match-display'.
fn display_matches(cx: &mut Leaf<'_>, keys: &FaceKeys, class: &Class, display: Value, frame: Value) -> std::result::Result<bool, Fault> {
    let mut conjuncts = if display == Value::TRUE { Value::NIL } else { display };
    let mut matched = true;
    while conjuncts != Value::NIL && matched {
        let (conjunct, rest) = uncons(cx, conjuncts)?;
        conjuncts = rest;
        let (req, options) = uncons(cx, conjunct)?;
        matched = if req == keys.type_condition {
            let gtk = cx.intern("gtk");
            (class.bare && memq(cx, gtk, options)) || window_system_matches(cx, keys, options, class.system)?
        } else if req == keys.min_colors {
            let (cells, least) = (number(cx, class.cells)?, uncons(cx, options)?.0);
            cells >= number(cx, least)?
        } else if req == keys.class {
            memq(cx, class.display_type, options)
        } else if req == keys.background {
            memq(cx, class.background, options)
        } else if req == keys.supports {
            class.bare || display_supports(cx, options, frame)? != Value::NIL
        } else {
            let template = cx.string("Unknown req `%S' with options `%S'");
            let text = cx.format_message(template, &[req, options])?;
            let (error, data) = (cx.intern("error"), cx.cons(text, Value::NIL));
            return Err(Fault::Signal(error, data));
        };
    }
    Ok(matched)
}

/// `(append DEFAULTS RESULT)'.
fn append(cx: &mut Leaf<'_>, defaults: Value, result: Value) -> Out {
    let mut items = vec![];
    let mut tail = defaults;
    while tail != Value::NIL {
        let (item, rest) = uncons(cx, tail)?;
        items.push(item);
        tail = rest;
    }
    Ok(items.into_iter().rev().fold(result, |list, item| cx.cons(item, list)))
}

/// `face-spec-choose': the attributes SPEC gives FRAME, None for no match.
pub(super) fn choose(cx: &mut Leaf<'_>, keys: &FaceKeys, class: &Class, spec: Value, frame: Value) -> std::result::Result<Option<Value>, Fault> {
    let (mut tail, mut defaults, mut chosen) = (spec, Value::NIL, None);
    while tail != Value::NIL {
        let (entry, rest) = uncons(cx, tail)?;
        tail = rest;
        let (display, attrs) = uncons(cx, entry)?;
        // An old-style entry: the attribute list is the first element.
        let (first, more) = uncons(cx, attrs)?;
        let value = if more == Value::NIL { first } else { attrs };
        if display == keys.default {
            defaults = value;
        } else if display_matches(cx, keys, class, display, frame)? {
            chosen = Some(value);
            break;
        }
    }
    if defaults != Value::NIL { return append(cx, defaults, chosen.unwrap_or(Value::NIL)).map(Some); }
    Ok(chosen)
}

/// `(face-attribute FACE ATTR FRAME)' for a slot: None when FACE has no
/// lface there, which GNU signals as `Invalid face'.
fn own_attribute(cx: &mut Leaf<'_>, keys: &FaceKeys, face: Value, index: usize, frame: Value) -> std::result::Result<Option<Value>, Fault> {
    let resolved = resolve(cx, face, true)?;
    let lface = cx.face_lookup(resolved, frame)?;
    if lface == Value::NIL { return Ok(None); }
    let value = cx.vector_get(lface, slot_value(index))?;
    Ok(Some(if value == keys.ignore_defface { keys.unspecified } else { value }))
}

/// `(face-attribute FACE :extend FRAME t)' and `face-attribute-merged-with'
/// for an attribute with no relative values but `unspecified': the first
/// specified along the inheritance, None for an `Invalid face' on the way.
fn extend_through(cx: &mut Leaf<'_>, keys: &FaceKeys, face: Value, frame: Value, depth: usize) -> std::result::Result<Option<Value>, Fault> {
    let Some(value) = own_attribute(cx, keys, face, slot::EXTEND, frame)? else { return Ok(None) };
    if value != keys.unspecified || depth > 256 { return Ok(Some(value)); }
    let Some(parents) = own_attribute(cx, keys, face, slot::INHERIT, frame)? else { return Ok(None) };
    if parents == Value::NIL || parents == keys.unspecified { return Ok(Some(value)); }
    let mut merged = value;
    let mut tail = parents;
    while merged == keys.unspecified && tail != Value::NIL {
        let (parent, rest) = match cx.pair(tail) { Some(pair) => pair, None => (tail, Value::NIL) };
        tail = rest;
        match extend_through(cx, keys, parent, frame, depth + 1)? {
            Some(found) => merged = found,
            // `face-attribute' drops the merge when a parent has no lface.
            None => return Ok(Some(value)),
        }
    }
    Ok(Some(merged))
}

fn plist_member(cx: &Leaf<'_>, plist: Value, key: Value) -> Option<Value> {
    let mut tail = plist;
    while let Some((name, rest)) = cx.pair(tail) {
        if name == key { return Some(tail); }
        tail = cx.pair(rest)?.1;
    }
    None
}

/// `FOUNDRY-FAMILY' as `set-face-attribute' splits it: the family and
/// foundry to write. A family of digits alone names no family.
fn split_family(cx: &mut Leaf<'_>, family: Value, foundry: Value) -> (Value, Value) {
    if !matches!(cx.kind(family), Kind::String | Kind::Bytes) { return (family, foundry); }
    let Ok(view) = cx.string_view(family) else { return (family, foundry) };
    let text = String::from_utf8_lossy(view.bytes()).into_owned();
    let Some((first, rest)) = text.split_once('-') else { return (family, foundry) };
    let second = rest.split('-').next().unwrap_or("");
    if second.bytes().all(|b| b.is_ascii_digit()) { return (family, foundry); }
    let foundry = if foundry == Value::NIL { cx.string(first) } else { foundry };
    (cx.string(second), foundry)
}

/// The face and frame one replay writes.
struct Replay {
    keys: FaceKeys,
    class: Class,
    face: Value,
    frame: Value,
    lface: Value,
    effects: bool,
    /// Whether the display shows this face on this frame.
    shown: bool,
    /// Whether what the display realized is gone since Lisp last ran: a
    /// callback may realize faces from the vector as it stands.
    invalidated: bool,
    /// The slots this replay wrote, by bit.
    written: u32,
    /// How many times a plist wrote `:inherit'.
    inherits: u32,
}

impl Replay {
    /// Before a slot changes: every exit after a write has invalidated.
    fn invalidate(&mut self, cx: &mut Leaf<'_>) {
        if self.shown && !self.invalidated {
            cx.clear_face_cache();
            self.invalidated = true;
        }
    }

    fn store(&mut self, cx: &mut Leaf<'_>, index: usize, value: Value) -> std::result::Result<(), Fault> {
        let at = slot_value(index);
        let old = cx.vector_get(self.lface, at)?;
        if old != value { self.invalidate(cx); }
        cx.vector_set(self.lface, at, value)?;
        self.written |= 1 << index;
        if index == slot::INHERIT { self.inherits += 1; }
        if self.shown && index == slot::HEIGHT {
            note_height(cx, self.face, value, self.frame)?;
            self.invalidated = false;
        }
        if !self.effects || value == self.keys.unspecified || value == self.keys.ignore_defface || value == self.keys.reset { return Ok(()); }
        if cx.equal(old, value).map_err(|_| Fault::Circular(value))? { return Ok(()); }
        let done = slot_effects(cx, &self.keys, self.face, index, value, self.frame, false);
        self.invalidated = false;
        done
    }

    /// `internal-set-lisp-face-attribute' on the frame.
    fn write(&mut self, cx: &mut Leaf<'_>, attr: Value, value: Value) -> std::result::Result<(), Fault> {
        let keys = self.keys;
        let checked = validate(cx, &keys, self.face, attr, value, self.frame, false);
        // A function's :height runs Lisp in validation.
        if attr == keys.keywords[slot::HEIGHT - 1] { self.invalidated = false; }
        match checked? {
            Some((index, value)) => self.store(cx, index, value),
            None => Ok(()),
        }
    }

    /// `face-spec-reset-face'.
    fn reset(&mut self, cx: &mut Leaf<'_>) -> std::result::Result<(), Fault> {
        let keys = self.keys;
        if self.face != keys.default {
            for index in RESET_SLOTS {
                self.written |= 1 << index;
                if cx.vector_slots(self.lface).is_some_and(|slots| slots[index] != keys.unspecified) {
                    self.invalidate(cx);
                    cx.vector_set(self.lface, slot_value(index), keys.unspecified)?;
                }
            }
            return Ok(());
        }
        let keyword = |index: usize| keys.keywords[index - 1];
        // Before romacs-frame.el there is no terminal to take colours from.
        if !cx.display_record_missing() && !graphic_record(cx)? {
            let selected = cx.face_frame(Value::NIL, true)?;
            let reverse = cx.stored_frame_parameter(selected, keys.reverse) != Value::NIL;
            let name = cx.string("default");
            self.write(cx, keyword(slot::FAMILY), name)?;
            self.write(cx, keyword(slot::FOUNDRY), name)?;
            for index in [slot::UNDERLINE, slot::OVERLINE, slot::STRIKE, slot::BOX, slot::INVERSE, slot::STIPPLE, slot::INHERIT, slot::EXTEND] {
                self.write(cx, keyword(index), Value::NIL)?;
            }
            self.write(cx, keyword(slot::WIDTH), keys.normal)?;
            self.write(cx, keyword(slot::HEIGHT), Value::integer(1).unwrap())?;
            self.write(cx, keyword(slot::WEIGHT), keys.normal)?;
            self.write(cx, keyword(slot::SLANT), keys.normal)?;
            let (fg, bg) = (cx.string("unspecified-fg"), cx.string("unspecified-bg"));
            self.write(cx, keyword(slot::FOREGROUND), if reverse { bg } else { fg })?;
            return self.write(cx, keyword(slot::BACKGROUND), if reverse { fg } else { bg });
        }
        for index in [slot::UNDERLINE, slot::OVERLINE, slot::STRIKE, slot::BOX, slot::INVERSE, slot::STIPPLE, slot::INHERIT, slot::EXTEND] {
            self.write(cx, keyword(index), Value::NIL)?;
        }
        Ok(())
    }

    /// `face-spec-set-2': the plist's attributes `face-x-resources' knows,
    /// through `set-face-attribute': family and foundry first.
    fn apply(&mut self, cx: &mut Leaf<'_>, plist: Value) -> std::result::Result<(), Fault> {
        let keys = self.keys;
        // Without faces.el there is no list to filter by: every key is known.
        let known = cx.variable_value(keys.face_x_resources)?;
        let mut pairs = Vec::with_capacity(8);
        let mut tail = plist;
        while tail != Value::NIL {
            let (attr, rest) = uncons(cx, tail)?;
            let (value, next) = uncons(cx, rest)?;
            tail = next;
            let Some(mut entries) = known else { pairs.push((attr, value)); continue };
            while let Some((entry, more)) = cx.pair(entries) {
                if cx.pair(entry).is_some_and(|(name, _)| name == attr) { pairs.push((attr, value)); break; }
                entries = more;
            }
        }
        let (family_key, foundry_key) = (keys.keywords[slot::FAMILY - 1], keys.keywords[slot::FOUNDRY - 1]);
        let last = |key: Value| pairs.iter().rev().find(|(attr, _)| *attr == key).map_or(Value::NIL, |&(_, value)| value);
        let (family, foundry) = (last(family_key), last(foundry_key));
        if family != Value::NIL || foundry != Value::NIL {
            let (family, foundry) = split_family(cx, family, foundry);
            let writes = |cx: &Leaf<'_>, value: Value| matches!(cx.kind(value), Kind::String | Kind::Bytes) || value == keys.unspecified;
            if writes(cx, family) { self.write(cx, family_key, family)?; }
            if writes(cx, foundry) { self.write(cx, foundry_key, foundry)?; }
        }
        for (attr, value) in pairs {
            if attr != family_key && attr != foundry_key { self.write(cx, attr, value)?; }
        }
        Ok(())
    }

    /// `face-spec-recalc' from the reset on.
    fn run(&mut self, cx: &mut Leaf<'_>, props: &FaceProps) -> std::result::Result<(), Fault> {
        let (keys, class, face, frame) = (self.keys, self.class, self.face, self.frame);
        self.reset(cx)?;
        let mut stack = vec![];
        let mut tail = props.theme_face;
        while let Some((entry, rest)) = cx.pair(tail) { stack.push(entry); tail = rest; }
        let mut themed = false;
        for entry in stack.into_iter().rev() {
            let spec = uncons(cx, uncons(cx, entry)?.1)?.0;
            if let Some(attrs) = choose(cx, &keys, &class, spec, frame)? {
                self.apply(cx, attrs)?;
                themed = true;
            }
        }
        // Chosen whatever a theme did: a bad spec signals either way.
        let defaults = choose(cx, &keys, &class, props.defface, frame)?.unwrap_or(Value::NIL);
        if !themed {
            self.apply(cx, defaults)?;
        } else if extend_through(cx, &keys, face, frame, 0)?.unwrap_or(keys.unspecified) == keys.unspecified {
            let extend = keys.keywords[slot::EXTEND - 1];
            if let Some(tail) = plist_member(cx, defaults, extend) {
                let value = uncons(cx, uncons(cx, tail)?.1)?.0;
                let tail = cx.cons(value, Value::NIL);
                let pair = cx.cons(extend, tail);
                self.apply(cx, pair)?;
            }
        }
        let attrs = choose(cx, &keys, &class, props.over, frame)?.unwrap_or(Value::NIL);
        self.apply(cx, attrs)?;
        if face == keys.scroll_bar {
            // `(face-foreground face)' and `-background' read the selected frame.
            let selected = cx.face_frame(Value::NIL, true)?;
            for (index, parameter) in [(slot::FOREGROUND, "scroll-bar-foreground"), (slot::BACKGROUND, "scroll-bar-background")] {
                let value = own_attribute(cx, &keys, face, index, selected)?.filter(|&v| v != keys.unspecified).unwrap_or(Value::NIL);
                let key = cx.intern(parameter);
                cx.windows().store_frame_parameter(frame, key, value)?;
            }
        }
        Ok(())
    }
}

/// What a memo holds: the inputs, whether the face can be memoised, the
/// result written last, and two results, each for one frame class.
const MEMO_DEFFACE: usize = 0;
const MEMO_CUSTOMIZED: usize = 1;
const MEMO_SAVED: usize = 2;
const MEMO_RESOURCES: usize = 3;
const MEMO_STABLE: usize = 4;
const MEMO_LATEST: usize = 5;
const MEMO_RESULTS: usize = 6;
const RESULT_CLASS: usize = 0;
const RESULT_MATCH: usize = 4;
const RESULT_VALIDATE: usize = 5;
const RESULT_SLOTS: usize = 6;
const RESULT_SIZE: usize = RESULT_SLOTS + LFACE_VECTOR_SIZE;
const MEMO_SIZE: usize = MEMO_RESULTS + 2 * RESULT_SIZE;

/// A replay of SPEC is a function of the spec and the frame class only: in
/// no alternative does it ask `supports', whose answers read `default', or
/// carry a function `:height', which runs Lisp.
fn memoisable(cx: &Leaf<'_>, keys: &FaceKeys, spec: Value) -> bool {
    let mut alternatives = spec;
    while let Some((entry, rest)) = cx.pair(alternatives) {
        alternatives = rest;
        let Some((display, attrs)) = cx.pair(entry) else { continue };
        let mut conjuncts = display;
        while let Some((conjunct, next)) = cx.pair(conjuncts) {
            if cx.pair(conjunct).is_some_and(|(req, _)| req == keys.supports) { return false; }
            conjuncts = next;
        }
        let plist = match cx.pair(attrs) { Some((first, more)) if more == Value::NIL => first, _ => attrs };
        let height = keys.keywords[slot::HEIGHT - 1];
        if let Some(tail) = plist_member(cx, plist, height) {
            let value = cx.pair(tail).and_then(|(_, rest)| cx.pair(rest)).map_or(Value::NIL, |(value, _)| value);
            if value.integer_value().is_none() && cx.kind(value) != Kind::Float { return false; }
        }
    }
    true
}

/// A memo's result for CLASS: its offset in the memo.
fn memo_result(memo: &[Value], class: &Class) -> Option<usize> {
    (0..2).map(|r| MEMO_RESULTS + r * RESULT_SIZE).find(|&at| {
        memo[at + RESULT_CLASS] == class.system && memo[at + RESULT_CLASS + 1] == class.cells
            && memo[at + RESULT_CLASS + 2] == class.display_type && memo[at + RESULT_CLASS + 3] == class.background
    })
}

fn memo_current(memo: &[Value], props: &FaceProps, resources: Value) -> bool {
    memo[MEMO_DEFFACE] == props.defface && memo[MEMO_CUSTOMIZED] == props.customized
        && memo[MEMO_SAVED] == props.saved && memo[MEMO_RESOURCES] == resources
}

/// Whether the vector LFACE holds what the result at AT wrote.
fn result_held(keys: &FaceKeys, memo: &[Value], at: usize, lface: &[Value]) -> bool {
    (1..LFACE_VECTOR_SIZE).all(|index| {
        let written = memo[at + RESULT_SLOTS + index];
        written == keys.unwritten || written == lface[index]
    })
}

enum Memoed { Done, Replay { record: bool } }

/// A defface-only face's recalculation from its memo, where its inputs and
/// the frame class are ones a replay has already answered: the same
/// vector, or the recorded slots stored, which is what the replay writes.
/// The live vector is the check, so nothing has to report writing to it.
fn from_memo(cx: &mut Leaf<'_>, keys: &FaceKeys, class: &Class, face: Value, frame: Value, lface: Value, props: &FaceProps, resources: Value, shown: bool)
    -> std::result::Result<Memoed, Fault> {
    let memos = cx.face_memos(frame, false)?;
    if memos == Value::NIL { return Ok(Memoed::Replay { record: true }); }
    let memo = cx.face_memo(memos, face);
    let Some(held) = cx.vector_slots(memo) else { return Ok(Memoed::Replay { record: true }) };
    if !memo_current(held, props, resources) { return Ok(Memoed::Replay { record: true }); }
    if held[MEMO_STABLE] == Value::NIL { return Ok(Memoed::Replay { record: false }); }
    let Some(at) = memo_result(held, class) else { return Ok(Memoed::Replay { record: true }) };
    let Some(vector) = cx.vector_slots(lface) else { return Ok(Memoed::Replay { record: false }) };
    if result_held(keys, held, at, vector) { return Ok(Memoed::Done); }
    // A result that writes `:inherit' replays: a local edit may have dropped
    // the edge and a later write closed a cycle GNU's write would signal.
    if held[at + RESULT_VALIDATE] != Value::NIL { return Ok(Memoed::Replay { record: true }); }
    let mut writes = [Value::NIL; LFACE_VECTOR_SIZE];
    writes.copy_from_slice(&held[at + RESULT_SLOTS..at + RESULT_SLOTS + LFACE_VECTOR_SIZE]);
    if shown { cx.clear_face_cache(); }
    for (index, &value) in writes.iter().enumerate().skip(1) {
        if value != keys.unwritten { cx.vector_set(lface, slot_value(index), value)?; }
    }
    Ok(Memoed::Done)
}

/// Keep what REPLAY wrote for its class.
fn record_memo(cx: &mut Leaf<'_>, replay: &Replay, props: &FaceProps, resources: Value) -> std::result::Result<(), Fault> {
    let keys = replay.keys;
    let memos = cx.face_memos(replay.frame, true)?;
    let mut memo = cx.face_memo(memos, replay.face);
    if !cx.vector_slots(memo).is_some_and(|held| memo_current(held, props, resources)) {
        let mut fresh = vec![keys.unwritten; MEMO_SIZE];
        (fresh[MEMO_DEFFACE], fresh[MEMO_CUSTOMIZED], fresh[MEMO_SAVED], fresh[MEMO_RESOURCES]) = (props.defface, props.customized, props.saved, resources);
        fresh[MEMO_STABLE] = if memoisable(cx, &keys, props.defface) { Value::TRUE } else { Value::NIL };
        fresh[MEMO_LATEST] = Value::integer(0).unwrap();
        memo = cx.vector_from(fresh);
        cx.face_memo_put(memos, replay.face, memo)?;
    }
    let (at, stable, latest) = {
        let held = cx.vector_slots(memo).expect("a memo is a vector");
        let latest = held[MEMO_LATEST].integer_value().unwrap_or(0) as usize;
        let at = memo_result(held, &replay.class).unwrap_or(MEMO_RESULTS + (1 - latest) * RESULT_SIZE);
        (at, held[MEMO_STABLE] != Value::NIL, (at - MEMO_RESULTS) / RESULT_SIZE)
    };
    if !stable { return Ok(()); }
    let index = |n: usize| Value::integer(n as i64).unwrap();
    if replay.inherits > 1 { cx.vector_set(memo, index(MEMO_STABLE), Value::NIL)?; return Ok(()); }
    let class = [replay.class.system, replay.class.cells, replay.class.display_type, replay.class.background];
    for (n, &value) in class.iter().enumerate() { cx.vector_set(memo, index(at + RESULT_CLASS + n), value)?; }
    cx.vector_set(memo, index(at + RESULT_MATCH), keys.unwritten)?;
    cx.vector_set(memo, index(at + RESULT_VALIDATE), if replay.inherits > 0 { Value::TRUE } else { Value::NIL })?;
    cx.vector_set(memo, index(MEMO_LATEST), index(latest))?;
    for slot_index in 1..LFACE_VECTOR_SIZE {
        let value = if replay.written & (1 << slot_index) != 0 { cx.vector_get(replay.lface, slot_value(slot_index))? } else { keys.unwritten };
        cx.vector_set(memo, index(at + RESULT_SLOTS + slot_index), value)?;
    }
    Ok(())
}

/// `face-spec-recalc' under CLASS. A frame showing another's faces has
/// nothing to recompute: they are recomputed where they live
/// (VM2-CHILD-FRAME).
fn recalc_in(cx: &mut Leaf<'_>, keys: &FaceKeys, class: &Class, face: Value, frame: Value) -> Out {
    let mut face = if cx.kind(face) == Kind::Symbol { face } else { resolve(cx, face, true)? };
    let mut props = cx.face_props(face)?;
    if props.alias != Value::NIL {
        face = resolve(cx, face, true)?;
        props = cx.face_props(face)?;
    }
    let Some(lface) = cx.owned_lface(face, frame)? else { return Ok(Value::NIL) };
    let lface = if lface == Value::NIL { make_lisp_face(cx, face, frame)? } else { lface };
    let effects = carries_effects(keys, face);
    let shown = super::face::displayed_faces(cx, frame)? && props.no_inherit == Value::NIL;
    let resources = cx.variable_value(keys.face_x_resources)?.unwrap_or(Value::NIL);
    let mut record = false;
    if !effects && props.theme_face == Value::NIL && props.over == Value::NIL {
        match from_memo(cx, keys, class, face, frame, lface, &props, resources, shown)? {
            Memoed::Done => return Ok(Value::NIL),
            Memoed::Replay { record: wanted } => record = wanted,
        }
    }
    let mut replay = Replay { keys: *keys, class: *class, face, frame, lface, effects, shown, invalidated: false, written: 0, inherits: 0 };
    replay.run(cx, &props)?;
    if record { record_memo(cx, &replay, &props, resources)?; }
    Ok(Value::NIL)
}

/// `face-spec-recalc'.
pub(super) fn recalc(cx: &mut Leaf<'_>, face: Value, frame: Value) -> Out {
    let keys = cx.face_keys();
    let frame = cx.face_frame(frame, true)?;
    let class = frame_class(cx, &keys, frame)?;
    recalc_in(cx, &keys, &class, face, frame)
}

/// `defface' without faces.el (`#:vm2-defface'): the spec recorded and
/// applied by the same replay, then kept as the default for new frames.
pub(crate) fn declare(cx: &mut Leaf<'_>, face: Value, spec: Value) -> std::result::Result<(), Fault> {
    let keys = cx.face_keys();
    let frame = cx.face_frame(Value::NIL, true)?;
    cx.face_put(face, keys.defface_spec, spec)?;
    if cx.face_lookup(face, Value::NIL)? == Value::NIL { make_lisp_face(cx, face, Value::NIL)?; }
    recalc(cx, face, frame)?;
    let (local, global) = (cx.face_lookup(face, frame)?, cx.face_lookup(face, Value::NIL)?);
    for index in 1..LFACE_VECTOR_SIZE {
        let value = cx.vector_get(local, slot_value(index))?;
        cx.vector_set(global, slot_value(index), value)?;
    }
    Ok(())
}

defsubr! { name = "romacs--face-spec-recalc", gnu = "romacs:faces", effect = Global,
fn romacs_face_spec_recalc(cx, face: Value, frame: Value) { recalc(cx, face, frame) }}

/// `custom-theme-recalc-face': one step of an alias, as custom.el, and
/// `face-spec-recalc' follows the rest.
fn recalc_themed(cx: &mut Leaf<'_>, keys: &FaceKeys, face: Value) -> std::result::Result<(), Fault> {
    let alias = cx.face_property(face, keys.face_alias)?;
    let face = if alias == Value::NIL { face } else { alias };
    let resolved = resolve(cx, face, true)?;
    if cx.face_lookup(resolved, Value::NIL)? == Value::NIL { return Ok(()); }
    let mut frames = cx.windows().frame_list()?;
    while let Some((frame, rest)) = cx.pair(frames) {
        recalc(cx, face, frame)?;
        cx.face_loop_quit()?;
        frames = rest;
    }
    Ok(())
}

defsubr! { name = "romacs--custom-theme-recalc-face", gnu = "romacs:faces", effect = Global,
fn romacs_custom_theme_recalc_face(cx, face: Value) {
    let keys = cx.face_keys();
    recalc_themed(cx, &keys, face)?;
    Ok(Value::NIL)
}}

/// `(assq-delete-all KEY ALIST)'.
fn assq_delete_all(cx: &mut Leaf<'_>, key: Value, mut alist: Value) -> Out {
    let matches = |cx: &Leaf<'_>, entry: Value| cx.pair(entry).is_some_and(|(car, _)| car == key);
    while let Some((entry, rest)) = cx.pair(alist) {
        if !matches(cx, entry) { break; }
        alist = rest;
    }
    let mut tail = alist;
    while let Some((_, next)) = cx.pair(tail) {
        let Some((entry, rest)) = cx.pair(next) else { break };
        if matches(cx, entry) { cx.setcdr(tail, rest)?; } else { tail = next; }
    }
    Ok(alist)
}

defsubr! { name = "romacs--theme-enable-faces", gnu = "romacs:faces", effect = Global,
fn romacs_theme_enable_faces(cx, theme: Value, settings: Value) {
    // `enable-theme''s loop over a run of face settings: the tail from
    // the first setting that is not a face's, for Lisp to take, else nil.
    let keys = cx.face_keys();
    let mut tail = settings;
    while let Some((setting, rest)) = cx.pair(tail) {
        let (prop, after) = uncons(cx, setting)?;
        if prop != keys.theme_face { return Ok(tail); }
        let (symbol, spec) = uncons(cx, after)?;
        let stack = cx.face_property(symbol, keys.theme_face)?;
        let kept = assq_delete_all(cx, theme, stack)?;
        let stack = cx.cons(spec, kept);
        cx.face_put(symbol, keys.theme_face, stack)?;
        recalc_themed(cx, &keys, symbol)?;
        // Most of a theme's settings name faces no package has defined,
        // which `recalc_themed' leaves before its own polling.
        cx.face_loop_quit()?;
        tail = rest;
    }
    Ok(Value::NIL)
}}

defsubr! { name = "romacs--theme-disable-faces", gnu = "romacs:faces", effect = Global,
fn romacs_theme_disable_faces(cx, theme: Value, settings: Value) {
    // `disable-theme''s loop over a run of face settings, as above.
    let keys = cx.face_keys();
    let mut tail = settings;
    while let Some((setting, rest)) = cx.pair(tail) {
        let (prop, after) = uncons(cx, setting)?;
        if prop != keys.theme_face { return Ok(tail); }
        let (symbol, spec) = uncons(cx, after)?;
        let stack = cx.face_property(symbol, keys.theme_face)?;
        let kept = assq_delete_all(cx, theme, stack)?;
        cx.face_put(symbol, keys.theme_face, kept)?;
        // A spec this theme gave `saved-face' goes with it.
        let (value, saved) = (uncons(cx, uncons(cx, spec)?.1)?.0, cx.face_property(symbol, keys.saved_face)?);
        if cx.equal(value, saved).map_err(|_| Fault::Circular(value))? {
            let next = uncons(cx, uncons(cx, uncons(cx, kept)?.0)?.1)?.0;
            cx.face_put(symbol, keys.saved_face, next)?;
        }
        cx.face_loop_quit()?;
        tail = rest;
    }
    Ok(Value::NIL)
}}

defsubr! { name = "romacs--recalc-all-faces", gnu = "romacs:faces", effect = Global,
fn romacs_recalc_all_faces(cx, frame: Value) {
    // `face-set-after-frame-default''s loop, `default' first: an invalid
    // spec must not prevent a frame's creation. A frame showing another's
    // faces has them from that frame's own turn.
    let frame = cx.face_frame(frame, true)?;
    if cx.frame_shares_faces(frame)? { return Ok(Value::NIL); }
    let keys = cx.face_keys();
    let mut class = frame_class(cx, &keys, frame)?;
    let mut faces = cx.face_ids()?;
    faces.sort_unstable_by_key(|&(id, _)| id);
    for (_, face) in faces {
        let done = recalc_in(cx, &keys, &class, face, frame).and_then(|_| super::face::merge_in_global(cx, face, frame));
        if let Err(fault) = done { cx.swallow_face_error(fault)?; }
        if carries_effects(&keys, face) { class = frame_class(cx, &keys, frame)?; }
        cx.face_loop_quit()?;
    }
    Ok(Value::NIL)
}}

/// Whether FACE on FRAME is left alone when the background mode changes.
fn locally_modified(cx: &mut Leaf<'_>, keys: &FaceKeys, class: &Class, face: Value, frame: Value, shown: Value, memos: Value) -> std::result::Result<bool, Fault> {
    let props = cx.face_props(face)?;
    if props.over != Value::NIL { return Ok(false); }
    let lface = cx.face_memo(shown, face);
    if lface == Value::NIL { return Ok(true); }
    let spec = [props.customized, props.saved, props.defface].into_iter().find(|&spec| spec != Value::NIL).unwrap_or(Value::NIL);
    let matched = |cx: &mut Leaf<'_>| -> std::result::Result<bool, Fault> {
        let attrs = choose(cx, keys, class, spec, frame)?.unwrap_or(Value::NIL);
        attrs_match(cx, keys, face, attrs, frame)
    };
    // The memo keeps GNU's answer for the vector a replay left.
    let resources = cx.variable_value(keys.face_x_resources)?.unwrap_or(Value::NIL);
    let memo = if memos == Value::NIL || props.theme_face != Value::NIL { Value::NIL } else { cx.face_memo(memos, face) };
    let known = cx.vector_slots(memo).filter(|held| memo_current(held, &props, resources) && held[MEMO_STABLE] != Value::NIL)
        .and_then(|held| memo_result(held, class).map(|at| (at, held[at + RESULT_MATCH])));
    let Some((at, answer)) = known else { return Ok(!matched(cx)?) };
    let held = cx.vector_slots(memo).expect("a memo is a vector");
    let vector = cx.vector_slots(lface).ok_or(Fault::Wrong("vectorp", lface))?;
    if !result_held(keys, held, at, vector) { return Ok(!matched(cx)?); }
    if answer != keys.unwritten { return Ok(answer == Value::NIL); }
    let result = matched(cx)?;
    cx.vector_set(memo, Value::integer((at + RESULT_MATCH) as i64).unwrap(), if result { Value::TRUE } else { Value::NIL })?;
    Ok(!result)
}

defsubr! { name = "romacs--faces-modified-on", gnu = "romacs:faces", effect = Reads,
fn romacs_faces_modified_on(cx, frame: Value) {
    // `frame-set-background-mode''s scan: faces that don't match their
    // specs were modified on FRAME, and a mode change leaves them alone. A
    // frame showing another's faces recalculates none, so has none to keep.
    let keys = cx.face_keys();
    let frame = cx.face_frame(frame, true)?;
    if cx.frame_shares_faces(frame)? { return Ok(Value::NIL); }
    let selected = cx.face_frame(Value::NIL, true)?;
    let owner = cx.face_owner(selected)?;
    let shown = cx.face_table(owner, true)?;
    let memos = cx.face_memos(frame, false)?;
    let class = frame_class(cx, &keys, frame)?;
    let mut faces = cx.face_ids()?;
    faces.sort_unstable_by_key(|&(id, _)| std::cmp::Reverse(id));
    let mut modified = Value::NIL;
    for (_, face) in faces {
        if locally_modified(cx, &keys, &class, face, frame, shown, memos)? { modified = cx.cons(face, modified); }
        cx.face_loop_quit()?;
    }
    Ok(modified)
}}

defsubr! { name = "romacs--recalc-faces-except", gnu = "romacs:faces", effect = Global,
fn romacs_recalc_faces_except(cx, frame: Value, modified: Value) {
    // `frame-set-background-mode''s recalculation, in `face-list' order.
    let frame = cx.face_frame(frame, true)?;
    if cx.frame_shares_faces(frame)? { return Ok(Value::NIL); }
    let mut kept = std::collections::HashSet::new();
    let mut tail = modified;
    while let Some((face, rest)) = cx.pair(tail) { kept.insert(face.0); tail = rest; }
    let keys = cx.face_keys();
    let mut class = frame_class(cx, &keys, frame)?;
    let mut faces = cx.face_ids()?;
    faces.sort_unstable_by_key(|&(id, _)| std::cmp::Reverse(id));
    for (_, face) in faces {
        if !kept.contains(&face.0) {
            recalc_in(cx, &keys, &class, face, frame)?;
            if carries_effects(&keys, face) { class = frame_class(cx, &keys, frame)?; }
        }
        cx.face_loop_quit()?;
    }
    Ok(Value::NIL)
}}

defsubr! { name = "romacs--face-spec-choose", gnu = "romacs:faces", effect = Reads,
fn romacs_face_spec_choose(cx, spec: Value; optional frame: Value, no_match_retval: Value) {
    let keys = cx.face_keys();
    let frame = cx.face_frame(frame, true)?;
    let class = frame_class(cx, &keys, frame)?;
    Ok(choose(cx, &keys, &class, spec, frame)?.unwrap_or(no_match_retval))
}}

defsubr! { name = "romacs--face-spec-set-match-display", gnu = "romacs:faces", effect = Reads,
fn romacs_face_spec_set_match_display(cx, display: Value, frame: Value) {
    let keys = cx.face_keys();
    let frame = cx.face_frame(frame, true)?;
    let class = frame_class(cx, &keys, frame)?;
    Ok(if display_matches(cx, &keys, &class, display, frame)? { Value::TRUE } else { Value::NIL })
}}

/// `face-attr-match-p'.
fn attrs_match(cx: &mut Leaf<'_>, keys: &FaceKeys, face: Value, attrs: Value, frame: Value) -> std::result::Result<bool, Fault> {
    let weight = keys.keywords[slot::WEIGHT - 1];
    let slant = keys.keywords[slot::SLANT - 1];
    // Emacs 20's :bold and :italic stand for the value whatever theirs is.
    let bold = plist_member(cx, attrs, keys.bold_key).is_some() && plist_member(cx, attrs, weight).is_none();
    let italic = plist_member(cx, attrs, keys.italic_key).is_some() && plist_member(cx, attrs, slant).is_none();
    let resolved = resolve(cx, face, true)?;
    let lface = cx.face_lookup(resolved, frame)?;
    if lface == Value::NIL { return Err(signal(cx, "Invalid face", face)); }
    for index in RESET_SLOTS {
        let attr = keys.keywords[index - 1];
        let specified = if bold && attr == weight { keys.bold }
            else if italic && attr == slant { keys.italic }
            else { match plist_member(cx, attrs, attr) { Some(tail) => uncons(cx, uncons(cx, tail)?.1)?.0, None => keys.unspecified } };
        let now = cx.vector_get(lface, slot_value(index))?;
        let now = if now == keys.ignore_defface { keys.unspecified } else { now };
        if specified != now && !cx.equal(specified, now).map_err(|_| Fault::Circular(now))? { return Ok(false); }
    }
    Ok(true)
}

defsubr! { name = "romacs--face-attr-match-p", gnu = "romacs:faces", effect = Reads,
fn romacs_face_attr_match_p(cx, face: Value, attrs: Value; optional frame: Value) {
    let keys = cx.face_keys();
    let frame = cx.face_frame(frame, true)?;
    Ok(if attrs_match(cx, &keys, face, attrs, frame)? { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "romacs--face-list", gnu = "romacs:faces", effect = Reads,
fn romacs_face_list(cx) {
    let mut faces = cx.face_ids()?;
    faces.sort_unstable_by_key(|&(id, _)| id);
    Ok(faces.into_iter().fold(Value::NIL, |list, (_, face)| cx.cons(face, list)))
}}
