//! The leaf side of insdel.c's change bracket (crate::insdel) for the
//! insertion and deletion state machines in prims/insdel.rs.
use super::*;
pub(crate) use crate::insdel::Prepared;

impl Leaf<'_> {
    pub(crate) fn prepare_change(&mut self, start: i64, end: i64) -> std::result::Result<Prepared, Fault> {
        self.vm.prepare_change(start, end)
    }
    pub(crate) fn point_char(&mut self) -> std::result::Result<i64, Fault> {
        self.vm.point_char()
    }
    /// Point, BEGV and ZV of the current buffer.
    pub(crate) fn point_extent(&mut self) -> std::result::Result<(i64, i64, i64), Fault> {
        self.vm.point_extent()
    }
    pub(crate) fn after_change_runs(&mut self, overlays: bool, insert_hooks: bool) -> std::result::Result<bool, Fault> {
        self.vm.after_change_runs(overlays, insert_hooks)
    }
    /// deactivate-mark is set once a change's before hooks ran, unless
    /// INHIBITED (modification hooks were inhibited when it was prepared).
    pub(crate) fn change_deactivates_mark(&mut self, inhibited: bool) -> std::result::Result<(), Fault> {
        if inhibited { Ok(()) } else { self.vm.mark_deactivated() }
    }
    pub(crate) fn insert_change(&mut self, arg: Value, inherit: bool, before_markers: bool) -> std::result::Result<i64, Fault> {
        self.vm.insert_change(arg, inherit, before_markers)
    }
    pub(crate) fn delete_change(&mut self, from: i64, to: i64, extract: bool) -> std::result::Result<Value, Fault> {
        self.vm.delete_change(from, to, extract)
    }
    /// The overlay hooks a change of START..END of the current buffer
    /// meets now (report_overlay_modification's scan).
    pub(crate) fn overlay_change_hooks(&mut self, start: i64, end: i64) -> std::result::Result<Vec<(Value, Value)>, Fault> {
        self.vm.overlay_change_hooks_at(start, end)
    }
    /// Point a marker made by `watch_marker' nowhere.
    pub(crate) fn release_watch(&mut self, marker: Value) -> std::result::Result<(), Fault> {
        self.vm.release_marker(marker).map(|_| ())
    }
    /// The character count of a string, or None for another object.
    pub(crate) fn string_chars(&self, v: Value) -> Option<usize> {
        self.vm.str_view(v).map(|view| view.len())
    }
    /// validate_region: START and END as ordered positions of the
    /// accessible region (fix_position), else args-out-of-range.
    pub(crate) fn validate_region(&mut self, start: Value, end: Value) -> std::result::Result<(i64, i64), Fault> {
        let fix = |cx: &Self, v: Value| -> std::result::Result<i64, Fault> {
            if let Some(n) = v.integer_value() { return Ok(n); }
            match cx.marker_position(v) {
                Some(Some(n)) => return Ok(n),
                Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
                None => {}
            }
            match cx.bigint_value(v) {
                Some(n) if n.sign() == num_bigint::Sign::Minus => Ok(Value::MIN_FIXNUM),
                Some(_) => Ok(Value::MAX_FIXNUM),
                None => Err(Fault::Wrong("integer-or-marker-p", v)),
            }
        };
        let (a, b) = (fix(self, start)?, fix(self, end)?);
        let (a, b) = (a.min(b), a.max(b));
        let (_, begv, zv) = self.point_extent()?;
        if a < begv || b > zv {
            let buffer = self.current_buffer()?;
            let data = self.cons(end, Value::NIL);
            let data = self.cons(start, data);
            let data = self.cons(buffer, data);
            return Err(Fault::Signal(self.intern("args-out-of-range"), data));
        }
        Ok((a, b))
    }
    /// FROM..TO of the current buffer as a string, with its properties.
    pub(crate) fn region_text(&mut self, from: i64, to: i64) -> std::result::Result<Value, Fault> {
        let buffer = self.current_buffer()?;
        self.vm.buffer_text(buffer, from, to)
    }
    /// FROM..TO of BUFFER as a string, with its properties.
    pub(crate) fn buffer_region_text(&mut self, buffer: Value, from: i64, to: i64) -> std::result::Result<Value, Fault> {
        self.vm.buffer_text(buffer, from, to)
    }
    pub(crate) fn replace_change(&mut self, from: i64, to: i64, text: Value) -> std::result::Result<Option<(i64, i64, i64)>, Fault> {
        self.vm.replace_change(from, to, text)
    }
    pub(crate) fn modify_text(&mut self, from: i64, to: i64) -> std::result::Result<(), Fault> {
        self.vm.modify_text(from, to)
    }
    pub(crate) fn update_search_regs(&mut self, oldstart: i64, oldend: i64, newend: i64) {
        self.vm.regexp_update_search_regs(oldstart, oldend, newend)
    }
    /// Fwiden for erase-buffer.
    pub(crate) fn widen_current(&mut self) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => b.widen(),
            data::Object::ByteBuffer(b) => b.restriction = crate::coordinates::ByteOffset(0)..crate::coordinates::ByteOffset(b.bytes.len()),
            _ => return Err(Fault::Error("buffer required".into())),
        }
        self.vm.display_state.version += 1;
        Ok(())
    }
}
