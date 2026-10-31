//! insdel.c's bracket around a change of the current buffer's text:
//! prepare_to_modify_buffer's checks and the hooks they find
//! (verify_interval_modification in textprop.c, report_overlay_modification
//! in buffer.c), the storage edit with its undo records (undo.c), and whether
//! signal_after_change has anything to run. The hooks themselves run in Lisp
//! (`romacs--before-change' and `romacs--after-change' in rlisp/vm/insdel.el)
//! and only when one can run: an edit with no hook and no undo-auto Lisp runs
//! no Lisp at all.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};
use crate::prim::Fault;
use data::Object;

type Faulty<T> = std::result::Result<T, Fault>;

/// What prepare_to_modify_buffer found before a change of START..END.
pub(crate) struct Prepared {
    /// `inhibit-modification-hooks' was set: no hook runs and the mark stays.
    pub inhibited: bool,
    /// `undo-auto--undoable-change' runs first (the undo list is not t).
    pub undoable: bool,
    /// Some Lisp must run before the change.
    pub before: bool,
    /// The changed characters' distinct `modification-hooks' values.
    pub modification_hooks: Vec<Value>,
    /// An insertion's `insert-behind-hooks' and `insert-in-front-hooks'.
    pub behind: Value,
    pub in_front: Value,
    /// Some overlay hook would run if the change met the overlays now.
    /// When other Lisp runs first, `romacs--before-change' finds them
    /// afresh after it, as report_overlay_modification does.
    pub overlays: bool,
    /// `after-change-functions' was set (not inhibited).
    pub after: bool,
}

/// The variables (and the one function) the bracket reads on every change.
#[derive(Clone, Copy)]
pub(crate) struct ChangeSlots {
    read_only: usize,
    inhibit_read_only: usize,
    inhibit_hooks: usize,
    before: usize,
    after: usize,
    first: usize,
    truename: usize,
    filename: usize,
    mark_active: usize,
    select_regions: usize,
    deactivate: usize,
    undoable: usize,
    pub(crate) inhibit_record_point: usize,
}
romacs_snapshot::snapshot_struct!(ChangeSlots { read_only, inhibit_read_only, inhibit_hooks, before, after, first, truename, filename, mark_active, select_regions, deactivate, undoable, inhibit_record_point });

/// An insertion that may skip the bracket (`Vm::quiet_insertion'): whether
/// modification hooks are inhibited, and buffer, point and modified state
/// for its undo record when undo is on.
pub(crate) struct QuietInsertion {
    inhibited: bool,
    undo: Option<(usize, i64, bool)>,
}

/// TMEM: SYMBOL is in SET, a list, or SET is any other non-nil value.
fn tmem(vm: &Vm, symbol: Value, set: Value) -> bool {
    if vm.cons_pair(set).is_err() {
        return set != Value::NIL;
    }
    let mut tail = set;
    let mut walk = annotations::TailWalk::new(set);
    while let Ok((item, next)) = vm.cons_pair(tail) {
        if item == symbol {
            return true;
        }
        tail = next;
        if walk.cycle(tail) {
            return false;
        }
    }
    false
}

impl Vm {
    fn current_buffer_value(&mut self) -> Faulty<Value> {
        self.buffer_index().map_err(Fault::Error)?;
        Ok(self.slots[self.buffer_slot].value)
    }

