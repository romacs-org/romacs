//! GNU's indirect buffers (buffer.c `make-indirect-buffer`): a base buffer
//! and its indirect buffers share one text, its text properties, its
//! modification counts and its markers' positions, while each keeps its own
//! point, narrowing, mark, local variables and overlays.
//!
//! The shared parts live in [`Shared`] cells, one value reached from every
//! member of the family, as GNU's `struct buffer_text` is reached through
//! each member's `text` pointer. An edit through one member moves the
//! others' point and narrowing as GNU's `pt_marker`, `begv_marker` and
//! `zv_marker` move: [`Vm::family_record`] gives each other member's
//! positions anchors before the edit and [`Vm::family_fetch`] reads them
//! back after it.
use crate::anchors::{Anchor, Bias};
use crate::coordinates::ByteOffset;
use crate::data::Object;
use crate::{Result, Value, Vm};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

/// A part of a buffer's text that its indirect buffers share. A clone is
/// a new, unshared copy; only [`Shared::share`] makes another handle.
///
/// Every handle to one cell is held by a different buffer object of one
/// VM's arena (`Vm::objects`), and a reference through a handle borrows the
/// object holding it. The arena lends a mutable borrow of one object only
/// while no other object is borrowed, except through a split borrow, and
/// the one split borrow of the arena (`threaded::quiet_insert`) pairs a
/// string with a buffer. So a mutable reference to a cell's value never
/// coexists with another reference to it.
pub(crate) struct Shared<T>(Rc<UnsafeCell<T>>);
// SAFETY: the cell's value is SnapshotSafe, and a world image maps the Rc
// and its value back where they were.
unsafe impl<T: romacs_snapshot::SnapshotSafe> romacs_snapshot::SnapshotSafe for Shared<T> {}

impl<T> Shared<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Rc::new(UnsafeCell::new(value)))
    }
    /// Another handle to this same value, for a new indirect buffer.
    pub(crate) fn share(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
    #[cfg(test)]
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: see the type's documentation.
        unsafe { &*self.0.get() }
    }
}
impl<T> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see the type's documentation.
        unsafe { &mut *self.0.get() }
    }
}
impl<T: Clone> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self::new((**self).clone())
    }
}
impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

/// The arena index of buffer object VALUE.
fn arena(value: Value) -> Result<usize> {
    value.index(0)?.checked_sub(1).ok_or_else(|| "buffer required".into())
}

/// Other members' point and narrowing across an edit of their shared text:
/// for each, anchors at its BEGV, PT and ZV, of GNU's `begv_marker`,
/// `pt_marker` and `zv_marker` insertion types.
pub(crate) struct FamilyMarks(Vec<(usize, [Anchor; 3])>);

impl Vm {
    /// The arena indexes of the buffers sharing buffer INDEX's text, its
    /// base first; empty for a buffer whose text is its own alone.
    pub(crate) fn text_family(&self, index: usize) -> Vec<usize> {
        let Some(Object::Buffer(b)) = self.objects.get(index) else { return Vec::new() };
        if b.base == Value::NIL && b.indirect.is_empty() {
            return Vec::new();
        }
        let base = if b.base == Value::NIL { index } else { arena(b.base).expect("live base") };
        let Object::Buffer(base_buffer) = &self.objects[base] else { return Vec::new() };
        std::iter::once(base).chain(base_buffer.indirect.iter().map(|&v| arena(v).expect("indirect buffer"))).collect()
    }

    /// Before an edit of buffer INDEX's text: anchors for the positions of
    /// every other buffer sharing it, or None when no other does.
    pub(crate) fn family_record(&mut self, index: usize) -> Option<FamilyMarks> {
        let family = self.text_family(index);
        if family.is_empty() {
            return None;
        }
        let mut marks = Vec::with_capacity(family.len() - 1);
        for member in family.into_iter().filter(|&m| m != index) {
            let Object::Buffer(b) = self.objects.entry_mut(member) else { continue };
            let (r, pt) = (b.accessible(), b.point);
            let mut at = |p: ByteOffset, bias| b.anchors.create(p.0 as u32, bias).expect("position in shared text");
            marks.push((member, [at(r.start, Bias::Left), at(pt, Bias::Left), at(r.end, Bias::Right)]));
        }
        Some(FamilyMarks(marks))
    }

