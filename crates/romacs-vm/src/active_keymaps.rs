//! Which keymaps are active: GNU's `Fcurrent_active_maps`,
//! `current_minor_maps` and `click_position` in keymap.c, and
//! `get_local_map` in intervals.c.
//!
//! The policy is the Lisp-visible state GNU reads, as the buffer in question
//! has it: `overriding-terminal-local-map`, `overriding-local-map`,
//! `emulation-mode-map-alists`, `minor-mode-overriding-map-alist`,
//! `minor-mode-map-alist` and each mode's variable, the `keymap` and
//! `local-map` properties, the buffer's local map and the global map.
use crate::{coordinates::ByteOffset, data::Object, prim::Fault, Value, Vm};
use std::collections::HashSet;

type Faulty<T> = std::result::Result<T, Fault>;

/// A buffer's positions, as characters from 1: the accessible region, point
/// and the end of the whole text.
struct Extent { begv: i64, point: i64, zv: i64, z: i64 }

/// The position after OFFSET characters.
fn position(offset: usize) -> i64 {
    i64::try_from(offset).map_or(i64::MAX, |before| before + 1)
}

/// VALUE's car and cdr if it is a cons.  The tag first: a failed
/// `cons_pair` formats its error.
fn pair(vm: &Vm, value: Value) -> Option<(Value, Value)> {
    if value.tag() == 3 { vm.cons_pair(value).ok() } else { None }
}

impl Vm {
    /// `current-active-maps`: the active keymaps, highest priority first.
    pub(crate) fn current_active_maps(&mut self, olp: bool, position: Value) -> Faulty<Value> {
        self.ensure_keymaps();
        let buffer = self.keymap_buffer(position)?;
        self.in_buffer(buffer, |vm| vm.active_maps(olp, buffer, position))
    }
    /// The active keymaps of BUFFER, which is current.
    fn active_maps(&mut self, olp: bool, buffer: Value, position: Value) -> Faulty<Value> {
        let global = self.slots[self.names["#:vm2-global-keymap"]].value;
        let terminal = self.keymap_variable("overriding-terminal-local-map")?;
        if olp && terminal == Value::NIL {
            let overriding = self.keymap_variable("overriding-local-map")?;
            if overriding != Value::NIL { return Ok(self.list_value(&[overriding, global])); }
        }
        let (local_map, keymap) = self.property_keymaps(buffer, position)?;
        let mut maps = Vec::new();
        if olp && terminal != Value::NIL { maps.push(terminal); }
        if keymap != Value::NIL { maps.push(keymap); }
        maps.extend(self.current_minor_mode_maps()?.into_iter().map(|(_, map)| map));
        if local_map != Value::NIL { maps.push(local_map); }
        maps.push(global);
        Ok(self.list_value(&maps))
    }

    /// PROPERTY's keymap at the current buffer's point (`local_map_at`).
    pub(crate) fn keymap_at_point(&mut self, property: Value) -> Faulty<Value> {
        let buffer = self.keymap_buffer(Value::NIL)?;
        let point = self.keymap_extent(buffer)?.point;
        self.local_map_at(buffer, point, property)
    }

    /// The buffer whose keymaps POSITION asks for: that of a click's window
    /// or of a live window, else the current buffer.
    fn keymap_buffer(&mut self, position: Value) -> Faulty<Value> {
        let window = pair(self, position).map_or(position, |(window, _)| window);
        let shown = match self.object_if(window) {
            Some(Object::Window(w)) if w.live() => Some(w.buffer),
            _ => None,
        };
        let shown = shown.filter(|&buffer| matches!(self.object_if(buffer), Some(Object::Buffer(_) | Object::ByteBuffer(_))));
        if let Some(buffer) = shown { return Ok(buffer); }
        self.buffer_index().map_err(Fault::Error)?;
        Ok(self.slots[self.buffer_slot].value)
    }
    /// READ's value with BUFFER current meanwhile, as GNU makes a clicked
    /// window's buffer current: every variable READ consults is that
    /// buffer's.  READ returns its errors as values, so the buffer that
    /// was current is current again on every path.
    fn in_buffer<T>(&mut self, buffer: Value, read: impl FnOnce(&mut Self) -> T) -> T {
        let current = std::mem::replace(&mut self.slots[self.buffer_slot].value, buffer);
        let value = read(self);
        self.slots[self.buffer_slot].value = current;
        value
    }

    /// NAME's value in the current buffer, nil when it is void.
    fn keymap_variable(&mut self, name: &str) -> Faulty<Value> {
        let slot = self.slot(name);
        self.keymap_slot_value(slot)
    }
    fn keymap_slot_value(&mut self, slot: usize) -> Faulty<Value> {
        self.variable_value(slot).map(|v| if v == Value::UNBOUND { Value::NIL } else { v }).map_err(Fault::Error)
    }
    /// OBJECT's value if it is a symbol, nil when void.
    fn keymap_symbol_value(&mut self, object: Value) -> Faulty<Option<Value>> {
        if object == Value::NIL || object == Value::TRUE { return Ok(Some(object)); }
        let Some(slot) = object.index_if(7).and_then(|i| i.checked_sub(1)) else { return Ok(None) };
        self.keymap_slot_value(slot).map(Some)
    }

