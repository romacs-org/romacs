//! editfns.c fields: find_field over the `field' char-property, and
//! constrain-to-field, which the field primitives and the field-respecting
//! line and word motion share.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

impl Leaf<'_> {
    /// CHECK_FIXNUM_COERCE_MARKER.
    fn field_position(&self, value: Value) -> std::result::Result<i64, Fault> {
        match self.marker_position(value) {
            Some(Some(n)) => Ok(n),
            Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
            None => value.integer_value().ok_or(Fault::Wrong("integer-or-marker-p", value)),
        }
    }

    /// POSITION's PROPERTY as Fget_char_property sees it in the current buffer.
    fn char_property_here(&mut self, position: i64, property: Value) -> super::super::Out {
        self.vm.char_property_accessible(position, property)
    }

    /// editfns.c find_field: the start and end of the field at POS (point
    /// when nil), each computed only if asked for. A position between two
    /// fields belongs to the one an inserted character would join (its
    /// stickiness); with MERGE, to both, and a `boundary' field between
    /// two others joins them. The limits bound the scans, not that choice.
    pub(crate) fn find_field(
        &mut self,
        pos: Value,
        merge: bool,
        beg_limit: Value,
        end_limit: Value,
        (want_beg, want_end): (bool, bool),
    ) -> std::result::Result<(i64, i64), Fault> {
        let pos = if pos == Value::NIL { self.point()? } else { self.field_position(pos)? };
        let shape = self.buffer_shape()?;
        let key = self.intern("field");
        let after = self.char_property_here(pos, key)?;
        // Not nil: a buffer may start with a non-sticky field.
        let before = if pos > shape.begv { self.char_property_here(pos - 1, key)? } else { after };
        let (mut at_start, mut at_end) = (false, false);
        if !merge {
            let at = Value::integer(pos).map_err(Fault::Error)?;
            let field = self.get_pos_property(at, key, Value::NIL)?;
            at_end = field != after;
            at_start = field != before;
            // A nil field between non-nil ones is not an empty field but
            // an uneditable one, such as a prompt.
            if field == Value::NIL && at_start && at_end {
                (at_start, at_end) = (false, false);
            }
        }
        let boundary = self.intern("boundary");
        let at = Value::integer(pos).map_err(Fault::Error)?;
        let beg = if !want_beg || at_start { pos } else {
            let mut p = at;
            if merge && before == boundary {
                p = self.previous_single_char_property_change(p, key, Value::NIL, beg_limit)?;
            }
            p = self.previous_single_char_property_change(p, key, Value::NIL, beg_limit)?;
            p.integer_value().unwrap_or(shape.begv)
        };
        let end = if !want_end || at_end { pos } else {
            let mut p = at;
            if merge && after == boundary {
                p = self.next_single_char_property_change(p, key, Value::NIL, end_limit)?;
            }
            p = self.next_single_char_property_change(p, key, Value::NIL, end_limit)?;
            p.integer_value().unwrap_or(shape.zv)
        };
        Ok((beg, end))
    }

    /// editfns.c Fconstrain_to_field: NEW (point when nil, which then
    /// moves) pulled back to the field of OLD.
    pub(crate) fn constrain_to_field(
        &mut self,
        new: Value,
        old: Value,
        escape: Value,
        only_in_line: Value,
        capture: Value,
    ) -> super::super::Out {
        let set_point = new == Value::NIL;
        let new = if set_point { self.point()? } else { self.field_position(new)? };
        let old = self.field_position(old)?;
        let constrained = self.field_constraint(new, old, escape, only_in_line, capture)?;
        if set_point && constrained != new {
            self.goto_char_position(constrained)?;
        }
        Value::integer(constrained).map_err(Fault::Error)
    }

    /// editfns.c Fline_beginning_position and Fline_end_position: the
    /// line boundary N lines on, constrained to point's field unless that
    /// would leave the boundary's line. A beginning N lines away escapes
    /// from a field edge at point.
    pub(crate) fn line_field_position(&mut self, n: Value, end: bool) -> super::super::Out {
        // No field at either end, the common case, leaves the boundary
        // as it is, found and checked in bytes.
        let byte = self.line_boundary_byte(n, end)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if let data::Object::Buffer(b) = &self.vm.objects[index] {
            let (point, buffer) = (b.point, self.vm.slots[self.vm.buffer_slot].value);
            // A read that signals takes the general path, which first
            // checks what would make it unnecessary.
            if let Ok(false) = self.field_near_bytes(buffer, ByteOffset(byte), point) {
                return self.position_of_byte(byte);
            }
        }
        let bound = self.position_of_byte(byte)?;
        let point = Value::integer(self.point()?).map_err(Fault::Error)?;
        let escape = !end && n != Value::NIL && n.integer_value() != Some(1);
        self.constrain_to_field(bound, point, if escape { Value::TRUE } else { Value::NIL }, Value::TRUE, Value::NIL)
    }

    fn field_constraint(&mut self, new: i64, old: i64, escape: Value, only_in_line: Value, capture: Value) -> std::result::Result<i64, Fault> {
        let inhibit = self.intern("inhibit-field-text-motion");
        if new == old || self.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL) {
            return Ok(new);
        }
        let shape = self.buffer_shape()?;
        let begv = shape.begv;
        if !self.field_near(new, old, (begv, shape.zv))? || (capture != Value::NIL && self.captured(old, capture, begv)?) {
            return Ok(new);
        }
        let forward = new > old;
        let (old_at, new_at) = (Value::integer(old).map_err(Fault::Error)?, Value::integer(new).map_err(Fault::Error)?);
        let escape = escape != Value::NIL;
        let bound = if forward {
            self.find_field(old_at, escape, Value::NIL, new_at, (false, true))?.1
        } else {
            self.find_field(old_at, escape, new_at, Value::NIL, (true, false))?.0
        };
        // ESCAPE can put BOUND beyond NEW, which is then already in OLD's field.
        let crosses = if bound < new { forward } else { !forward };
        Ok(if crosses && (only_in_line == Value::NIL || !self.newline_between(new, bound)?) { bound } else { new })
    }

    /// Whether a `field' char-property is non-nil at character position NEW
    /// or OLD, or at the character before either: a non-sticky field such
    /// as a prompt ends there with no `field' at the position itself.
    /// A position outside BEGV..ZV signals as `get-char-property' does.
    fn field_near(&mut self, new: i64, old: i64, (begv, zv): (i64, i64)) -> std::result::Result<bool, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let accessible = (begv..=zv).contains(&new) && (begv..=zv).contains(&old);
        if let (true, data::Object::Buffer(b)) = (accessible, &self.vm.objects[index]) {
            let byte = |n: i64| b.byte_of_char(CharOffset((n - 1) as usize)).map_err(Fault::Error);
            let (new, old, buffer) = (byte(new)?, byte(old)?, self.vm.slots[self.vm.buffer_slot].value);
            return self.field_near_bytes(buffer, new, old);
        }
        let key = self.intern("field");
        Ok(self.char_property_here(new, key)? != Value::NIL
            || self.char_property_here(old, key)? != Value::NIL
            || (new > begv && self.char_property_here(new - 1, key)? != Value::NIL)
            || (old > begv && self.char_property_here(old - 1, key)? != Value::NIL))
    }

    /// `field_near' at bytes NEW and OLD of BUFFER, the current buffer and
    /// a multibyte one, each in its accessible region: read where they are,
    /// converting no position.
    fn field_near_bytes(&mut self, buffer: Value, new: ByteOffset, old: ByteOffset) -> std::result::Result<bool, Fault> {
        let key = self.intern("field");
        let data::Object::Buffer(b) = self.vm.object(buffer).map_err(Fault::Error)? else { unreachable!("a multibyte buffer") };
        let begv = b.accessible().start;
        // Usually nothing near them could give a field: then none is read.
        // A character is at most 4 bytes, so the span holds the ones before.
        let low = new.min(old).0.saturating_sub(4).max(begv.0);
        if self.vm.char_property_absent(buffer, key, low..new.0.max(old.0) + 1) {
            return Ok(false);
        }
        let before = |at: ByteOffset| (at > begv).then(|| ByteOffset(b.body().char_start_before(at.0)));
        let spots = [Some(new), Some(old), before(new), before(old)];
        for at in spots.into_iter().flatten() {
            if self.vm.char_property_at_byte(buffer, at.0, key, None).map_err(Fault::Error)?.0 != Value::NIL {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether OLD's CAPTURE property says to ignore field boundaries: it
    /// would be inherited at OLD, or the characters on both sides have it.
    fn captured(&mut self, old: i64, capture: Value, begv: i64) -> std::result::Result<bool, Fault> {
        let at = Value::integer(old).map_err(Fault::Error)?;
        Ok(self.get_pos_property(at, capture, Value::NIL)? != Value::NIL
            || (old > begv
                && self.char_property_here(old, capture)? != Value::NIL
                && self.char_property_here(old - 1, capture)? != Value::NIL))
    }

    /// Whether a newline lies between character positions A and B.
    fn newline_between(&mut self, a: i64, b: i64) -> std::result::Result<bool, Fault> {
        let (low, high) = (a.min(b) as usize - 1, a.max(b) as usize - 1);
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let byte = |n: usize| b.byte_of_char(crate::coordinates::CharOffset(n)).map_err(Fault::Error);
                b.as_str().as_bytes()[byte(low)?.0..byte(high)?.0].contains(&b'\n')
            }
            data::Object::ByteBuffer(b) => b.bytes[low..high].contains(&b'\n'),
            _ => return Err(Fault::Error("buffer required".into())),
        })
    }
}