    /// After the edit `family_record` prepared: each other member's point
    /// and narrowing where the edit moved their anchors.
    pub(crate) fn family_fetch(&mut self, marks: Option<FamilyMarks>) {
        let Some(FamilyMarks(marks)) = marks else { return };
        for (member, handles) in marks {
            let Object::Buffer(b) = self.objects.entry_mut(member) else { continue };
            let [begv, pt, zv] = handles.map(|h| ByteOffset(b.anchors.resolve(h).expect("family mark") as usize));
            for h in handles {
                b.anchors.remove(h);
            }
            b.set_view(begv..zv, pt);
            // An insertion that kept a member's turned-over overlay in place
            // moved the overlays around it: reorder that member's index too.
            let starts: Vec<Anchor> = b.reversed_pairs.keys().copied().collect();
            for start in starts {
                b.intervals.refresh(start, &b.anchors);
            }
        }
    }

    /// Fmake_indirect_buffer's storage: a live buffer NAME sharing the text
    /// of BASE (or of BASE's own base), at BASE's point and narrowing, with
    /// a new buffer's variables. CLONE and the hooks are its caller's.
    pub(crate) fn make_indirect_buffer(&mut self, base: Value, name: &str, inhibit_hooks: bool) -> Result<Value> {
        let base = match &self.objects[arena(base)?] {
            Object::Buffer(b) if b.base != Value::NIL => b.base,
            Object::Buffer(_) => base,
            // DIVERGENCES.md VM2-INDIRECT1.
            Object::ByteBuffer(_) => return Err("VM2 has no indirect buffer of a unibyte buffer".into()),
            _ => return Err("buffer required".into()),
        };
        let Object::Buffer(b) = &self.objects[arena(base)?] else { unreachable!("base buffer") };
        let mut buffer = b.share_text();
        buffer.base = base;
        buffer.state.inhibit_buffer_hooks = inhibit_hooks;
        let value = self.register_buffer(buffer, name)?;
        let Object::Buffer(b) = self.objects.entry_mut(arena(base)?) else { unreachable!("base buffer") };
        b.indirect.push(value);
        Ok(value)
    }

    /// clone_per_buffer_values (buffer.c:727) from BASE to its new indirect
    /// buffer BUFFER: the per-buffer variables, with a positioned mark
    /// remade in BUFFER and the visited file forgotten, the Lisp locals,
    /// the keymap and tables, and copies of the overlays.
    pub(crate) fn clone_indirect_state(&mut self, buffer: Value, base: Value) -> Result<()> {
        let Object::Buffer(from) = &self.objects[arena(base)?] else { return Err("buffer required".into()) };
        let mut per_buffer = from.per_buffer.clone();
        let (keymap, cases, syntax, category) = (from.local_keymap, from.case_tables, from.syntax_table, from.category_table);
        // copy_overlays walks the overlays in tree order: ascending start,
        // equal starts latest inserted first (overlay_order::tree_order).
        let mut overlays: Vec<(Value, u32, u32, u64)> = self.objects.iter().enumerate().filter_map(|(i, object)| match object {
            Object::Overlay(o) if o.buffer == base => Some((
                Value::reference(i + 1, 0),
                from.anchors.resolve(o.start?)?,
                from.anchors.resolve(o.end?)?,
                o.inserted,
            )),
            _ => None,
        }).collect();
        overlays.sort_by_key(|&(_, start, _, inserted)| (start, std::cmp::Reverse(inserted)));
        per_buffer.reset_visit();
        // Its list is its base's (`per_buffer_owner'): a copy of the head
        // would hold the history as it stands now, past any cut.
        per_buffer.clear_local(self.undo_list_index());
        let mark = per_buffer.editor_mark();
        let Object::Buffer(to) = self.objects.entry_mut(arena(buffer)?) else { return Err("buffer required".into()) };
        to.per_buffer = per_buffer;
        to.local_keymap = keymap;
        to.case_tables = cases;
        to.syntax_table = syntax;
        to.category_table = category;
        self.clone_lisp_locals(base, buffer)?;
        for name in ["buffer-file-name", "buffer-file-truename", "buffer-auto-save-file-name", "buffer-backed-up", "local-minor-modes"] {
            self.set_per_buffer_of(arena(buffer)?, name, Value::NIL);
        }
        self.set_per_buffer_of(arena(buffer)?, "buffer-display-count", Value::integer(0)?);
        // A mark that points somewhere is remade in BUFFER; another marker
        // is the same object in both, as GNU copies it.
        if let Some(mark) = mark
            && let Object::Marker(m) = self.object(mark)?
            && m.buffer == base
            && let Some(anchor) = m.anchor
        {
            let (bias, at) = (m.bias, self.marker_anchors(base)?.resolve(anchor).ok_or("invalid mark")?);
            let copy = self.allocate_object(Object::Marker(Box::new(crate::annotations::Marker { buffer, anchor: None, bias, last_position: 0 })));
            let anchor = self.marker_anchors_mut(buffer)?.create(at, bias).ok_or("invalid mark")?;
            let Object::Marker(m) = self.objects.entry_mut(arena(copy)?) else { unreachable!("new marker") };
            m.anchor = Some(anchor);
            let Object::Buffer(to) = self.objects.entry_mut(arena(buffer)?) else { unreachable!("indirect buffer") };
            to.per_buffer.set_editor_mark(copy);
        }
        for (overlay, start, end, _) in overlays {
            self.copy_overlay(overlay, start, end, buffer)?;
        }
        Ok(())
    }