    fn keymap_extent(&self, buffer: Value) -> Faulty<Extent> {
        let extent = match self.object(buffer).map_err(Fault::Error)? {
            Object::Buffer(b) => {
                let at = |byte: ByteOffset| b.char_of_byte(byte).map(|c| position(c.0));
                let range = b.accessible();
                at(range.start).and_then(|begv| Ok(Extent { begv, point: at(b.point)?, zv: at(range.end)?, z: position(b.len_chars()) }))
            }
            Object::ByteBuffer(b) => Ok(Extent {
                begv: position(b.restriction.start.0), point: position(b.point.0),
                zv: position(b.restriction.end.0), z: position(b.bytes.len()),
            }),
            _ => Err("buffer required".into()),
        };
        extent.map_err(Fault::Error)
    }

    /// The `local-map` and `keymap` maps POSITION gives in BUFFER: those at
    /// point or at an integer or marker; for a click,
    /// the clicked string's properties, then those of the clicked buffer
    /// position, then the buffer's local map.
    fn property_keymaps(&mut self, buffer: Value, position: Value) -> Faulty<(Value, Value)> {
        let (local, keymap) = (self.intern("local-map"), self.intern("keymap"));
        if pair(self, position).is_none() {
            let extent = self.keymap_extent(buffer)?;
            let at = match (position.integer_value(), self.object_if(position)) {
                (Some(at), _) => at,
                (None, Some(Object::Marker(_))) => self.marker_position_value(position).map_err(Fault::Error)?
                    .integer_value().ok_or_else(|| Fault::Error("Marker does not point anywhere".into()))?,
                _ => extent.point,
            };
            if at < extent.begv || at > extent.zv { return Err(Fault::Range(buffer, position)); }
            return Ok((self.local_map_at(buffer, at, local)?, self.local_map_at(buffer, at, keymap)?));
        }
        let (mut local_map, mut map) = (Value::NIL, Value::NIL);
        if let Some((string, at)) = pair(self, self.nth(4, position)?) {
            let inside = self.str_view(string).zip(at.integer_value())
                .filter(|(view, at)| usize::try_from(*at).is_ok_and(|at| at < view.chars));
            if let Some((_, at)) = inside {
                local_map = self.text_property_at(string, at, local)?;
                map = self.text_property_at(string, at, keymap)?;
            }
        }
        let z = self.keymap_extent(buffer)?.z;
        if let Some(at) = self.posn_point(position)?.integer_value().filter(|at| (1..=z).contains(at)) {
            if local_map == Value::NIL { local_map = self.local_map_at(buffer, at, local)?; }
            if map == Value::NIL { map = self.local_map_at(buffer, at, keymap)?; }
        }
        if local_map == Value::NIL {
            let index = buffer.index(0).map_err(Fault::Error)? - 1;
            local_map = *self.local_keymap(index);
        }
        Ok((local_map, map))
    }

    /// `nth` of a position list.
    fn nth(&self, n: usize, list: Value) -> Faulty<Value> {
        let mut tail = list;
        for _ in 0..n {
            match pair(self, tail) {
                Some((_, rest)) => tail = rest,
                None => break,
            }
        }
        match pair(self, tail) {
            Some((item, _)) => Ok(item),
            None if tail == Value::NIL => Ok(Value::NIL),
            None => Err(Fault::Wrong("listp", list)),
        }
    }
    /// `posn-point`: the buffer position, else the one in the position's
    /// area slot.
    fn posn_point(&self, position: Value) -> Faulty<Value> {
        let point = self.nth(5, position)?;
        if point != Value::NIL { return Ok(point); }
        let area = pair(self, position).and_then(|(_, rest)| pair(self, rest)).map_or(Value::NIL, |(area, _)| area);
        Ok(match pair(self, area) {
            Some((point, _)) => point,
            None if area.integer_value().is_some() => area,
            None => Value::NIL,
        })
    }

