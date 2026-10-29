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

    /// GNU keeps an overlay's plist newest-first (`overlay-put' prepends a
    /// new property); VM2 keeps insertion order, so the copy is reversed.
    pub fn overlay_plist(&mut self, overlay: Value) -> std::result::Result<Value, Fault> {
        let properties = match self.vm.object(overlay) {
            Ok(Object::Overlay(o)) => o.properties.clone(),
            _ => return Err(Fault::Wrong("overlayp", overlay)),
        };
        Ok(properties.into_iter().fold(Value::NIL, |tail, (key, value)| {
            let tail = self.cons(value, tail);
            self.cons(key, tail)
        }))
    }

    /// The current buffer's overlays in GNU's interval-tree order
    /// (buffer.c:4045): by start, and among equal starts newest first.
    pub fn current_overlays(&mut self) -> std::result::Result<Vec<Value>, Fault> {
        let buffer = self.current_buffer()?;
        let mut overlays = match self.vm.object(buffer).map_err(|e| Fault::Error(e.to_string()))? {
            Object::Buffer(b) => b.intervals.query_with_end(0, b.anchors.extent(), false, true, &b.anchors).0,
            _ => return Ok(Vec::new()),
        };
        let mut keyed = Vec::with_capacity(overlays.len());
        for overlay in overlays.drain(..) {
            let (start, _) = self.vm.overlay_range(overlay).map_err(Fault::Error)?.unwrap_or((0, 0));
            keyed.push((start, std::cmp::Reverse(overlay.0), overlay));
        }
        keyed.sort_unstable_by_key(|&(start, newest, _)| (start, newest));
        Ok(keyed.into_iter().map(|(_, _, overlay)| overlay).collect())
    }

    pub fn delete_all_overlays(&mut self, buffer: Value) -> std::result::Result<Value, Fault> {
        let buffer = self.decode_buffer(buffer)?;
        // Overlays are display state no buffer modification tick counts.
        self.vm.display_state.overlays += 1;
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