    /// buffer_lisp_local_variables (buffer.c) copied: every Lisp local of
    /// FROM becomes a binding of TO's own with FROM's value.
    fn clone_lisp_locals(&mut self, from: Value, to: Value) -> Result<()> {
        for slot in self.local_slots(from.index(0)? - 1) {
            let table = self.slots[slot].local;
            if table == Value::NIL {
                continue;
            }
            if let Some(binding) = self.declaration_hash_get(table, from)? {
                let value = self.slots[binding.index(7)? - 1].value;
                let local = self.allocate_local_binding_slot();
                self.slots[local].value = value;
                self.declaration_hash_put(table, to, Value::reference(local + 1, 7))?;
                self.note_local_binding(to, slot);
            }
        }
        Ok(())
    }

    /// copy_overlays (buffer.c): OVERLAY, at START..END of the shared text,
    /// copied into BUFFER with its insertion types and a copy of its
    /// property list.
    fn copy_overlay(&mut self, overlay: Value, start: u32, end: u32, buffer: Value) -> Result<()> {
        let source = self.overlay(overlay)?.clone();
        let (Some(a), Some(z)) = (source.start, source.end) else { return Ok(()) };
        let evaporates = matches!(self.object(source.buffer)?, Object::Buffer(b) if b.evaporating.contains_key(&a));
        let anchors = self.marker_anchors_mut(buffer)?;
        let (front, rear) = (anchors.bias(a).ok_or("invalid overlay")?, anchors.bias(z).ok_or("invalid overlay")?);
        let start = anchors.create(start, front).ok_or("invalid overlay")?;
        let end = anchors.create(end, rear).ok_or("invalid overlay")?;
        let copy = self.allocate_object(Object::Overlay(Box::new(crate::annotations::Overlay {
            buffer, start: Some(start), end: Some(end), front: source.front, rear: source.rear, properties: source.properties.clone(),
            inserted: 0,
        })));
        let Object::Buffer(b) = self.objects.entry_mut(arena(buffer)?) else { unreachable!("indirect buffer") };
        // The turned-over overlays are the shared text's, of every member.
        if b.reversed_pairs.contains_key(&a) {
            b.reversed_pairs.insert(start, end);
        }
        if evaporates {
            b.evaporating.insert(start, copy);
        }
        b.intervals.insert(copy, start, end, &b.anchors);
        self.note_overlay_insertion(buffer, copy)?;
        self.overlay_changed(copy)
    }

    /// Fkill_buffer (buffer.c:2019): killing a base buffer kills its
    /// indirect buffers first.
    pub(crate) fn kill_indirect_buffers(&mut self, value: Value) -> Result<()> {
        if let Some(Object::Buffer(b)) = self.objects.get(arena(value)?)
            && b.base == Value::NIL
        {
            for indirect in b.indirect.clone() {
                self.kill_buffer_model(indirect)?;
            }
        }
        Ok(())
    }