    /// GNU's `get_local_map`: the keymap PROPERTY gives at POSITION of
    /// BUFFER, the character's first (`get-char-property`) and then what an
    /// insertion there would inherit (`get-pos-property`), with narrowing
    /// ignored.  For `local-map`, the buffer's local map otherwise.
    fn local_map_at(&mut self, buffer: Value, position: i64, property: Value) -> Faulty<Value> {
        let extent = self.keymap_extent(buffer)?;
        let at = Value::integer(position.clamp(extent.begv, extent.zv)).map_err(Fault::Error)?;
        let value = self.widened(buffer, |vm| {
            match vm.get_char_property_values(at, property, buffer).map_err(Fault::Error)? {
                Value::NIL => vm.get_pos_property_values(at, property, buffer),
                value => Ok(value),
            }
        })??;
        let map = if value == Value::NIL { None } else { self.resolve_keymap(value) };
        Ok(match map {
            Some(map) => map,
            None if self.names.get("local-map").is_some_and(|&slot| property == Value::reference(slot + 1, 7)) => {
                let index = buffer.index(0).map_err(Fault::Error)? - 1;
                *self.local_keymap(index)
            }
            None => Value::NIL,
        })
    }

    /// READ's value with BUFFER's narrowing lifted meanwhile, point where
    /// it is.
    fn widened<T>(&mut self, buffer: Value, read: impl FnOnce(&mut Self) -> T) -> Faulty<T> {
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        let narrowed = match self.objects.entry_mut(index) {
            Object::Buffer(b) => {
                let (view, whole) = (b.accessible(), ByteOffset(0)..ByteOffset(b.len_bytes()));
                (view != whole).then(|| { b.widen(); view })
            }
            Object::ByteBuffer(b) => {
                let (view, whole) = (b.restriction.clone(), ByteOffset(0)..ByteOffset(b.bytes.len()));
                (view != whole).then(|| { b.restriction = whole; view })
            }
            _ => None,
        };
        let value = read(self);
        let Some(view) = narrowed else { return Ok(value) };
        match self.objects.entry_mut(index) {
            Object::Buffer(b) => { let point = b.point; b.set_view(view, point); }
            Object::ByteBuffer(b) => {
                b.restriction = view;
                if b.shown().is_some() { b.mirror_now().map_err(Fault::Error)?; }
            }
            _ => {}
        }
        Ok(value)
    }

    /// GNU's `current_minor_maps`: the current buffer's active minor modes
    /// and their keymaps, highest priority first: the emulation alists, the
    /// overriding alist, then `minor-mode-map-alist` less the modes that
    /// alist names.
    pub(crate) fn current_minor_mode_maps(&mut self) -> Faulty<Vec<(Value, Value)>> {
        let overriding = self.keymap_variable("minor-mode-overriding-map-alist")?;
        let minor = self.keymap_variable("minor-mode-map-alist")?;
        let mut found = Vec::new();
        let mut alists = self.keymap_variable("emulation-mode-map-alists")?;
        for _ in 0..=self.list_limit() {
            let Some((alist, rest)) = pair(self, alists) else { break };
            let alist = self.keymap_symbol_value(alist)?.unwrap_or(alist);
            self.minor_mode_alist(alist, &HashSet::new(), &mut found)?;
            alists = rest;
        }
        self.minor_mode_alist(overriding, &HashSet::new(), &mut found)?;
        let overridden = self.alist_keys(overriding);
        self.minor_mode_alist(minor, &overridden, &mut found)?;
        Ok(found)
    }
    /// ALIST's (MODE . KEYMAP) entries whose MODE is on and not OVERRIDDEN,
    /// onto FOUND with the keymap's function indirection followed; an entry
    /// whose keymap is nil is none.
    fn minor_mode_alist(&mut self, mut alist: Value, overridden: &HashSet<u64>, found: &mut Vec<(Value, Value)>) -> Faulty<()> {
        for _ in 0..=self.list_limit() {
            let Some((entry, rest)) = pair(self, alist) else { break };
            alist = rest;
            let Some((mode, map)) = pair(self, entry) else { continue };
            if matches!(self.keymap_symbol_value(mode)?, None | Some(Value::NIL)) || overridden.contains(&mode.0) { continue; }
            let map = self.function_indirection(map);
            if map != Value::NIL { found.push((mode, map)); }
        }
        Ok(())
    }
    /// The keys of ALIST, as `assq` would find them: read once, so a mode
    /// is looked up in them without a scan.  Their identities, good until
    /// the next collection.
    fn alist_keys(&self, mut alist: Value) -> HashSet<u64> {
        let mut keys = HashSet::new();
        for _ in 0..=self.list_limit() {
            let Some((entry, rest)) = pair(self, alist) else { break };
            if let Some((key, _)) = pair(self, entry) { keys.insert(key.0); }
            alist = rest;
        }
        keys
    }
    /// `indirect-function`: the end of OBJECT's function chain, nil when a
    /// symbol in it has no function.
    fn function_indirection(&self, mut object: Value) -> Value {
        for _ in 0..=self.slots.len() {
            if object == Value::TRUE { return Value::NIL; }
            let Some(slot) = object.index_if(7).and_then(|i| i.checked_sub(1)) else { return object };
            object = self.slots.get(slot).map_or(Value::NIL, |s| s.function);
            if object == Value::UNBOUND { return Value::NIL; }
        }
        Value::NIL
    }
}
