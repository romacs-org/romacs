//! Owned scalar snapshot; no buffer borrow escapes this non-collecting leaf.
use super::*;
use crate::coordinates::CharOffset;

impl Leaf<'_> {
    pub fn following_character(&mut self) -> std::result::Result<Value, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let code = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                if buffer.point >= buffer.accessible().end { 0 }
                else { buffer.character_at(buffer.point).map_err(Fault::Error)?.unwrap_or(0) }
            }
            data::Object::ByteBuffer(buffer) => {
                if buffer.point >= buffer.restriction.end { 0 }
                else { i64::from(buffer.bytes[buffer.point.0]) }
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        Value::integer(code).map_err(Fault::Error)
    }

    pub fn character_before(&mut self, position: Value) -> std::result::Result<Value, Fault> {
        use crate::coordinates::ByteOffset;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        // GNU marker positions are byte positions in the marker's own buffer,
        // then interpreted against the current buffer's accessible interval.
        let marker_byte = match self.vm.object_if(position) {
            Some(data::Object::Marker(marker)) => {
                let anchor = marker.anchor.ok_or_else(|| Fault::Error("Marker does not point anywhere".into()))?;
                let buffer = self.vm.marker_anchors(marker.buffer).map_err(|_| Fault::Error("Marker does not point anywhere".into()))?;
                Some(buffer.resolve(anchor).ok_or_else(|| Fault::Error("Marker does not point anywhere".into()))? as usize)
            }
            _ => None,
        };
        let scalar = if position == Value::NIL || marker_byte.is_some() { None } else {
            let n = self.bigint_value(position).ok_or(Fault::Wrong("integer-or-marker-p", position))?;
            Some(i64::try_from(&n).unwrap_or(if n.sign() == num_bigint::Sign::Minus { i64::MIN } else { i64::MAX }))
        };
        let result = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                let accessible = buffer.accessible();
                let at = if let Some(p) = scalar {
                    let beginning = buffer.char_of_byte(accessible.start).map_err(Fault::Error)?.0 as i64 + 1;
                    let end = buffer.char_of_byte(accessible.end).map_err(Fault::Error)?.0 as i64 + 1;
                    if p <= beginning || p > end { return Ok(Value::NIL); }
                    buffer.byte_of_char(CharOffset((p - 1) as usize)).map_err(Fault::Error)?.0
                } else { marker_byte.unwrap_or(buffer.point.0) };
                if at <= accessible.start.0 || at > accessible.end.0 { return Ok(Value::NIL); }
                buffer.character_at(ByteOffset(buffer.char_start_before(at))).map_err(Fault::Error)?
            }
            data::Object::ByteBuffer(buffer) => {
                let at = if let Some(p) = scalar {
                    if p <= buffer.restriction.start.0 as i64 + 1 || p > buffer.restriction.end.0 as i64 + 1 { return Ok(Value::NIL); }
                    (p - 1) as usize
                } else { marker_byte.unwrap_or(buffer.point.0) };
                if at <= buffer.restriction.start.0 || at > buffer.restriction.end.0 { return Ok(Value::NIL); }
                Some(buffer.bytes[at - 1] as i64)
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        result.map_or(Ok(Value::NIL), |c| Value::integer(c).map_err(Fault::Error))
    }

    pub fn point_boundaries(&mut self) -> std::result::Result<(bool, bool, bool, bool), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let (point, beginning, end, previous_newline, next_newline) = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                let accessible = buffer.accessible();
                let previous = if buffer.point > accessible.start {
                    let character = buffer.char_of_byte(buffer.point).map_err(Fault::Error)?;
                    buffer.character_of_char(CharOffset(character.0 - 1)).map_err(Fault::Error)? == Some(u32::from(b'\n'))
                } else { false };
                let next = buffer.point < accessible.end
                    && buffer.character_at(buffer.point).map_err(Fault::Error)? == Some(10);
                (buffer.point.0, accessible.start.0, accessible.end.0, previous, next)
            }
            data::Object::ByteBuffer(buffer) => {
                let point = buffer.point.0;
                let beginning = buffer.restriction.start.0;
                let end = buffer.restriction.end.0;
                (point, beginning, end,
                    point > beginning && buffer.bytes.get(point - 1) == Some(&b'\n'),
                    point < end && buffer.bytes.get(point) == Some(&b'\n'))
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        // GNU tests adjacent bytes for ASCII newline. The equivalent decoded
        // character lookup uses rope chunks without materializing the entire
        // buffer; never treat an Elisp character offset as a byte offset.
        Ok((point == beginning, point == end, previous_newline, next_newline))
    }
}

impl Leaf<'_> {
    /// fns.c Fline_number_at_pos: one more than the newlines from the
    /// accessible start (the buffer start when ABSOLUTE) to POSITION.
    pub fn line_number_at_pos(&mut self, position: Value, absolute: Value) -> std::result::Result<Value, Fault> {
        // display_count_lines: with `selective-display' t, a CR ends a line too.
        let name = self.intern("selective-display");
        let selective = self.variable_value(name)?.is_some_and(|v| v != Value::NIL && v.integer_value().is_none());
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let size = match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.len_chars(),
            data::Object::ByteBuffer(b) => b.bytes.len(),
            _ => unreachable!("buffer_index validates"),
        } as i64;
        let mut pos = match self.marker_position(position) {
            Some(Some(n)) => n,
            Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
            None if position == Value::NIL => self.point()?,
            None => {
                let pos = position.integer_value().ok_or(Fault::Wrong("fixnump", position))?;
                if pos < 1 || pos > size + 1 {
                    return Err(Fault::Range3(position, Value::integer(1).unwrap(), Value::integer(size + 1).map_err(Fault::Error)?));
                }
                pos
            }
        };
        let (low, high): (i64, i64) = match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let r = b.accessible();
                (b.char_of_byte(r.start).map_err(Fault::Error)?.0 as i64 + 1, b.char_of_byte(r.end).map_err(Fault::Error)?.0 as i64 + 1)
            }
            data::Object::ByteBuffer(b) => (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
            _ => unreachable!("buffer_index validates"),
        };
        let start = if absolute == Value::NIL { pos = pos.clamp(low, high); low } else { 1 };
        let byte = |c: i64| match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.byte_of_char(CharOffset((c - 1) as usize)).map(|b| b.0).map_err(Fault::Error),
            _ => Ok((c - 1) as usize),
        };
        let (from, to) = (byte(start)?, byte(pos.max(start))?);
        let breaks = |bytes: &[u8]| bytes.iter().filter(|&&c| c == b'\n' || (selective && c == b'\r')).count();
        let lines = match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.chunks_in(from..to).map(breaks).sum::<usize>(),
            data::Object::ByteBuffer(b) => breaks(&b.bytes[from..to]),
            _ => unreachable!("buffer_index validates"),
        } as i64 + 1;
        Value::integer(lines).map_err(Fault::Error)
    }
}