    /// Fkill_buffer (buffer.c:2111) for an indirect buffer VALUE: it leaves
    /// its base's list and takes its markers and overlays out of the text
    /// it shared.
    pub(crate) fn leave_base(&mut self, value: Value) -> Result<()> {
        let index = arena(value)?;
        let Some(Object::Buffer(b)) = self.objects.get(index) else { return Ok(()) };
        let base = b.base;
        if base == Value::NIL {
            return Ok(());
        }
        if let Object::Buffer(base) = self.objects.entry_mut(arena(base)?) {
            base.indirect.retain(|&v| v != value);
        }
        let mut overlays = Vec::new();
        let mut markers = Vec::new();
        for i in self.buffer_referrers(index) {
            match &self.objects[i] {
                Object::Marker(m) if m.buffer == value => markers.extend(m.anchor),
                Object::Overlay(o) if o.buffer == value => overlays.push(Value::reference(i + 1, 0)),
                _ => {}
            }
        }
        for overlay in overlays {
            self.delete_overlay_object(overlay)?;
        }
        let Object::Buffer(b) = self.objects.entry_mut(index) else { unreachable!("indirect buffer") };
        for anchor in markers {
            b.anchors.remove(anchor);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::CharOffset;

    /// Unregister BUFFER's name and drop it from the buffer list, so only
    /// the collector's explicit roots keep it.
    fn unlist(vm: &mut Vm, name: &str) {
        let slot = vm.slot(&format!("#:vm2-buffer:{name}"));
        vm.slots[slot].value = Value::UNBOUND;
        vm.set_buffer_order(&[]);
    }

    #[test]
    fn indirect_buffer_keeps_its_base_and_shared_properties_through_moving_collection() {
        let mut vm = Vm::new();
        // Garbage below the buffers, so collection moves them.
        for _ in 0..64 {
            vm.allocate_object(Object::String(Box::new(crate::data::LispString::new("garbage"))));
        }
        let base = vm.create_named_buffer("gc-base", false).unwrap();
        let leaf = vm.allocate_object(Object::String(Box::new(crate::data::LispString::new("λ"))));
        let vector = vm.allocate_object(Object::Vector(vec![leaf, Value::integer(7).unwrap()]));
        let Object::Buffer(b) = vm.objects.entry_mut(arena(base).unwrap()) else { panic!() };
        assert!(b.replace(ByteOffset(0)..ByteOffset(0), "aλЖz".as_bytes()).is_ok());
        b.properties.put(ByteOffset(1)..ByteOffset(5), "probe", vector);
        let indirect = vm.make_indirect_buffer(base, "gc-indirect", false).unwrap();
        unlist(&mut vm, "gc-base");
        unlist(&mut vm, "gc-indirect");
        let mut roots = [indirect];
        vm.collect(&mut roots);
        let Object::Buffer(i) = vm.object(roots[0]).unwrap() else { panic!("indirect buffer moved away") };
        let base = i.base;
        let Object::Buffer(b) = vm.object(base).unwrap() else { panic!("base buffer reclaimed") };
        assert!(b.shares_body(i) && b.indirect == [roots[0]]);
        // The shared properties were rewritten once, from the base.
        let probe = i.properties.get_property(ByteOffset(1), "probe").unwrap();
        let Object::Vector(v) = vm.object(probe).unwrap() else { panic!("property value lost") };
        assert!(matches!(vm.object(v[0]).unwrap(), Object::String(s) if s.as_str() == "λ"));
        assert_eq!(v[1], Value::integer(7).unwrap());
        assert_eq!(i.slice(ByteOffset(0)..i.byte_of_char(CharOffset(4)).unwrap()).unwrap(), "aλЖz");
        // Neither rooted, both go, with their directories and the property
        // graph; what stays is the VM's own (the standard case tables).
        let live = vm.heap_objects();
        vm.collect(&mut []);
        assert_eq!(live - vm.heap_objects(), 6);
    }

    #[test]
    fn edit_through_one_member_moves_the_others_positions_as_markers() {
        let mut vm = Vm::new();
        let base = vm.create_named_buffer("pos-base", false).unwrap();
        let Object::Buffer(b) = vm.objects.entry_mut(arena(base).unwrap()) else { panic!() };
        assert!(b.replace(ByteOffset(0)..ByteOffset(0), b"0123456789").is_ok());
        let indirect = vm.make_indirect_buffer(base, "pos-indirect", false).unwrap();
        let (bi, ii) = (arena(base).unwrap(), arena(indirect).unwrap());
        let Object::Buffer(i) = vm.objects.entry_mut(ii) else { panic!() };
        assert!(i.narrow(ByteOffset(2)..ByteOffset(6)).is_ok());
        i.point = ByteOffset(2);
        // An insertion at the indirect buffer's BEGV and point leaves them
        // before it and its ZV after the text past it.
        let marks = vm.family_record(bi);
        let Object::Buffer(b) = vm.objects.entry_mut(bi) else { panic!() };
        assert!(b.replace(ByteOffset(2)..ByteOffset(2), b"ab").is_ok());
        vm.family_fetch(marks);
        let Object::Buffer(i) = &vm.objects[ii] else { panic!() };
        assert_eq!((i.accessible(), i.point), (ByteOffset(2)..ByteOffset(8), ByteOffset(2)));
        // A deletion around them collapses them to its start.
        let marks = vm.family_record(bi);
        let Object::Buffer(b) = vm.objects.entry_mut(bi) else { panic!() };
        assert!(b.replace(ByteOffset(1)..ByteOffset(9), b"").is_ok());
        vm.family_fetch(marks);
        let Object::Buffer(i) = &vm.objects[ii] else { panic!() };
        assert_eq!((i.accessible(), i.point), (ByteOffset(1)..ByteOffset(1), ByteOffset(1)));
        // The marks are gone from the shared text.
        assert!(i.anchors.is_empty());
    }
}
