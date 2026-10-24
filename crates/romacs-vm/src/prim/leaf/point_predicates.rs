//! Owned scalar snapshot; no buffer borrow escapes this non-collecting leaf.
use super::*;
use crate::coordinates::CharOffset;

impl Leaf<'_> {
    pub fn character_before(&mut self, position: Value) -> std::result::Result<Value, Fault> {
        use crate::coordinates::ByteOffset;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        // GNU marker positions are byte positions in the marker's own buffer,
        // then interpreted against the current buffer's accessible interval.
        let marker_byte = match self.vm.object_if(position) {
            Some(data::Object::Marker(marker)) => {
                let anchor = marker.anchor.ok_or_else(|| Fault::Error("Marker does not point anywhere".into()))?;
                let Some(data::Object::Buffer(buffer)) = self.vm.object_if(marker.buffer) else {
                    return Err(Fault::Error("Marker does not point anywhere".into()));
                };
                Some(buffer.anchors.resolve(anchor).ok_or_else(|| Fault::Error("Marker does not point anywhere".into()))? as usize)
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
                let bytes = buffer.bytes();
                let mut previous = at - 1;
                while previous > 0 && bytes[previous] & 0xc0 == 0x80 { previous -= 1; }
                buffer.character_at(ByteOffset(previous)).map_err(Fault::Error)?
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
                    buffer.character_of_char(CharOffset(character.0 - 1)).map_err(Fault::Error)? == Some('\n')
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
