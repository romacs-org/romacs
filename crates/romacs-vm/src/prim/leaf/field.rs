//! editfns.c fields: find_field over the `field' char-property, and
//! constrain-to-field, which the field primitives and the field-respecting
//! line and word motion share.
use super::*;

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
        let bound = self.line_boundary_position(n, end)?;
        let point = Value::integer(self.point()?).map_err(Fault::Error)?;
        let escape = !end && n != Value::NIL && n.integer_value() != Some(1);
        self.constrain_to_field(bound, point, if escape { Value::TRUE } else { Value::NIL }, Value::TRUE, Value::NIL)
    }

    fn field_constraint(&mut self, new: i64, old: i64, escape: Value, only_in_line: Value, capture: Value) -> std::result::Result<i64, Fault> {
        let inhibit = self.intern("inhibit-field-text-motion");
        if new == old || self.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL) {
            return Ok(new);
        }
        let begv = self.buffer_shape()?.begv;
        let key = self.intern("field");
        // The positions before NEW and OLD too: a non-sticky field such as
        // a prompt ends there with no `field' at the position itself.
        let near_field = self.char_property_here(new, key)? != Value::NIL
            || self.char_property_here(old, key)? != Value::NIL
            || (new > begv && self.char_property_here(new - 1, key)? != Value::NIL)
            || (old > begv && self.char_property_here(old - 1, key)? != Value::NIL);
        if !near_field || (capture != Value::NIL && self.captured(old, capture, begv)?) {
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
