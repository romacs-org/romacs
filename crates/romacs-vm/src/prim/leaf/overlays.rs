//! Buffer-owned overlay operations; no evaluation or collection at this boundary.
use super::*;
use crate::coordinates::ByteOffset;
use crate::data::Object;

impl Leaf<'_> {
    pub fn overlay_buffer(&self, overlay: Value) -> std::result::Result<Value, Fault> {
        match self.vm.object(overlay) {
            Ok(Object::Overlay(o)) => Ok(o.buffer),
            _ => Err(Fault::Wrong("overlayp", overlay)),
        }
    }

    pub fn delete_all_overlays(&mut self, buffer: Value) -> std::result::Result<Value, Fault> {
        let buffer = self.decode_buffer(buffer)?;
        let overlays = match self
            .vm
            .object(buffer)
            .map_err(|e| Fault::Error(e.to_string()))?
        {
            Object::Buffer(b) => {
                b.intervals
                    .query_with_end(0, b.anchors.extent(), false, true, &b.anchors)
                    .0
            }
            _ => return Ok(Value::NIL),
        };
        // Deletion changes anchors, not the object table, and retains the plist.
        // No collection occurs while these buffer-owned identities are borrowed.
        for overlay in overlays {
            self.vm
                .delete_overlay_object(overlay)
                .map_err(|e| Fault::Error(e.to_string()))?;
        }
        Ok(Value::NIL)
    }

    pub fn previous_overlay_change(
        &mut self,
        position: Value,
    ) -> std::result::Result<Value, Fault> {
        let position = if let Some(n) = position.integer_value() {
            n
        } else if let Some(Object::Bignum(n)) = self.vm.object_if(position) {
            if n.sign() == num_bigint::Sign::Minus {
                i64::MIN
            } else {
                i64::MAX
            }
        } else {
            match self.marker_position(position) {
                Some(Some(n)) => n,
                Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
                None => return Err(Fault::Wrong("integer-or-marker-p", position)),
            }
        };
        let buffer = self.decode_buffer(Value::NIL)?;
        let b = match self
            .vm
            .object(buffer)
            .map_err(|e| Fault::Error(e.to_string()))?
        {
            Object::Buffer(b) => b,
            Object::ByteBuffer(b) => {
                return Value::integer(b.restriction.start.0 as i64 + 1)
                    .map_err(|e| Fault::Error(e.to_string()));
            }
            _ => unreachable!("decode_buffer validated the buffer"),
        };
        // GNU bounds the search below by BEGV, not above by ZV. POS can
        // lie outside the buffer, so clamp only for character-to-byte conversion.
        let cutoff = if position <= 1 {
            0
        } else if position > b.len_chars() as i64 + 1 {
            b.anchors.extent() as u64 + 1
        } else {
            b.byte_of_char(crate::coordinates::CharOffset((position - 1) as usize))
                .map_err(|e| Fault::Error(e.to_string()))?
                .0 as u64
        };
        let mut previous = b.accessible().start.0 as u32;
        b.intervals
            .visit(0, b.anchors.extent(), false, &b.anchors, |_, start, end| {
                for endpoint in [start, end] {
                    if (endpoint as u64) < cutoff {
                        previous = previous.max(endpoint);
                    }
                }
            });
        let previous = b
            .char_of_byte(ByteOffset(previous as usize))
            .map_err(|e| Fault::Error(e.to_string()))?
            .0 as i64
            + 1;
        Value::integer(previous).map_err(|e| Fault::Error(e.to_string()))
    }
}