    /// The current buffer's point, BEGV and ZV as character positions.
    pub(crate) fn point_extent(&mut self) -> Faulty<(i64, i64, i64)> {
        let index = self.buffer_index().map_err(Fault::Error)?;
        let char_at = |b: &annotations::AnnotatedBuffer, at| b.char_of_byte(at).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
        Ok(match &self.objects[index] {
            Object::Buffer(b) => {
                let r = b.accessible();
                (char_at(b, b.point)?, char_at(b, r.start)?, char_at(b, r.end)?)
            }
            Object::ByteBuffer(b) => (b.point.0 as i64 + 1, b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
            _ => return Err(Fault::Error("buffer required".into())),
        })
    }

    /// The current buffer's point as a character position.
    pub(crate) fn point_char(&mut self) -> Faulty<i64> {
        let index = self.buffer_index().map_err(Fault::Error)?;
        Ok(match &self.objects[index] {
            Object::Buffer(b) => b.char_of_byte(b.point).map_err(Fault::Error)?.0 as i64 + 1,
            Object::ByteBuffer(b) => b.point.0 as i64 + 1,
            _ => return Err(Fault::Error("buffer required".into())),
        })
    }

    /// The byte offset of the current buffer's character position POSITION.
    fn change_byte(&self, buffer: Value, position: i64) -> Faulty<ByteOffset> {
        let at = usize::try_from(position - 1).map_err(|_| Fault::Error("position out of range".into()))?;
        match self.object_if(buffer) {
            Some(Object::Buffer(b)) => b.byte_of_char(CharOffset(at)).map_err(Fault::Error),
            _ => Ok(ByteOffset(at)),
        }
    }

    pub(crate) fn change_slots(&mut self) -> ChangeSlots {
        if let Some(slots) = self.change_slots { return slots; }
        let slots = ChangeSlots {
            read_only: self.slot("buffer-read-only"),
            inhibit_read_only: self.slot("inhibit-read-only"),
            inhibit_hooks: self.slot("inhibit-modification-hooks"),
            before: self.slot("before-change-functions"),
            after: self.slot("after-change-functions"),
            first: self.slot("first-change-hook"),
            truename: self.slot("buffer-file-truename"),
            filename: self.slot("buffer-file-name"),
            mark_active: self.slot("mark-active"),
            select_regions: self.slot("select-active-regions"),
            deactivate: self.slot("deactivate-mark"),
            undoable: self.slot("undo-auto--undoable-change"),
            inhibit_record_point: self.slot("undo-inhibit-record-point"),
        };
        self.change_slots = Some(slots);
        slots
    }

    /// A variable's current value, nil when void.
    fn change_value(&mut self, slot: usize) -> Faulty<Value> {
        let value = self.variable_value(slot).map_err(Fault::Error)?;
        Ok(if value == Value::UNBOUND { Value::NIL } else { value })
    }

    fn undo_auto_bound(&mut self) -> bool {
        let slot = self.change_slots().undoable;
        !matches!(self.slots[slot].function, Value::UNBOUND | Value::NIL)
    }

    /// prepare_to_modify_buffer_1 for a change of START..END of the current
    /// buffer (an insertion when they are equal), up to the Lisp it runs:
    /// barf-if-buffer-read-only, verify_interval_modification's read-only
    /// checks and hooks, and what signal_before_change would run.
    pub(crate) fn prepare_change(&mut self, start: i64, end: i64) -> Faulty<Prepared> {
        self.prepare_change_of(start, end, false)
    }

    /// prepare_change for a change that gives the text an interval tree
    /// first if it has none (a property change: add_text_properties_1
    /// makes the root interval before it prepares, so the read-only checks
    /// always run).
    pub(crate) fn prepare_change_of(&mut self, start: i64, end: i64, intervals: bool) -> Faulty<Prepared> {
        let slots = self.change_slots();
        let buffer = self.current_buffer_value()?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        let intervals = intervals || self.buffer_has_intervals(buffer);
        let overlays = self.buffer_has_overlays(index);
        // Positions are read only for the text's intervals and overlays.
        let bytes = if intervals || overlays {
            Some((self.change_byte(buffer, start)?, self.change_byte(buffer, end)?))
        } else {
            None
        };
        // Fbarf_if_buffer_read_only (START).
        if self.change_value(slots.read_only)? != Value::NIL
            && self.change_value(slots.inhibit_read_only)? == Value::NIL
            && match bytes {
                Some((start_byte, _)) if intervals => {
                    let plist = self.char_properties(buffer, start_byte);
                    self.text_property_in(&plist, "inhibit-read-only")? == Value::NIL
                }
                _ => true,
            }
        {
            return Err(self.text_property_signal("buffer-read-only", &[buffer]));
        }
        let undoable = self.undo_list_of(index) != Value::TRUE && self.undo_auto_bound();
        let inhibited = self.change_value(slots.inhibit_hooks)? != Value::NIL;
        let (mut behind, mut in_front, mut modification_hooks) = (Value::NIL, Value::NIL, Vec::new());
        if let Some((start_byte, end_byte)) = bytes.filter(|_| intervals) {
            if start == end {
                (behind, in_front) = self.verify_insertion(buffer, start, start_byte)?;
            } else {
                self.check_text_property_edit(buffer, start_byte..end_byte)?;
                if !inhibited {
                    modification_hooks = self.modification_hooks_in(buffer, start_byte..end_byte)?;
                }
            }
        }
        let mut prepared = Prepared { inhibited, undoable, before: undoable, modification_hooks, behind, in_front, overlays: false, after: false };
        if inhibited {
            return Ok(prepared);
        }
        if let Some((start_byte, end_byte)) = bytes.filter(|_| overlays) {
            prepared.overlays = !self.overlay_change_hooks(buffer, start_byte, end_byte)?.is_empty();
        }
        let unmodified = !self.buffer_state(buffer).map_err(Fault::Error)?.modified();
        let after = self.change_value(slots.after)?;
        prepared.after = after != Value::NIL;
        prepared.before |= !prepared.modification_hooks.is_empty()
            || prepared.overlays
            || self.change_value(slots.before)? != Value::NIL
            || (unmodified && self.change_value(slots.first)? != Value::NIL)
            // lock_file for a visited file's first change.
            || (unmodified && self.change_locks_file(buffer)?)
            // select-active-regions saves the region before it changes.
            || (self.change_value(slots.mark_active)? != Value::NIL
                && self.change_value(slots.select_regions)? != Value::NIL);
        Ok(prepared)
    }

    /// Whether a change of BUFFER locks a visited file: its base's, when
    /// indirect, names one in `buffer-file-truename' and `buffer-file-name'
    /// (insdel.c:2160-2174).
    fn change_locks_file(&mut self, buffer: Value) -> Faulty<bool> {
        let slots = self.change_slots();
        let base = match self.object_if(buffer) { Some(Object::Buffer(b)) if b.base != Value::NIL => b.base, _ => buffer };
        let mut names = |slot| -> Faulty<bool> {
            let value = if base == buffer { self.change_value(slot)? } else { self.value_in_buffer(slot, base).map_err(Fault::Error)? };
            Ok(!matches!(value, Value::NIL | Value::UNBOUND))
        };
        Ok(names(slots.truename)? && names(slots.filename)?)
    }

    /// verify_interval_modification for an insertion at POSITION (byte AT):
    /// read-only text refuses it by its stickiness, and the characters
    /// around it give the insert-behind and insert-in-front hooks.
    fn verify_insertion(&mut self, buffer: Value, position: i64, at: ByteOffset) -> Faulty<(Value, Value)> {
        let (_, begv, zv) = self.point_extent()?;
        let after = (position < zv).then(|| self.char_properties(buffer, at));
        let before = if position > begv {
            let previous = self.change_byte(buffer, position - 1)?;
            Some(self.char_properties(buffer, previous))
        } else {
            None
        };
        let inhibit = self.text_property_variable("inhibit-read-only")?;
        if inhibit == Value::NIL || self.cons_pair(inhibit).is_ok() {
            let (read_only, category) = (self.intern("read-only"), self.intern("category"));
            let raw = |plist: &[(String, Value)]| plist.iter().any(|(k, v)| k == "read-only" && *v != Value::NIL);
            let refused = |vm: &mut Self, value: Value| -> Faulty<bool> {
                Ok(value != Value::NIL && vm.text_property_find(inhibit, |item| item == value)?.is_none())
            };
            if let Some(plist) = &after {
                let value = self.text_property_in(plist, "read-only")?;
                if refused(self, value)? {
                    let sticky = self.text_property_in(plist, "front-sticky")?;
                    if tmem(self, read_only, sticky) || (!raw(plist) && tmem(self, category, sticky)) {
                        return Err(self.text_read_only(value));
                    }
                }
            }
            if let Some(plist) = &before {
                let value = self.text_property_in(plist, "read-only")?;
                if refused(self, value)? {
                    let nonsticky = self.text_property_in(plist, "rear-nonsticky")?;
                    if !tmem(self, read_only, nonsticky) && (raw(plist) || !tmem(self, category, nonsticky)) {
                        return Err(self.text_read_only(value));
                    }
                }
            }
        }
        let behind = match &before { Some(plist) => self.text_property_in(plist, "insert-behind-hooks")?, None => Value::NIL };
        let in_front = match &after { Some(plist) => self.text_property_in(plist, "insert-in-front-hooks")?, None => Value::NIL };
        Ok((behind, in_front))
    }

    fn text_read_only(&mut self, value: Value) -> Fault {
        let data = if self.str_view(value).is_some() { vec![value] } else { vec![] };
        self.text_property_signal("text-read-only", &data)
    }

    fn buffer_has_overlays(&self, index: usize) -> bool {
        matches!(&self.objects[index], Object::Buffer(b) if !b.intervals.is_empty())
    }

    /// overlay_change_hooks for character positions START..END of the
    /// current buffer.
    pub(crate) fn overlay_change_hooks_at(&mut self, start: i64, end: i64) -> Faulty<Vec<(Value, Value)>> {
        let buffer = self.current_buffer_value()?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        if !self.buffer_has_overlays(index) {
            return Ok(Vec::new());
        }
        // Earlier hooks may have shrunk the buffer below the change's
        // original bounds; GNU still selects with those bounds.
        let z = match self.object_if(buffer) {
            Some(Object::Buffer(b)) => b.len_chars() as i64 + 1,
            _ => return Ok(Vec::new()),
        };
        if start > z || end > z {
            return self.overlay_change_hooks_beyond(buffer, start, end);
        }
        let (start, end) = (self.change_byte(buffer, start)?, self.change_byte(buffer, end)?);
        self.overlay_change_hooks(buffer, start, end)
    }

    /// report_overlay_modification for bounds past Z, in character
    /// positions: ITREE_FOREACH's selection over BEGIN..END (widened by one
    /// for an insertion; an empty overlay is met at its position) and the
    /// three hook tests, all with the original bounds and insertion flag.
    fn overlay_change_hooks_beyond(&mut self, buffer: Value, begin: i64, end: i64) -> Faulty<Vec<(Value, Value)>> {
        let Some(Object::Buffer(b)) = self.object_if(buffer) else { return Ok(Vec::new()) };
        let whole = ByteOffset(0)..ByteOffset(b.len_bytes());
        let mut spans = Vec::new();
        self.visit_overlays_in(buffer, whole, |overlay, span| spans.push((span.start, span.end, overlay))).map_err(Fault::Error)?;
        let Some(Object::Buffer(b)) = self.object_if(buffer) else { return Ok(Vec::new()) };
        let mut found = Vec::new();
        for (s, e, overlay) in spans {
            let s = b.char_of_byte(s).map_err(Fault::Error)?.0 as i64 + 1;
            let e = b.char_of_byte(e).map_err(Fault::Error)?.0 as i64 + 1;
            found.push((s, e, overlay));
        }
        found.sort_by_key(|&(s, _, _)| s);
        let insertion = begin == end;
        let (low, high) = if insertion { (begin - 1, end + 1) } else { (begin, end) };
        let names = [self.intern("insert-in-front-hooks"), self.intern("insert-behind-hooks"), self.intern("modification-hooks")];
        let mut hooks = Vec::new();
        for (s, e, overlay) in found {
            if !((low < e && s < high) || (s == e && low == s)) {
                continue;
            }
            let get = |vm: &Self, name: Value| vm.overlay_property(overlay, name).unwrap_or(Value::NIL);
            if insertion && (begin == s || end == s) {
                let prop = get(self, names[0]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
            if insertion && (begin == e || end == e) {
                let prop = get(self, names[1]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
            if end > s && begin < e {
                let prop = get(self, names[2]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
        }
        Ok(hooks)
    }

    /// report_overlay_modification's scan before a change of START..END:
    /// each overlay's insert-in-front-hooks when an insertion is at its
    /// start, insert-behind-hooks at its end, and modification-hooks when
    /// the change overlaps it, in ascending overlay order.
    pub(crate) fn overlay_change_hooks(&mut self, buffer: Value, start: ByteOffset, end: ByteOffset) -> Faulty<Vec<(Value, Value)>> {
        let Some(Object::Buffer(b)) = self.object_if(buffer) else { return Ok(Vec::new()) };
        if b.intervals.is_empty() {
            return Ok(Vec::new());
        }
        let insertion = start == end;
        // An insertion also meets the overlays that end or start at it.
        let (low, high) = if insertion {
            let c = b.char_of_byte(start).map_err(Fault::Error)?.0;
            let low = if c > 0 { b.byte_of_char(CharOffset(c - 1)).map_err(Fault::Error)? } else { start };
            let high = if end.0 < b.len_bytes() { b.byte_of_char(CharOffset(c + 1)).map_err(Fault::Error)? } else { end };
            (low, high)
        } else {
            (start, end)
        };
        let mut found = Vec::new();
        self.visit_overlays_in(buffer, low..high, |overlay, span| found.push((span.start, span.end, overlay))).map_err(Fault::Error)?;
        found.sort_by_key(|&(s, _, _)| s);
        let names = [self.intern("insert-in-front-hooks"), self.intern("insert-behind-hooks"), self.intern("modification-hooks")];
        let mut hooks = Vec::new();
        for (s, e, overlay) in found {
            let get = |vm: &Self, name: Value| vm.overlay_property(overlay, name).unwrap_or(Value::NIL);
            if insertion && (start == s || end == s) {
                let prop = get(self, names[0]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
            if insertion && (start == e || end == e) {
                let prop = get(self, names[1]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
            if end > s && start < e {
                let prop = get(self, names[2]);
                if prop != Value::NIL { hooks.push((prop, overlay)); }
            }
        }
        Ok(hooks)
    }

    /// Whether signal_after_change has Lisp to run now, given whether the
    /// change met overlay hooks and (an insertion) text insert hooks.
    pub(crate) fn after_change_runs(&mut self, overlays: bool, insert_hooks: bool) -> Faulty<bool> {
        let slots = self.change_slots();
        if self.change_value(slots.inhibit_hooks)? != Value::NIL {
            return Ok(false);
        }
        Ok(overlays || insert_hooks || self.change_value(slots.after)? != Value::NIL)
    }

    /// Whether an insertion at the end of the current buffer may skip the
    /// bracket: the text has no intervals or overlays, and the insertion runs
    /// no Lisp before or after it. Undo, when on, is then recorded by
    /// `quiet_inserted'.
    pub(crate) fn quiet_insertion(&mut self) -> Option<QuietInsertion> {
        let index = self.buffer_index().ok()?;
        let Object::Buffer(b) = &self.objects[index] else { return None };
        if b.properties.has_intervals() || !b.intervals.is_empty() { return None; }
        let end = b.len_chars() as i64 + 1;
        let unmodified = !b.state.modified();
        let prepared = self.prepare_change(end, end).ok()?;
        if prepared.before || prepared.after { return None; }
        let undo = (self.undo_list_of(index) != Value::TRUE).then_some((index, end, unmodified));
        Some(QuietInsertion { inhibited: prepared.inhibited, undo })
    }

    /// The rest of a quiet insertion once its text is in: record_insert,
    /// and deactivate-mark unless modification hooks are inhibited.
    pub(crate) fn quiet_inserted(&mut self, quiet: QuietInsertion, length: usize) -> Faulty<()> {
        // insert_from_string does nothing at all for an empty string.
        if length == 0 { return Ok(()); }
        if let Some((index, point, unmodified)) = quiet.undo {
            self.record_insert(index, point, length as i64, unmodified).map_err(Fault::Error)?;
        }
        if !quiet.inhibited { self.mark_deactivated()?; }
        Ok(())
    }

    /// deactivate-mark set by a change whose hooks were not inhibited.
    pub(crate) fn mark_deactivated(&mut self) -> Faulty<()> {
        let deactivate = self.change_slots().deactivate;
        // Already t with no buffer-local binding: nothing changes.
        if self.slots[deactivate].value == Value::TRUE && self.slots[deactivate].local == Value::NIL
            && self.trapped.is_empty() && self.indirect_variable(deactivate) == deactivate {
            return Ok(());
        }
        self.set_variable_value(deactivate, Value::TRUE).map_err(Fault::Error)
    }

    /// Run a storage kernel on ARGS outside the engine's registers, which
    /// point into its own stack: the kernel gets the reused kernel stack.
    /// The bracket made its read-only checks, which GNU does not repeat.
    fn edit_kernel(&mut self, args: &[Value], run: impl FnOnce(&mut Self, usize) -> Result<Value>) -> Faulty<Value> {
        let mut kernel = std::mem::take(&mut self.kernel_stack);
        kernel.clear();
        kernel.extend_from_slice(args);
        let saved = std::mem::replace(&mut self.stack, kernel);
        self.change_prepared = true;
        let result = run(self, args.len());
        self.change_prepared = false;
        self.kernel_stack = std::mem::replace(&mut self.stack, saved);
        result.map_err(Fault::Error)
    }

    /// insert_from_string_1 / insert_1_both for one `insert' argument (a
    /// non-empty string or a character) at point of the current buffer,
    /// with INHERIT (sticky properties) and BEFORE_MARKERS, after its hooks:
    /// the text, then record_insert. Returns the characters inserted.
    pub(crate) fn insert_change(&mut self, arg: Value, inherit: bool, before_markers: bool) -> Faulty<i64> {
        let buffer = self.current_buffer_value()?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        let unmodified = !self.buffer_state(buffer).map_err(Fault::Error)?.modified();
        let point = self.point_char()?;
        let unibyte = matches!(self.objects[index], Object::ByteBuffer(_));
        // A unibyte buffer without properties has nothing to inherit.
        let inherit = inherit && !(unibyte && !self.buffer_has_intervals(buffer));
        match (inherit, before_markers) {
            (false, false) => self.edit_kernel(&[arg], |vm, n| vm.buffer_primitive(Primitive::Insert, 0, n))?,
            (false, true) => self.edit_kernel(&[arg], |vm, n| vm.primitive(Primitive::InsertBeforeMarkers, 0, n))?,
            (true, before) => self.edit_kernel(&[arg], |vm, n| vm.annotation_insert_inherit(0, n, before))?,
        };
        let after = self.point_char()?;
        self.record_insert(index, point, after - point, unmodified).map_err(Fault::Error)?;
        Ok(after - point)
    }

    /// del_range_2 for FROM..TO of the current buffer after its hooks:
    /// record_delete with marker adjustments, then the deletion. Returns the
    /// deleted text (with properties) when EXTRACT or undo needed it, else nil.
    /// FROM outside the accessible region (the hooks narrowed it) is refused
    /// before anything is recorded, as make_buffer_string_both's
    /// next-property-change refuses it in GNU (VM2-EDIT-HOOKS1).
    pub(crate) fn delete_change(&mut self, from: i64, to: i64, extract: bool) -> Faulty<Value> {
        let buffer = self.current_buffer_value()?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        let (_, begv, zv) = self.point_extent()?;
        if from < begv || from > to || to > zv {
            let position = Value::integer(from).map_err(Fault::Error)?;
            return Err(self.text_property_signal("args-out-of-range", &[position, position]));
        }
        let recording = self.undo_list_of(index) != Value::TRUE;
        let text = if extract || recording { self.buffer_text(buffer, from, to)? } else { Value::NIL };
        if recording {
            self.record_delete(index, from, text, true).map_err(Fault::Error)?;
        }
        let (from_v, to_v) = (Value::integer(from).map_err(Fault::Error)?, Value::integer(to).map_err(Fault::Error)?);
        self.edit_kernel(&[from_v, to_v], |vm, n| vm.buffer_primitive(Primitive::DeleteRegion, 0, n))?;
        Ok(text)
    }

    /// replace_range (insdel.c:1463) of FROM..TO of the current buffer by
    /// TEXT after its hooks, inheriting no properties. The old text's
    /// markers go to FROM and those at TO follow the new text, as
    /// adjust_markers_for_replace moves them (an empty FROM..TO is an
    /// insertion); point moves as such a marker would after FROM, and at TO
    /// as for an insertion. Undo gets record_insert and record_delete
    /// without marker adjustments, MODIFF one increment for the whole
    /// change. Returns FROM, TO and the characters inserted as the change
    /// used them, or None when nothing changed.
    pub(crate) fn replace_change(&mut self, from: i64, to: i64, text: Value) -> Faulty<Option<(i64, i64, i64)>> {
        let buffer = self.current_buffer_value()?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        let (point, begv, zv) = self.point_extent()?;
        let from = from.max(begv);
        let to = to.min(zv).max(from);
        let inserted = self.str_view(text).ok_or(Fault::Wrong("stringp", text))?.chars().count() as i64;
        if from == to && inserted == 0 {
            return Ok(None);
        }
        // A unibyte buffer takes the text as bytes, refused before undo
        // records anything.
        let bytes = match self.objects[index] {
            Object::ByteBuffer(_) => Some(self.unibyte_text(text).map_err(Fault::Error)?),
            _ => None,
        };
        if self.undo_list_of(index) != Value::TRUE {
            let unmodified = !self.buffer_state(buffer).map_err(Fault::Error)?.modified();
            let deletion = self.buffer_text(buffer, from, to)?;
            self.record_insert(index, to, inserted, unmodified).map_err(Fault::Error)?;
            self.record_delete(index, from, deletion, false).map_err(Fault::Error)?;
        }
        let modification = *self.buffer_state(buffer).map_err(Fault::Error)?.modification;
        if let Some(bytes) = bytes {
            // ByteBuffer::replace moves the saved positions as
            // adjust_markers_for_replace does.
            let (a, z) = (self.change_byte(buffer, from)?, self.change_byte(buffer, to)?);
            let Object::ByteBuffer(b) = &mut self.objects[index] else { unreachable!("unibyte buffer") };
            b.replace(a..z, &bytes).map_err(Fault::Error)?;
            self.display_state.version += 1;
        } else {
            self.replace_text(buffer, from, to, text, inserted)?;
        }
        let state = self.buffer_state_mut(buffer).map_err(Fault::Error)?;
        *state.modification = modification;
        state.modification.changed((to - from + inserted) as usize, true);
        let point = if from < point || point == to { point + from + inserted - point.min(to) } else { point };
        self.set_change_point(buffer, point)?;
        Ok(Some((from, to, inserted)))
    }

    /// Inserting TEXT at TO before its markers, then deleting FROM..TO,
    /// moves markers and overlay ends as adjust_markers_for_replace does.
    fn replace_text(&mut self, buffer: Value, from: i64, to: i64, text: Value, inserted: i64) -> Faulty<()> {
        self.set_change_point(buffer, to)?;
        if inserted > 0 {
            if from < to {
                self.edit_kernel(&[text], |vm, n| vm.primitive(Primitive::InsertBeforeMarkers, 0, n))?;
            } else {
                self.edit_kernel(&[text], |vm, n| vm.buffer_primitive(Primitive::Insert, 0, n))?;
            }
        }
        if from < to {
            let (from_v, to_v) = (Value::integer(from).map_err(Fault::Error)?, Value::integer(to).map_err(Fault::Error)?);
            self.edit_kernel(&[from_v, to_v], |vm, n| vm.buffer_primitive(Primitive::DeleteRegion, 0, n))?;
        }
        Ok(())
    }

    fn set_change_point(&mut self, buffer: Value, position: i64) -> Faulty<()> {
        let at = self.change_byte(buffer, position)?;
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        match &mut self.objects[index] {
            Object::Buffer(b) => b.point = at,
            Object::ByteBuffer(b) => b.point = at,
            _ => return Err(Fault::Error("buffer required".into())),
        }
        Ok(())
    }

    /// make_buffer_string_both with properties: FROM..TO of BUFFER.
    pub(crate) fn buffer_text(&mut self, buffer: Value, from: i64, to: i64) -> Faulty<Value> {
        let (a, b) = (self.change_byte(buffer, from)?, self.change_byte(buffer, to)?);
        let object = match self.object_if(buffer) {
            Some(Object::Buffer(t)) => data::string_object(t.lisp_string(a..b).map_err(Fault::Error)?),
            Some(Object::ByteBuffer(t)) => Object::Bytes(t.bytes[a.0..b.0].to_vec()),
            _ => return Err(Fault::Error("buffer required".into())),
        };
        Ok(self.allocate_object(object))
    }

    /// The markers of buffer INDEX's text at character positions FROM..=TO,
    /// those of the buffers sharing it too, with their insertion types, the
    /// last attached first as GNU's marker chain lists them.
    pub(crate) fn markers_between(&mut self, index: usize, from: i64, to: i64) -> Result<Vec<(Value, i64, bool)>> {
        let Object::Buffer(b) = &self.objects[index] else { return Ok(Vec::new()) };
        let (a, z) = (b.byte_of_char(CharOffset((from - 1) as usize))?, b.byte_of_char(CharOffset((to - 1) as usize))?);
        if !b.anchors.any_between(a.0 as u32, z.0 as u32) {
            return Ok(Vec::new());
        }
        let family = self.text_family(index);
        let buffers: Vec<Value> = if family.is_empty() { vec![index] } else { family }
            .into_iter().map(|i| Value::reference(i + 1, 0)).collect();
        let mut found = Vec::new();
        self.referrers();
        let Object::Buffer(b) = &self.objects[index] else { unreachable!("buffer above") };
        for &i in self.referrers.as_deref().unwrap_or_default() {
            let Object::Marker(m) = &self.objects[i] else { continue };
            if !buffers.contains(&m.buffer) { continue; }
            let Some(anchor) = m.anchor else { continue };
            let Some(at) = b.anchors.resolve(anchor) else { continue };
            if (a.0 as u32..=z.0 as u32).contains(&at) {
                let position = b.char_of_byte(ByteOffset(at as usize))?.0 as i64 + 1;
                found.push((b.anchors.birth(anchor), Value::reference(i + 1, 0), position, m.bias == anchors::Bias::Right));
            }
        }
        found.sort_unstable_by_key(|&(birth, ..)| std::cmp::Reverse(birth));
        Ok(found.into_iter().map(|(_, marker, position, insertion)| (marker, position, insertion)).collect())
    }
}
