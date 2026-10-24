//! Native access to the same annotation substrate as get-char-property.
use super::*;

impl Leaf<'_> {
    pub fn native_current_column(&mut self) -> super::super::Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let prefix = match &self.vm.objects[index] {
            crate::data::Object::Buffer(b) => b.as_str()[..b.point.0].to_owned(),
            crate::data::Object::ByteBuffer(b) => std::str::from_utf8(&b.bytes[..b.point.0])
                .map_err(|_| Fault::Error("VM2 current-column cannot display non-UTF-8 bytes".into()))?.to_owned(),
            _ => unreachable!(),
        };
        let variable = self.intern("tab-width");
        let tab = self.variable_value(variable)?.and_then(|v| v.integer_value())
            .filter(|n| (1..=1000).contains(n)).unwrap_or(8) as usize;
        let line = prefix.rsplit('\n').next().unwrap_or("");
        let mut column = 0usize;
        let mut start = 0;
        for (at, c) in line.char_indices() {
            if c == '\t' || c.is_control() {
                column += unicode_width::UnicodeWidthStr::width(&line[start..at]);
                column += if c == '\t' { tab - column % tab } else { 2 };
                start = at + c.len_utf8();
            }
        }
        column += unicode_width::UnicodeWidthStr::width(&line[start..]);
        Value::integer(column as i64).map_err(Fault::Error)
    }

    pub fn fieldless_constraint(&mut self, new: Value, old: Value) -> super::super::Out {
        let position = |leaf: &Self, value: Value| match leaf.marker_position(value) {
            Some(Some(n)) => Ok(n),
            Some(None) => Ok(0),
            None => value.integer_value().ok_or(Fault::Wrong("integer-or-marker-p", value)),
        };
        let new = if new == Value::NIL { self.point()? } else { position(self, new)? };
        let old = position(self, old)?;
        let result = Value::integer(new).map_err(Fault::Error)?;
        let inhibit = self.intern("inhibit-field-text-motion");
        if new == old || self.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL) {
            return Ok(result);
        }
        let shape = self.buffer_shape()?;
        let field = self.intern("field");
        for pos in [Some(new), Some(old), (new > shape.begv).then(|| new - 1), (old > shape.begv).then(|| old - 1)].into_iter().flatten() {
            let position = Value::integer(pos).map_err(Fault::Error)?;
            if pos < 1 || pos > shape.z { return Err(Fault::Range1(position)); }
            let value = self.character_property(position, field)?;
            if value != Value::NIL {
                return Err(Fault::Error("VM2 field-boundary motion is not implemented".into()));
            }
        }
        Ok(result)
    }

    fn character_property(&mut self, pos: Value, key: Value) -> super::super::Out {
        // Threaded activations exclude the final 4096 stack slots. This
        // non-evaluating property query uses two of them without reallocating
        // the Vec and invalidating the engine's live register pointers.
        let start = self.vm.stack.len().checked_sub(2)
            .ok_or_else(|| Fault::Error("native property scratch unavailable".into()))?;
        let saved = [self.vm.stack[start], self.vm.stack[start + 1]];
        self.vm.stack[start..].copy_from_slice(&[pos, key]);
        let value = self.vm.annotation_primitive(Primitive::GetCharProperty, start, 2);
        self.vm.stack[start..].copy_from_slice(&saved);
        value.map_err(Fault::Error)
    }

    pub fn invisible_property(&mut self, pos: Value) -> super::super::Out {
        // GNU deliberately treats negative fixnums, floats and bignums as
        // property values, not positions (FIXNATP, rather than INTEGERP).
        let marker = self.marker_position(pos);
        if !pos.integer_value().is_some_and(|n| n >= 0) && marker.is_none() {
            return Ok(pos);
        }
        let n = match marker {
            Some(Some(n)) => n,
            Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
            None => pos.integer_value().unwrap(),
        };
        let shape = self.buffer_shape()?;
        if n < shape.begv || n > shape.zv {
            return Err(Fault::Range1(Value::integer(n).map_err(Fault::Error)?));
        }
        let key = self.intern("invisible");
        self.character_property(Value::integer(n).map_err(Fault::Error)?, key)
    }
}
