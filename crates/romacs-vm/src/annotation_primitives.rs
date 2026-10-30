//! Lisp marker/overlay identity and buffer-coordinate adapters.
#[cfg(test)]
#[path = "annotation_tests.rs"]
mod tests;
use crate::{
    anchors::Bias,
    annotations::{Marker, Overlay},
    coordinates::{ByteOffset, CharOffset},
    data::Object,
    *,
};
impl Vm {
    pub(crate) fn keymap_at_point(&mut self, property: Value) -> Result<Value> {
        let index = self.buffer_index()?;
        let buffer = self.slots[self.buffer_slot].value;
        let (at, left, right, same, local, overlays) = match &self.objects[index] {
            Object::Buffer(text) => {
                let at = text.point;
                let raw = |at| text.properties.properties_at(at).into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
                (at.0,
                 if at.0 > 0 { raw(ByteOffset(at.0 - 1)) } else { vec![] },
                 if at.0 < text.len_bytes() { raw(at) } else { vec![] },
                 at.0 > 0 && at.0 < text.len_bytes() && text.properties.same_run_state(ByteOffset(at.0 - 1), at), text.local_keymap,
                 text.intervals.query_with_end(at.0 as u32, at.0 as u32, true,
                     at.0 == text.len_bytes(), &text.anchors).0)
            }
            Object::ByteBuffer(_) => return Err("keymap lookup in unibyte buffers is not implemented".into()),
            _ => unreachable!(),
        };
        let mut value = self.ordered_overlay_property(overlays, property)?;
        if value == Value::NIL {
            value = self.annotation_resolve(&right, property)?;
        }
        if value == Value::NIL {
            value = self.insertion_overlay_property(buffer, at, property)?;
        }
        if value == Value::NIL {
            let inherited = self.annotation_inherited(left, right, same)?;
            value = self.annotation_resolve(&inherited, property)?;
        }
        Ok(self.resolve_keymap(value).unwrap_or_else(|| {
            if self.names.get("local-map").is_some_and(|&slot| property == Value::reference(slot + 1, 7)) { local } else { Value::NIL }
        }))
    }

    fn insertion_overlay_property(&self, buffer: Value, at: usize, property: Value) -> Result<Value> {
        let Object::Buffer(text) = self.object(buffer)? else { return Ok(Value::NIL); };
        let mut candidates = text.intervals.query_with_end(at.saturating_sub(1) as u32,
            (at + 1).min(text.len_bytes()) as u32, false, true, &text.anchors).0;
        candidates.retain(|&overlay| {
            let object = self.overlay(overlay).unwrap();
            let (start, end) = self.overlay_range(overlay).unwrap().unwrap();
            start as usize <= at && at <= end as usize
                && !(start as usize == at && object.front == Bias::Right)
                && !(end as usize == at && object.rear != Bias::Right)
        });
        self.ordered_overlay_property(candidates, property)
    }

    fn ordered_overlay_property(&self, overlays: Vec<Value>, property: Value) -> Result<Value> {
        Ok(self.ordered_overlay_hit(overlays, property)?.0)
    }

    /// The highest-priority overlay among OVERLAYS with a non-nil PROPERTY:
    /// (VALUE, OVERLAY), or (nil, nil) when none has it.
    fn ordered_overlay_hit(&self, mut overlays: Vec<Value>, property: Value) -> Result<(Value, Value)> {
        overlays.sort_by_key(|&overlay| self.overlay_order_key(overlay));
        for overlay in overlays.into_iter().rev() {
            let value = self.overlay_get(overlay, property)?;
            if value != Value::NIL { return Ok((value, overlay)); }
        }
        Ok((Value::NIL, Value::NIL))
    }

    fn overlay_order_key(&self, overlay: Value) -> (i64, u32, std::cmp::Reverse<u32>, usize) {
        let priority = self.overlay_property_named(overlay, "priority").unwrap().integer_value().unwrap_or(0);
        let (start, end) = self.overlay_range(overlay).unwrap().unwrap();
        (priority, start, std::cmp::Reverse(end), overlay.index(0).unwrap())
    }

    /// Visit covering overlays without allocating a result list or Lisp conses.
    /// Positions and emitted ranges are zero-based UTF-8 byte boundaries.
    /// Identities must be rooted/re-written across collection like other Values.
    pub(crate) fn visit_overlays_at(
        &self,
        buffer: Value,
        position: ByteOffset,
        visitor: impl FnMut(Value, std::ops::Range<ByteOffset>),
    ) -> Result<()> {
        self.visit_overlay_bytes(buffer, position..position, true, visitor)
    }
    /// Visit intersecting overlays, including empty overlays in the half-open range.
    /// Empty overlays at buffer end are included when the query ends there.
    /// An empty query excludes non-empty overlays starting at that boundary.
    /// Traversal order is unspecified; the visitor cannot mutate or collect this VM.
    pub(crate) fn visit_overlays_in(
        &self,
        buffer: Value,
        range: std::ops::Range<ByteOffset>,
        visitor: impl FnMut(Value, std::ops::Range<ByteOffset>),
    ) -> Result<()> {
        self.visit_overlay_bytes(buffer, range, false, visitor)
    }
    fn visit_overlay_bytes(
        &self,
        buffer: Value,
        range: std::ops::Range<ByteOffset>,
        point: bool,
        mut visitor: impl FnMut(Value, std::ops::Range<ByteOffset>),
    ) -> Result<()> {
        let b = self.annotated(buffer)?;
        if range.start > range.end {
            return Err("invalid range".into());
        }
        b.char_of_byte(range.start)?;
        b.char_of_byte(range.end)?;
        b.intervals.visit(
            range.start.0 as u32,
            range.end.0 as u32,
            point,
            &b.anchors,
            |value, start, end| {
                visitor(value, ByteOffset(start as usize)..ByteOffset(end as usize))
            },
        );
        Ok(())
    }
    /// Next overlay endpoint strictly after a checked UTF-8 byte boundary, or buffer end.
    /// This uses start- and end-ordered indexes, not an overlapping-identity scan.
    pub(crate) fn next_overlay_change_byte(
        &self,
        buffer: Value,
        position: ByteOffset,
    ) -> Result<ByteOffset> {
        let b = self.annotated(buffer)?;
        b.char_of_byte(position)?;
        Ok(ByteOffset(
            b.intervals
                .next(position.0 as u32, b.len_bytes() as u32, &b.anchors) as usize,
        ))
    }
    /// Read one overlay property by symbol name without Lisp evaluation or interning.
    /// An absent property returns nil; the returned Value follows normal GC rooting rules.
    pub(crate) fn overlay_property_named(&self, overlay: Value, key: &str) -> Result<Value> {
        let o = self.overlay(overlay)?;
        let Some(&slot) = self.names.get(key) else {
            return Ok(Value::NIL);
        };
        let key = Value::reference(slot + 1, 7);
        Ok(o.properties
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(Value::NIL, |(_, v)| *v))
    }
    fn annotated(&self, buffer: Value) -> Result<&annotations::AnnotatedBuffer> {
        match self.object(buffer)? {
            Object::Buffer(b) => Ok(b),
            _ => Err("text buffer required".into()),
        }
    }
    pub(super) fn annotated_mut(&mut self, buffer: Value) -> Result<&mut annotations::AnnotatedBuffer> {
        let i = buffer.index(0)?.checked_sub(1).ok_or("buffer required")?;
        match self.objects.get_mut(i) {
            Some(Object::Buffer(b)) => Ok(b),
            _ => Err("text buffer required".into()),
        }
    }
    /// The anchors markers in BUFFER live in: a text buffer's, or a
    /// unibyte buffer's, whose byte offsets are its character positions.
    pub(crate) fn marker_anchors(&self, buffer: Value) -> Result<&crate::anchors::AnchorTree> {
        match self.object(buffer)? {
            Object::Buffer(b) => Ok(&b.anchors),
            Object::ByteBuffer(b) => Ok(&b.anchors),
            _ => Err("buffer required".into()),
        }
    }
    pub(crate) fn marker_anchors_mut(&mut self, buffer: Value) -> Result<&mut crate::anchors::AnchorTree> {
        let i = buffer.index(0)?.checked_sub(1).ok_or("buffer required")?;
        match self.objects.get_mut(i) {
            Some(Object::Buffer(b)) => Ok(&mut b.anchors),
            Some(Object::ByteBuffer(b)) => Ok(&mut b.anchors),
            _ => Err("buffer required".into()),
        }
    }
    /// The byte offset MARKER points at in its buffer, or None if it points nowhere.
    pub(crate) fn marker_byte(&self, marker: &Marker) -> Result<Option<usize>> {
        let Some(anchor) = marker.anchor else { return Ok(None) };
        Ok(Some(self.marker_anchors(marker.buffer)?.resolve(anchor).ok_or("invalid marker anchor")? as usize))
    }
    pub(super) fn marker(&self, v: Value) -> Result<&Marker> {
        match self.object(v)? {
            Object::Marker(m) => Ok(m),
            _ => Err("marker required".into()),
        }
    }
    pub(crate) fn overlay(&self, v: Value) -> Result<&Overlay> {
        match self.object(v)? {
            Object::Overlay(o) => Ok(o),
            _ => Err("overlay required".into()),
        }
    }
    fn current_annotation_buffer(&mut self) -> Result<Value> {
        self.buffer_index()?;
        let v = self.slots[self.buffer_slot].value;
        self.annotated(v)?;
        Ok(v)
    }
    /// The current buffer, where a marker given no buffer goes.
    fn current_marker_buffer(&mut self) -> Result<Value> {
        self.buffer_index()?;
        Ok(self.slots[self.buffer_slot].value)
    }
    /// MARKER's insertion type: Right advances it on insertion at its position.
    pub(super) fn set_marker_bias(&mut self, x: Value, b: Bias) -> Result<()> {
        let m = self.marker(x)?.clone();
        if let Some(a) = m.anchor {
            let tree = self.marker_anchors_mut(m.buffer)?;
            let pos = tree.resolve(a).unwrap();
            assert!(tree.set(a, pos, b));
        }
        let Object::Marker(m) = &mut self.objects[x.index(0)? - 1] else {
            unreachable!()
        };
        m.bias = b;
        Ok(())
    }
    pub(super) fn marker_position_value(&self, v: Value) -> Result<Value> {
        let m = self.marker(v)?;
        match self.marker_byte(m)? {
            Some(pos) => match self.object(m.buffer)? {
                Object::Buffer(b) => Value::integer((b.char_of_byte(ByteOffset(pos))?.0 + 1) as i64),
                _ => Value::integer(pos as i64 + 1),
            },
            None => Ok(Value::NIL),
        }
    }
    pub(super) fn scalar_position(&self, v: Value) -> Result<i64> {
        if let Ok(n) = v.as_integer() {
            return Ok(n);
        }
        self.marker_position_value(v)?.as_integer()
    }
    fn buffer_position(&self, buffer: Value, v: Value, clip: bool) -> Result<u32> {
        self.require_live_buffer(buffer)?;
        let n = self.scalar_position(v)?;
        if let Object::ByteBuffer(b) = self.object(buffer)? {
            let max = b.bytes.len() as i64 + 1;
            if !clip && (n < 1 || n > max) {
                return Err("index out of range".into());
            }
            return u32::try_from(n.clamp(1, max) - 1).map_err(|_| "buffer too large".into());
        }
        let b = self.annotated(buffer)?;
        let max = b.len_chars() as i64 + 1;
        let n = if clip {
            n.clamp(1, max)
        } else {
            if n < 1 || n > max {
                return Err("index out of range".into());
            }
            n
        };
        u32::try_from(b.byte_of_char(CharOffset((n - 1) as usize))?.0)
            .map_err(|_| "buffer too large".into())
    }
    pub(super) fn set_marker_object(
        &mut self,
        v: Value,
        pos: Value,
        buffer: Value,
    ) -> Result<Value> {
        let m = self.marker(v)?.clone();
        // GNU set_marker_internal detaches on a dead destination before
        // inspecting POSITION; match-data can still refer to that buffer.
        let pos = if self.buffer_state(buffer).is_ok_and(|state| !state.live) {
            Value::NIL
        } else if pos != Value::NIL && pos.as_integer().is_err() {
            self.marker_position_value(pos)?
        } else {
            pos
        };
        let destination = if pos == Value::NIL {
            None
        } else {
            Some(self.buffer_position(buffer, pos, true)?)
        };
        if let (Some(a), Some(p)) = (m.anchor, destination)
            && m.buffer == buffer
        {
            assert!(self.marker_anchors_mut(buffer)?.set(a, p, m.bias));
            return Ok(v);
        }
        if let Some(a) = m.anchor {
            self.marker_anchors_mut(m.buffer)?.remove(a);
        }
        let anchor = destination.map(|p| self.marker_anchors_mut(buffer).unwrap().create(p, m.bias).unwrap());
        let Object::Marker(target) = &mut self.objects[v.index(0)? - 1] else {
            unreachable!()
        };
        target.buffer = if anchor.is_some() { buffer } else { Value::NIL };
        target.anchor = anchor;
        Ok(v)
    }
    /// Whether overlay primitive P with ARGS changes what a window may show,
    /// as GNU's modify_overlay calls tell redisplay (buffer.c): a new
    /// overlay has no properties yet, deleting a detached one changes
    /// nothing, and `overlay-put' changes something only with a value not
    /// `eq' to the one there. Moving or removing overlays always counts.
    fn changes_overlays(&self, p: Primitive, args: &[Value]) -> bool {
        use Primitive::*;
        match (p, args) {
            (MakeOverlay, _) => false,
            (DeleteOverlay, [overlay, ..]) => !matches!(self.overlay_range(*overlay), Ok(None)),
            (OverlayPut, [overlay, prop, value, ..]) => self.overlay_get(*overlay, *prop).map_or(true, |old| old != *value),
            (MoveOverlay | RemoveOverlays | DeleteOverlay | OverlayPut, _) => true,
            _ => false,
        }
    }
    pub(crate) fn overlay_range(&self, v: Value) -> Result<Option<(u32, u32)>> {
        let o = self.overlay(v)?;
        let (Some(a), Some(b)) = (o.start, o.end) else {
            return Ok(None);
        };
        let tree = &self.annotated(o.buffer)?.anchors;
        Ok(Some((
            tree.resolve(a).ok_or("invalid overlay anchor")?,
            tree.resolve(b).ok_or("invalid overlay anchor")?,
        )))
    }
    pub(crate) fn delete_overlay_object(&mut self, v: Value) -> Result<()> {
        let source = self.overlay(v)?;
        let o = Overlay {
            buffer: source.buffer,
            start: source.start,
            end: source.end,
            front: source.front,
            rear: source.rear,
            properties: Vec::new(),
        };
        if o.buffer != Value::NIL {
            let b = self.annotated_mut(o.buffer)?;
            if let Some(a) = o.start {
                b.intervals.remove(a, &b.anchors);
                b.reversed_pairs.remove(&a);
                b.anchors.remove(a);
            }
            if let Some(a) = o.end {
                b.anchors.remove(a);
            }
            if let Some(a) = o.start {
                b.evaporating.remove(&a);
            }
        }
        let Object::Overlay(o) = &mut self.objects[v.index(0)? - 1] else {
            unreachable!()
        };
        o.buffer = Value::NIL;
        o.start = None;
        o.end = None;
        Ok(())
    }
    fn move_overlay_object(
        &mut self,
        v: Value,
        a: Value,
        b: Value,
        buffer: Value,
    ) -> Result<Value> {
        let mut start = self.buffer_position(buffer, a, true)?;
        let mut end = self.buffer_position(buffer, b, true)?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        let source = self.overlay(v)?;
        let o = Overlay {
            buffer: source.buffer,
            start: source.start,
            end: source.end,
            front: source.front,
            rear: source.rear,
            properties: Vec::new(),
        };
        let evaporates = self.names.get("evaporate").is_some_and(|&slot| {
            source
                .properties
                .iter()
                .any(|&(k, v)| k == Value::reference(slot + 1, 7) && v != Value::NIL)
        });
        if o.buffer == buffer {
            let target = self.annotated_mut(buffer)?;
            target.intervals.remove(o.start.unwrap(), &target.anchors);
            assert!(target.anchors.set(o.start.unwrap(), start, o.front));
            assert!(target.anchors.set(o.end.unwrap(), end, o.rear));
            target
                .intervals
                .insert(v, o.start.unwrap(), o.end.unwrap(), &target.anchors);
        } else {
            self.delete_overlay_object(v)?;
            let target = self.annotated_mut(buffer)?;
            let a = target
                .anchors
                .create(start, o.front)
                .ok_or("anchor limit")?;
            let b = target.anchors.create(end, o.rear).ok_or("anchor limit")?;
            if o.front == Bias::Right && o.rear == Bias::Left {
                target.reversed_pairs.insert(a, b);
            }
            target.intervals.insert(v, a, b, &target.anchors);
            if evaporates {
                target.evaporating.insert(a, v);
            }
            let Object::Overlay(o) = &mut self.objects[v.index(0)? - 1] else {
                unreachable!()
            };
            o.buffer = buffer;
            o.start = Some(a);
            o.end = Some(b);
        }
        if evaporates {
            self.finish_annotation_edit(buffer)?;
        }
        Ok(v)
    }
    pub(super) fn finish_annotation_edit(&mut self, buffer: Value) -> Result<()> {
        // A deletion empties the overlays of every buffer sharing the text
        // (adjust_overlays_for_delete, buffer.c:3621).
        let family = self.text_family(buffer.index(0)? - 1);
        let buffers: Vec<Value> = if family.is_empty() { vec![buffer] }
            else { family.into_iter().map(|i| Value::reference(i + 1, 0)).collect() };
        let mut expired = Vec::new();
        for member in buffers {
            for &v in self.annotated(member)?.evaporating.values() {
                if self.overlay_range(v)?.is_some_and(|(a, b)| a == b) {
                    expired.push(v);
                }
            }
        }
        expired.sort_unstable_by_key(|v| v.0);
        for v in expired {
            self.delete_overlay_object(v)?;
        }
        Ok(())
    }
    /// buffer.c Foverlay_get: intervals.c lookup_char_property on OVERLAY's
    /// plist with TEXTPROP false. PROP's own entry, else the last `category'
    /// symbol's PROP, else the first non-nil `char-property-alias-alist'
    /// alternative; never `default-text-properties'.
    pub(crate) fn overlay_get(&self, overlay: Value, prop: Value) -> Result<Value> {
        let properties = &self.overlay(overlay)?.properties;
        let category = self.names.get("category").map(|&slot| Value::reference(slot + 1, 7));
        let mut fallback = Value::NIL;
        for &(key, value) in properties {
            if key == prop { return Ok(value); }
            if Some(key) == category && (value == Value::NIL || value == Value::TRUE || value.index(7).is_ok()) {
                fallback = self.symbol_property(value, prop)?;
            }
        }
        if fallback != Value::NIL { return Ok(fallback); }
        for entry in self.list_values(self.annotation_variable("char-property-alias-alist"))? {
            let Ok((key, aliases)) = self.cons_pair(entry) else { continue };
            if key != prop { continue; }
            for alias in self.list_values(aliases)? {
                if let Some(&(_, value)) = properties.iter().find(|(k, _)| *k == alias)
                    && value != Value::NIL { return Ok(value); }
            }
            break;
        }
        Ok(Value::NIL)
    }
    pub(crate) fn overlay_property(&self, v: Value, key: Value) -> Result<Value> {
        Ok(self
            .overlay(v)?
            .properties
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(Value::NIL, |(_, v)| *v))
    }
    pub(crate) fn get_char_property_values(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
    ) -> Result<Value> {
        Ok(self.get_char_property_and_overlay_values(position, property, object)?.0)
    }

    /// textprop.c get_char_property_and_overlay: POSITION's PROPERTY in
    /// OBJECT and the overlay that supplied it, nil for a text property.
    pub(crate) fn get_char_property_and_overlay_values(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
    ) -> Result<(Value, Value)> {
        // A window means its buffer, seeing only overlays whose `window'
        // property is nil or that window.
        let (object, window) = match self.object_if(object) {
            _ if object == Value::NIL => {
                self.buffer_index()?;
                (self.slots[self.buffer_slot].value, None)
            }
            Some(Object::Window(w)) => (w.buffer, Some(object)),
            _ => (object, None),
        };
        let at = if matches!(self.object_if(object), Some(Object::Buffer(_))) {
            let byte = self.buffer_position(object, position, false)?;
            let buffer = self.annotated(object)?;
            let found = buffer
                .intervals
                .query_with_end(
                    byte,
                    byte,
                    true,
                    byte == buffer.anchors.extent(),
                    &buffer.anchors,
                )
                .0;
            let at = buffer.char_of_byte(ByteOffset(byte as usize))?.0 + 1;
            let found = match window {
                // window.h overlay_matches_window: a `window' that is not a
                // window object restricts nothing.
                Some(window) => {
                    let key = self.intern("window");
                    let mut kept = vec![];
                    for o in found {
                        let w = self.overlay_get(o, key)?;
                        if w == window || !matches!(self.object_if(w), Some(Object::Window(_))) { kept.push(o); }
                    }
                    kept
                }
                None => found,
            };
            let hit = self.ordered_overlay_hit(found, property)?;
            if hit.0 != Value::NIL {
                return Ok(hit);
            }
            at
        } else {
            usize::try_from(position.as_integer()?).map_err(|_| "index out of range")?
        };
        let raw = self.annotation_raw(object, at)?;
        Ok((self.annotation_resolve(&raw, property)?, Value::NIL))
    }

    /// Fget_text_property (POSITION, PROPERTY, OBJECT) for a buffer of
    /// either kind or a string: args-out-of-range outside its text.
    pub(crate) fn text_property_at(&self, object: Value, position: i64, property: Value) -> std::result::Result<Value, crate::prim::Fault> {
        use crate::prim::Fault;
        let raw = match usize::try_from(position).ok().map(|at| self.annotation_raw(object, at)) {
            Some(Ok(raw)) => raw,
            Some(Err(e)) if e != "index out of range" => return Err(Fault::Error(e)),
            _ => {
                let at = Value::integer(position).map_err(Fault::Error)?;
                return Err(Fault::Range(at, at));
            }
        };
        self.annotation_resolve(&raw, property).map_err(Fault::Error)
    }

    /// get_char_property_and_overlay (textprop.c) with a window as OBJECT,
    /// as the display iterator asks: PROPERTY of the character at BYTE of
    /// BUFFER from the highest overlay that applies to WINDOW (an overlay
    /// whose `window' property is another window does not), else from the
    /// text, together with that overlay or nil.
    pub(crate) fn window_char_property(&self, buffer: Value, byte: usize, property: Value, window: Value) -> Result<(Value, Value)> {
        let text = self.annotated(buffer)?;
        let at = byte as u32;
        let mut found = text.intervals.query_with_end(at, at, true, at == text.anchors.extent(), &text.anchors).0;
        found.retain(|&overlay| self.overlay_applies_to(overlay, window));
        found.sort_by_key(|&overlay| self.overlay_order_key(overlay));
        for overlay in found.into_iter().rev() {
            let value = self.overlay_property(overlay, property)?;
            if value != Value::NIL { return Ok((value, overlay)); }
        }
        let raw = self.annotation_raw(buffer, text.char_of_byte(ByteOffset(byte))?.0 + 1)?;
        Ok((self.annotation_resolve(&raw, property)?, Value::NIL))
    }
    /// The display ignores an overlay whose `window' property names
    /// another window (xdisp.c load_overlay_strings).
    pub(crate) fn overlay_applies_to(&self, overlay: Value, window: Value) -> bool {
        let named = self.overlay_property_named(overlay, "window").unwrap_or(Value::NIL);
        !matches!(self.object_if(named), Some(Object::Window(_))) || named == window
    }
    /// An overlay's start and end as character positions, if it is live.
    pub(crate) fn overlay_positions(&self, overlay: Value) -> Result<Option<(i64, i64)>> {
        let Some((start, end)) = self.overlay_range(overlay)? else { return Ok(None) };
        let text = self.annotated(self.overlay(overlay)?.buffer)?;
        let position = |byte: u32| text.char_of_byte(ByteOffset(byte as usize)).map(|c| c.0 as i64 + 1);
        Ok(Some((position(start)?, position(end)?)))
    }

    /// editfns.c Fget_pos_property: the PROPERTY a character inserted at
    /// POSITION would get, from the highest overlay that would cover it,
    /// else from the text property it would inherit (textprop.c
    /// text_property_stickiness).
    pub(crate) fn get_pos_property_values(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
    ) -> std::result::Result<Value, crate::prim::Fault> {
        use crate::prim::Fault;
        let pos = self.scalar_position(position).map_err(Fault::Error)?;
        let object = match self.object_if(object) {
            _ if object == Value::NIL => self.current_annotation_buffer().map_err(Fault::Error)?,
            Some(Object::Window(w)) => w.buffer,
            _ => object,
        };
        // Fget_text_property: (args-out-of-range AT AT) outside the text.
        let raw_at = |vm: &mut Self, at: i64| -> std::result::Result<Vec<(String, Value)>, Fault> {
            match usize::try_from(at).ok().map(|at| vm.annotation_raw(object, at)) {
                Some(Ok(raw)) => Ok(raw),
                Some(Err(e)) if e != "index out of range" => Err(Fault::Error(e)),
                _ => {
                    let at = Value::integer(at).map_err(Fault::Error)?;
                    let condition = vm.intern("args-out-of-range");
                    let data = vm.list_value(&[at, at]);
                    Err(Fault::Signal(condition, data))
                }
            }
        };
        let text_property = |vm: &mut Self, at: i64, key: Value| -> std::result::Result<Value, Fault> {
            let raw = raw_at(vm, at)?;
            vm.annotation_resolve(&raw, key).map_err(Fault::Error)
        };
        if !matches!(self.object_if(object), Some(Object::Buffer(_))) {
            return text_property(self, pos, property);
        }
        let b = self.annotated(object).map_err(Fault::Error)?;
        let begv = b.char_of_byte(b.accessible().start).map_err(Fault::Error)?.0 as i64 + 1;
        let max = b.len_chars() as i64 + 1;
        let byte = |n: i64| b.byte_of_char(CharOffset((n.clamp(1, max) - 1) as usize)).map(|b| b.0 as u32);
        let (at, low, high) = (byte(pos), byte(pos - 1), byte(pos + 1));
        let (at, low, high) = (at.map_err(Fault::Error)?, low.map_err(Fault::Error)?, high.map_err(Fault::Error)?);
        let mut candidates = Vec::new();
        // ITREE_FOREACH (posn - 1, posn + 1): every overlay meeting that span.
        self.visit_overlay_bytes(object, ByteOffset(low as usize)..ByteOffset(high as usize), false, |overlay, _| candidates.push(overlay))
            .map_err(Fault::Error)?;
        let mut best: Option<(Value, Value)> = None;
        for overlay in candidates {
            let value = self.overlay_property(overlay, property).map_err(Fault::Error)?;
            let Some((start, end)) = self.overlay_range(overlay).map_err(Fault::Error)? else { continue };
            let o = self.overlay(overlay).map_err(Fault::Error)?;
            let covers = start <= at && at <= end
                && !(start == at && o.front == Bias::Right)
                && !(end == at && o.rear != Bias::Right);
            if value == Value::NIL || !covers { continue; }
            if best.is_none_or(|(winner, _)| self.overlay_order_key(winner) < self.overlay_order_key(overlay)) {
                best = Some((overlay, value));
            }
        }
        if let Some((_, value)) = best { return Ok(value); }
        // text_property_stickiness: rear-sticky by default, unless
        // text-property-default-nonsticky or rear-nonsticky says otherwise;
        // front-sticky inherits from after; a conflict prefers the side
        // whose value is non-nil, rear first.
        let nonsticky = self.slot("text-property-default-nonsticky");
        let defaults = self.variable_value(nonsticky).map_err(Fault::Error)?;
        let mut default = Value::NIL;
        let mut tail = defaults;
        while let Ok((entry, rest)) = self.cons_pair(tail) {
            if self.cons_pair(entry).is_ok_and(|(key, _)| key == property) { default = entry; break; }
            tail = rest;
        }
        let ignore_previous = pos <= begv;
        let member = |vm: &Self, mut list: Value| -> bool {
            while let Ok((item, rest)) = vm.cons_pair(list) {
                if item == property { return true; }
                list = rest;
            }
            false
        };
        let rear_sticky = !(ignore_previous || self.cons_pair(default).is_ok_and(|(_, tail)| tail != Value::NIL)) && {
            let key = self.intern("rear-nonsticky");
            let rear = text_property(self, pos - 1, key)?;
            !(if self.cons_pair(rear).is_ok() { member(self, rear) } else { rear != Value::NIL })
        };
        let key = self.intern("front-sticky");
        let front = text_property(self, pos, key)?;
        let front_sticky = front == Value::TRUE || (self.cons_pair(front).is_ok() && member(self, front));
        let stickiness = match (rear_sticky, front_sticky) {
            (true, false) => -1,
            (false, true) => 1,
            (false, false) => 0,
            (true, true) => if ignore_previous || text_property(self, pos - 1, property)? == Value::NIL { 1 } else { -1 },
        };
        match stickiness {
            1 => text_property(self, pos, property),
            -1 if pos > begv => text_property(self, pos - 1, property),
            _ => Ok(Value::NIL),
        }
    }

    pub(super) fn annotation_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        if p == GetCharProperty {
            if !(2..=3).contains(&n) { return Err("wrong primitive arity".into()); }
            let object = if n == 3 { self.stack[start + 2] } else { Value::NIL };
            return self.get_char_property_values(self.stack[start], self.stack[start + 1], object);
        }
        if (p as usize) < MakeMarker as usize {
            return self.text_annotation_primitive(p, start, n);
        }
        if self.changes_overlays(p, &self.stack[start..start + n]) {
            self.display_state.overlays += 1;
        }
        let mut fixed = [Value::NIL; 5];
        let variable = if p == InsertBeforeMarkers && n > 5 {
            Some(self.stack[start..start + n].to_vec())
        } else {
            None
        };
        if n <= 5 {
            fixed[..n].copy_from_slice(&self.stack[start..start + n]);
        }
        let args = variable.as_deref().unwrap_or(&fixed[..n.min(5)]);
        let arity = match p {
            MakeMarker => n == 0,
            SetMarker | MoveOverlay => {
                if p == SetMarker {
                    (2..=3).contains(&n)
                } else {
                    (3..=4).contains(&n)
                }
            }
            CopyMarker => n <= 2,
            MarkerPosition | MarkerBuffer | MarkerInsertionType | DeleteOverlay | OverlayStart
            | OverlayEnd | OverlaysAt | NextOverlayChange => n == 1,
            SetMarkerInsertionType | OverlayGet | OverlaysIn => n == 2,
            OverlayPut => n == 3,
            MakeOverlay => (2..=5).contains(&n),
            RemoveOverlays => n <= 4,
            InsertBeforeMarkers => true,
            _ => false,
        };
        if !arity {
            return Err("wrong primitive arity".into());
        }
        let x = args.first().copied().unwrap_or(Value::NIL);
        let bias = |v| {
            if v == Value::NIL {
                Bias::Left
            } else {
                Bias::Right
            }
        };
        match p {
            MakeMarker => Ok(self.allocate_object(Object::Marker(Box::new(Marker {
                buffer: Value::NIL,
                anchor: None,
                bias: Bias::Left,
            })))),
            SetMarker => {
                let buffer = if n == 3 && args[2] != Value::NIL {
                    args[2]
                } else {
                    self.current_marker_buffer()?
                };
                self.set_marker_object(x, args[1], buffer)
            }
            CopyMarker => {
                // Fcopy_marker: no MARKER makes a marker pointing nowhere.
                if x == Value::NIL {
                    let bias = if n == 2 { bias(args[1]) } else { Bias::Left };
                    return Ok(self.allocate_object(Object::Marker(Box::new(Marker { buffer: Value::NIL, anchor: None, bias }))));
                }
                let pos = if x.as_integer().is_ok() {
                    x
                } else {
                    self.marker_position_value(x)?
                };
                let buffer = if x.as_integer().is_ok() {
                    self.current_marker_buffer()?
                } else {
                    self.marker(x)?.buffer
                };
                let m = self.allocate_object(Object::Marker(Box::new(Marker {
                    buffer: Value::NIL,
                    anchor: None,
                    bias: if n == 2 { bias(args[1]) } else { Bias::Left },
                })));
                self.set_marker_object(m, pos, buffer)
            }
            MarkerPosition => self.marker_position_value(x),
            MarkerBuffer => Ok(self.marker(x)?.buffer),
            MarkerInsertionType => Ok(if self.marker(x)?.bias == Bias::Right {
                Value::TRUE
            } else {
                Value::NIL
            }),
            SetMarkerInsertionType => {
                self.set_marker_bias(x, bias(args[1]))?;
                Ok(args[1])
            }
            InsertBeforeMarkers => {
                let index = self.buffer_index()?;
                if matches!(self.objects[index], Object::ByteBuffer(_)) {
                    let mut bytes = Vec::new();
                    for &v in args {
                        bytes.extend(self.unibyte_text(v)?);
                    }
                    let Object::ByteBuffer(b) = &mut self.objects[index] else { unreachable!("unibyte buffer") };
                    b.insert_before_markers(&bytes)?;
                    self.display_state.version += 1;
                    return Ok(Value::NIL);
                }
                let buffer = self.current_annotation_buffer()?;
                let mut bytes = Vec::new();
                let mut properties = properties::Properties::new();
                for &v in args {
                    if let Ok(n) = v.as_integer() {
                        let c = u32::try_from(n)
                            .ok()
                            .and_then(char::from_u32)
                            .ok_or("invalid character")?;
                        bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                    } else {
                        let text = self.annotation_string(v)?;
                        bytes.extend_from_slice(text.as_bytes());
                        if !Self::image_value(v)
                            && let Ok(Object::String(s) | Object::RawString(s)) = self.object(v)
                        {
                            properties
                                .copy_into(ByteOffset(bytes.len() - text.len()), &s.properties);
                        }
                    }
                }
                let marks = self.family_record(index);
                let edited = self.annotated_mut(buffer).and_then(|b| {
                    let at = b.point;
                    b.replace_with_bias(at..at, &bytes, true)?;
                    b.properties.copy_into(at, &properties);
                    Ok(())
                });
                self.family_fetch(marks);
                edited?;
                self.finish_annotation_edit(buffer)?;
                Ok(Value::NIL)
            }
            MakeOverlay => {
                let buffer = if n > 2 && args[2] != Value::NIL {
                    args[2]
                } else {
                    self.current_annotation_buffer()?
                };
                // Validate before creating identity.
                self.buffer_position(buffer, args[0], true)?;
                self.buffer_position(buffer, args[1], true)?;
                let o = self.allocate_object(Object::Overlay(Box::new(Overlay {
                    buffer: Value::NIL,
                    start: None,
                    end: None,
                    front: if n > 3 { bias(args[3]) } else { Bias::Left },
                    rear: if n > 4 { bias(args[4]) } else { Bias::Left },
                    properties: Vec::new(),
                })));
                self.move_overlay_object(o, args[0], args[1], buffer)
            }
            MoveOverlay => {
                let buffer = if n == 4 && args[3] != Value::NIL {
                    args[3]
                } else {
                    let b = self.overlay(x)?.buffer;
                    if b == Value::NIL {
                        self.current_annotation_buffer()?
                    } else {
                        b
                    }
                };
                self.move_overlay_object(x, args[1], args[2], buffer)
            }
            DeleteOverlay => {
                self.delete_overlay_object(x)?;
                Ok(Value::NIL)
            }
            OverlayStart | OverlayEnd => {
                if let Some((a, b)) = self.overlay_range(x)? {
                    let buffer = self.annotated(self.overlay(x)?.buffer)?;
                    Value::integer(
                        (buffer
                            .char_of_byte(ByteOffset(
                                if p == OverlayStart { a } else { b } as usize
                            ))?
                            .0
                            + 1) as i64,
                    )
                } else {
                    Ok(Value::NIL)
                }
            }
            OverlayGet => self.overlay_get(x, args[1]),
            OverlayPut => {
                self.overlay(x)?;
                let Object::Overlay(o) = &mut self.objects[x.index(0)? - 1] else {
                    unreachable!()
                };
                if let Some((_, v)) = o.properties.iter_mut().find(|(k, _)| *k == args[1]) {
                    *v = args[2];
                } else {
                    o.properties.push((args[1], args[2]));
                }
                let buffer = self.overlay(x)?.buffer;
                let slot = args[1].index(7).ok().and_then(|i| i.checked_sub(1));
                if slot.is_some_and(|i| self.names.get("evaporate") == Some(&i))
                    && buffer != Value::NIL
                {
                    let anchor = self.overlay(x)?.start.unwrap();
                    let list = &mut self.annotated_mut(buffer)?.evaporating;
                    list.remove(&anchor);
                    if args[2] != Value::NIL {
                        list.insert(anchor, x);
                        self.finish_annotation_edit(buffer)?;
                    }
                }
                Ok(args[2])
            }
            OverlaysAt | OverlaysIn | NextOverlayChange | RemoveOverlays => {
                let buffer = self.current_annotation_buffer()?;
                let (a, b) = if p == RemoveOverlays {
                    let t = self.annotated(buffer)?;
                    (
                        if n == 0 || args[0] == Value::NIL {
                            t.accessible().start.0 as u32
                        } else {
                            self.buffer_position(buffer, args[0], false)?
                        },
                        if n < 2 || args[1] == Value::NIL {
                            t.accessible().end.0 as u32
                        } else {
                            self.buffer_position(buffer, args[1], false)?
                        },
                    )
                } else {
                    let a = self.buffer_position(buffer, x, false)?;
                    let b = if p == OverlaysIn {
                        self.buffer_position(buffer, args[1], false)?
                    } else {
                        a
                    };
                    (a.min(b), a.max(b))
                };
                let (a, b) = (a.min(b), a.max(b));
                let target = self.annotated(buffer)?;
                let next = if p == NextOverlayChange {
                    target
                        .intervals
                        .next(a, target.accessible().end.0 as u32, &target.anchors)
                } else {
                    0
                };
                let found = if p == NextOverlayChange {
                    Vec::new()
                } else {
                    target
                        .intervals
                        .query_with_end(
                            a,
                            b,
                            p == OverlaysAt,
                            b == target.anchors.extent(),
                            &target.anchors,
                        )
                        .0
                };
                match p {
                    NextOverlayChange => Value::integer(
                        (self
                            .annotated(buffer)?
                            .char_of_byte(ByteOffset(next as usize))?
                            .0
                            + 1) as i64,
                    ),
                    RemoveOverlays => {
                        for o in found {
                            if self
                                .overlay_property(o, args.get(2).copied().unwrap_or(Value::NIL))?
                                == args.get(3).copied().unwrap_or(Value::NIL)
                            {
                                let (s, e) = self.overlay_range(o)?.unwrap();
                                let cv = |vm: &Self, p| {
                                    Value::integer(
                                        (vm.annotated(buffer)?
                                            .char_of_byte(ByteOffset(p as usize))?
                                            .0
                                            + 1) as i64,
                                    )
                                };
                                if s < a && e > b {
                                    let mut left = self.overlay(o)?.clone();
                                    left.buffer = Value::NIL;
                                    left.start = None;
                                    left.end = None;
                                    left.front = Bias::Left;
                                    left.rear = Bias::Left;
                                    let copy =
                                        self.allocate_object(Object::Overlay(Box::new(left)));
                                    self.move_overlay_object(
                                        copy,
                                        cv(self, s)?,
                                        cv(self, a)?,
                                        buffer,
                                    )?;
                                    self.move_overlay_object(
                                        o,
                                        cv(self, b)?,
                                        cv(self, e)?,
                                        buffer,
                                    )?;
                                } else if s < a {
                                    self.move_overlay_object(
                                        o,
                                        cv(self, s)?,
                                        cv(self, a)?,
                                        buffer,
                                    )?;
                                } else if e > b {
                                    self.move_overlay_object(
                                        o,
                                        cv(self, b)?,
                                        cv(self, e)?,
                                        buffer,
                                    )?;
                                } else {
                                    self.delete_overlay_object(o)?;
                                }
                            }
                        }
                        Ok(Value::NIL)
                    }
                    _ => Ok(self.list_value(&found)),
                }
            }
            _ => Err("annotation primitive required".into()),
        }
    }
}

impl Vm {
    pub(crate) fn next_property_change_values(
        &mut self,
        position: Value,
        object: Value,
        limit: Value,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        if limit == Value::TRUE {
            return Err(Fault::Error(
                "raw text-property interval boundaries (LIMIT t) are not implemented".into(),
            ));
        }
        let limit = if limit == Value::NIL {
            None
        } else {
            Some(self.text_property_position(limit)?)
        };
        let object = self.property_query_object(object)?;
        let (_, bytes, _) = self.property_query_cursor(object, position, false)?;
        let next = self.property_next_text_boundary(object, bytes.start, bytes.end);
        let next = self.property_query_scalar(object, next)?;
        let end = self.property_query_scalar(object, bytes.end)?;
        if next < end && limit.is_none_or(|limit| next < limit) {
            return Value::integer(next).map_err(Fault::Error);
        }
        limit.map_or(Ok(Value::NIL), |n| Value::integer(n).map_err(Fault::Error))
    }

    /// textprop.c Ftext_property_not_all: the first position in START..END
    /// whose PROPERTY (as `get-text-property' reads it) is not eq to VALUE.
    /// A property is constant within an interval, so a per-position scan
    /// returns GNU's interval start.
    pub(crate) fn text_property_not_all_values(
        &mut self,
        start: Value,
        end: Value,
        property: Value,
        value: Value,
        object: Value,
    ) -> crate::prim::Out {
        self.text_property_scan(start, end, property, value, object, false)
    }

    /// textprop.c Ftext_property_any.
    pub(crate) fn text_property_any_values(
        &mut self,
        start: Value,
        end: Value,
        property: Value,
        value: Value,
        object: Value,
    ) -> crate::prim::Out {
        self.text_property_scan(start, end, property, value, object, true)
    }

    /// The first position from START to END whose PROPERTY is (EQUAL) or is
    /// not (otherwise) `eq' to VALUE.
    fn text_property_scan(
        &mut self,
        start: Value,
        end: Value,
        property: Value,
        value: Value,
        object: Value,
        equal: bool,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        let object = self.property_query_object(object)?;
        // CHECK_STRING_OR_BUFFER comes first in validate_interval_range.
        if self.str_view(object).is_none() && !matches!(self.object_if(object), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            return Err(Fault::Wrong("buffer-or-string-p", object));
        }
        let (mut from, mut to) = (self.text_property_position(start)?, self.text_property_position(end)?);
        // validate_interval_range answers an empty range before checking it.
        if from == to { return Ok(Value::NIL); }
        if from > to { std::mem::swap(&mut from, &mut to); }
        // validate_interval_range: BEGV..ZV of a buffer, 0..length of a string.
        let (low, high) = if let Some(string) = self.str_view(object) { (0, string.len() as i64) } else {
            match self.object_if(object) {
                Some(Object::Buffer(b)) => {
                    let r = b.accessible();
                    (b.char_of_byte(r.start).map_err(Fault::Error)?.0 as i64 + 1, b.char_of_byte(r.end).map_err(Fault::Error)?.0 as i64 + 1)
                }
                Some(Object::ByteBuffer(b)) => (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
                _ => return Err(Fault::Wrong("buffer-or-string-p", object)),
            }
        };
        if from < low || to > high { return Err(Fault::Range(start, end)); }
        // No text or no property intervals: GNU consults no plist at all,
        // not even default-text-properties.
        let intervals = match self.object_if(object) {
            Some(Object::String(s) | Object::RawString(s)) => !s.properties.is_empty(),
            Some(Object::Buffer(b)) => !b.properties.is_empty(),
            Some(Object::ByteBuffer(b)) => !b.properties.is_empty(),
            _ => false,
        };
        // Every position's PROPERTY is then nil.
        if low == high || !intervals {
            return Ok(if (value == Value::NIL) != equal { Value::NIL } else { Value::integer(from).map_err(Fault::Error)? });
        }
        for at in from..to {
            let raw = self.annotation_raw(object, at as usize).map_err(Fault::Error)?;
            if (self.annotation_resolve(&raw, property).map_err(Fault::Error)? == value) == equal {
                return Value::integer(at).map_err(Fault::Error);
            }
        }
        Ok(Value::NIL)
    }

    pub(crate) fn next_single_char_property_change_values(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
        limit: Value,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        let object = self.property_query_object(object)?;
        let string = self.str_view(object).is_some();
        if !string
            && !matches!(
                self.object_if(object),
                Some(Object::Buffer(_) | Object::ByteBuffer(_))
            )
        {
            return Err(Fault::Wrong("bufferp", object));
        }
        if string {
            return self.property_single_change(position, property, object, limit, true);
        }
        self.require_live_buffer(object).map_err(Fault::Error)?;
        let current = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = object;
        let result = self.property_single_change(position, property, object, limit, false);
        self.slots[self.buffer_slot].value = current;
        result
    }

    /// textprop.c Fprevious_single_char_property_change: walking back from
    /// POSITION over text-property and overlay boundaries, the first where
    /// PROPERTY's char-property value before it differs from the value
    /// before POSITION; else LIMIT (default BEGV).
    pub(crate) fn previous_single_char_property_change_values(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
        limit: Value,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        if self.str_view(object).is_some() {
            let found = self.previous_property_change_values(position, Some(property), object, limit)?;
            return match () {
                _ if found != Value::NIL => Ok(found),
                _ if limit == Value::NIL => Ok(Value::integer(0).unwrap()),
                _ if limit.integer_value().is_some() => Ok(limit),
                _ => Err(Fault::Wrong("fixnump", limit)),
            };
        }
        let object = self.property_query_object(object)?;
        if !matches!(self.object_if(object), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            return Err(Fault::Wrong("bufferp", object));
        }
        self.require_live_buffer(object).map_err(Fault::Error)?;
        let current = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = object;
        let result = self.previous_single_char_property_change_here(position, property, limit);
        self.slots[self.buffer_slot].value = current;
        result
    }

    fn previous_single_char_property_change_here(&mut self, position: Value, property: Value, limit: Value) -> crate::prim::Out {
        use crate::prim::Fault;
        let integer = |n: i64| Value::integer(n).map_err(Fault::Error);
        let object = self.slots[self.buffer_slot].value;
        let position = self.text_property_position(position)?;
        let Object::Buffer(b) = self.object(object).map_err(Fault::Error)? else {
            return Err(Fault::Error("property-change queries in unibyte buffers are not implemented".into()));
        };
        let begv = b.char_of_byte(b.accessible().start).map_err(Fault::Error)?.0 as i64 + 1;
        let limit = if limit == Value::NIL { begv } else { self.text_property_position(limit)? };
        if position <= limit {
            return integer(limit.max(begv));
        }
        let initial = self.char_property_accessible(position - i64::from(position >= 0), property)?;
        // Fprevious_char_property_change's steps: each is the nearest
        // overlay endpoint or text-property boundary before the last, never
        // below BEGV, so BEGV ends the walk.
        let boundaries = self.char_property_boundaries_before(object, position, limit.max(begv))?;
        for at in boundaries.into_iter().chain([begv]) {
            if at <= limit { break; }
            if self.char_property_accessible(at - 1, property)? != initial {
                return integer(at);
            }
        }
        integer(limit)
    }

    /// Fget_char_property in the current buffer: POSITION's PROPERTY from
    /// overlays and text properties, args-out-of-range outside BEGV..ZV.
    pub(crate) fn char_property_accessible(&mut self, position: i64, property: Value) -> crate::prim::Out {
        use crate::prim::Fault;
        let object = self.slots[self.buffer_slot].value;
        let (begv, zv) = match self.object(object).map_err(Fault::Error)? {
            Object::Buffer(b) => {
                let r = b.accessible();
                (b.char_of_byte(r.start).map_err(Fault::Error)?.0 as i64 + 1, b.char_of_byte(r.end).map_err(Fault::Error)?.0 as i64 + 1)
            }
            Object::ByteBuffer(b) => (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
            _ => return Err(Fault::Error("buffer required".into())),
        };
        let at = Value::integer(position).map_err(Fault::Error)?;
        if position < begv || position > zv {
            return Err(Fault::Range1(at));
        }
        self.get_char_property_values(at, property, object).map_err(Fault::Error)
    }

    /// The positions above LOW and below POSITION in a text buffer where a
    /// text property run or an overlay starts or ends, nearest first.
    fn char_property_boundaries_before(&self, object: Value, position: i64, low: i64) -> std::result::Result<Vec<i64>, crate::prim::Fault> {
        use crate::prim::Fault;
        let b = self.annotated(object).map_err(Fault::Error)?;
        let range = b.accessible();
        let byte = |n: i64| b.byte_of_char(CharOffset((n.clamp(1, b.len_chars() as i64 + 1) - 1) as usize)).map_err(Fault::Error);
        let (from, to) = (byte(low)?, byte(position)?.min(range.end));
        let mut bytes = Vec::new();
        let mut at = from;
        while at < to {
            at = b.properties.next_change(at, to);
            if at < to { bytes.push(at.0 as u32); }
        }
        b.intervals.visit(from.0 as u32, to.0 as u32, false, &b.anchors, |_, start, end| {
            bytes.extend([start, end].into_iter().filter(|&e| e > from.0 as u32 && e < to.0 as u32));
        });
        bytes.sort_unstable_by(|a, b| b.cmp(a));
        bytes.dedup();
        bytes.into_iter()
            .map(|at| b.char_of_byte(ByteOffset(at as usize)).map(|c| c.0 as i64 + 1).map_err(Fault::Error))
            .collect()
    }

    fn property_single_change(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
        limit: Value,
        string: bool,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        let mut bound = if string && limit != Value::NIL {
            Some(self.text_property_position(limit)?)
        } else {
            None
        };
        let (position, bytes, end) = self.property_query_cursor(object, position, true)?;
        let initial = self
            .get_char_property_values(
                Value::integer(position).map_err(Fault::Error)?,
                property,
                object,
            )
            .map_err(Fault::Error)?;
        if !string && limit != Value::NIL {
            bound = Some(self.text_property_position(limit)?);
        }
        let bound = if string {
            bound.unwrap_or(end)
        } else {
            bound.unwrap_or(end).min(end)
        };
        if bound <= position {
            return Value::integer(bound).map_err(Fault::Error);
        }
        let mut at = bytes.start;
        while at < bytes.end {
            let mut next = self.property_next_text_boundary(object, at, bytes.end);
            if !string {
                next = next.min(
                    self.next_overlay_change_byte(object, at)
                        .map_err(Fault::Error)?,
                );
            }
            let scalar = self.property_query_scalar(object, next)?;
            if next >= bytes.end || scalar >= bound {
                break;
            }
            let value = self
                .get_char_property_values(
                    Value::integer(scalar).map_err(Fault::Error)?,
                    property,
                    object,
                )
                .map_err(Fault::Error)?;
            if value != initial {
                return Value::integer(scalar).map_err(Fault::Error);
            }
            at = next;
        }
        Value::integer(bound).map_err(Fault::Error)
    }

    /// The positions in OBJECT's accessible text, from its start, where its
    /// text properties change: the starts of its property runs (GNU's
    /// intervals, merged where their plists are equal).
    pub(crate) fn property_run_starts(&self, object: Value) -> std::result::Result<Vec<i64>, crate::prim::Fault> {
        use crate::prim::Fault;
        let (bytes, start) = match self.object_if(object) {
            Some(Object::Buffer(b)) => {
                let r = b.accessible();
                (r.clone(), b.char_of_byte(r.start).map_err(Fault::Error)?.0 as i64 + 1)
            }
            _ => {
                let len = self.str_view(object).map_or(0, |s| s.bytes().len());
                (ByteOffset(0)..ByteOffset(len), 0)
            }
        };
        let mut starts = vec![start];
        let mut at = bytes.start;
        while at < bytes.end {
            let next = self.property_next_text_boundary(object, at, bytes.end);
            if next >= bytes.end { break; }
            starts.push(self.property_query_scalar(object, next)?);
            at = next;
        }
        Ok(starts)
    }

    /// textget: OBJECT's text property PROP at character POSITION (no
    /// overlays), with category, aliases and default text properties.
    pub(crate) fn property_textget(&self, object: Value, position: i64, prop: Value) -> std::result::Result<Value, crate::prim::Fault> {
        let raw = self.annotation_raw(object, position as usize).map_err(crate::prim::Fault::Error)?;
        self.annotation_resolve(&raw, prop).map_err(crate::prim::Fault::Error)
    }

    /// fix_position of a non-nil LIMIT.
    fn property_limit(&self, limit: Value) -> std::result::Result<Option<i64>, crate::prim::Fault> {
        if limit == Value::NIL { Ok(None) } else { Ok(Some(self.text_property_position(limit)?)) }
    }

    /// textprop.c Fnext_single_property_change: the next position after
    /// POSITION where PROP's text-property value changes, before LIMIT and
    /// the end of OBJECT; else LIMIT.
    pub(crate) fn next_single_property_change_values(&mut self, position: Value, prop: Value, object: Value, limit: Value) -> crate::prim::Out {
        use crate::prim::Fault;
        let object = self.property_query_object(object)?;
        let limit = self.property_limit(limit)?;
        let (pos, _, end) = self.property_query_cursor(object, position, false)?;
        let none = || limit.map_or(Ok(Value::NIL), |n| Value::integer(n).map_err(Fault::Error));
        if pos >= end { return none(); }
        let here = self.property_textget(object, pos, prop)?;
        for b in self.property_run_starts(object)?.into_iter().filter(|&b| b > pos) {
            if limit.is_some_and(|l| b >= l) { return none(); }
            if self.property_textget(object, b, prop)? != here { return Value::integer(b).map_err(Fault::Error); }
        }
        none()
    }

    /// textprop.c Fprevious_single_property_change (PROP given) and
    /// Fprevious_property_change (PROP None, any property): the previous
    /// position before POSITION where the value changes, after LIMIT and the
    /// start of OBJECT; else LIMIT.
    pub(crate) fn previous_property_change_values(&mut self, position: Value, prop: Option<Value>, object: Value, limit: Value) -> crate::prim::Out {
        use crate::prim::Fault;
        let object = self.property_query_object(object)?;
        let limit = self.property_limit(limit)?;
        let (pos, _, _) = self.property_query_cursor(object, position, false)?;
        let none = || limit.map_or(Ok(Value::NIL), |n| Value::integer(n).map_err(Fault::Error));
        let starts = self.property_run_starts(object)?;
        let start = starts[0];
        let lower = limit.unwrap_or(start);
        if pos <= start { return none(); }
        // The run holding the character before POSITION, walked back over
        // runs with the same value of PROP.
        let run = |b: i64| *starts.iter().rev().find(|&&s| s < b).unwrap_or(&start);
        let mut b = run(pos);
        if let Some(prop) = prop {
            let here = self.property_textget(object, pos - 1, prop)?;
            while b > start && limit.is_none_or(|l| b > l) && self.property_textget(object, b - 1, prop)? == here {
                b = run(b);
            }
        }
        if b <= start || b <= lower { none() } else { Value::integer(b).map_err(Fault::Error) }
    }

    fn property_query_object(
        &mut self,
        object: Value,
    ) -> std::result::Result<Value, crate::prim::Fault> {
        if object != Value::NIL {
            return Ok(object);
        }
        self.buffer_index().map_err(crate::prim::Fault::Error)?;
        Ok(self.slots[self.buffer_slot].value)
    }

    fn property_query_cursor(
        &self,
        object: Value,
        position: Value,
        single: bool,
    ) -> std::result::Result<(i64, std::ops::Range<ByteOffset>, i64), crate::prim::Fault> {
        use crate::prim::Fault;
        let string = self.str_view(object);
        if string.is_none()
            && !matches!(
                self.object_if(object),
                Some(Object::Buffer(_) | Object::ByteBuffer(_))
            )
        {
            return Err(Fault::Wrong("buffer-or-string-p", object));
        }
        let number = self.text_property_position(position)?;
        let error = || {
            if single && string.is_none() {
                Fault::Range1(Value::integer(number).expect("fix_position yields a fixnum"))
            } else {
                Fault::Range(position, position)
            }
        };
        if let Some(string) = string {
            if number < 0 || number as u64 > string.len() as u64 {
                return Err(error());
            }
            let byte = match self.object_if(object) {
                Some(Object::String(s) | Object::RawString(s)) => {
                    s.byte_offset(CharOffset(number as usize)).unwrap()
                }
                Some(Object::Bytes(_)) => number as usize,
                _ => string
                    .chars()
                    .take(number as usize)
                    .map(|code| crate::lstring::char_string(code, &mut [0; 5]))
                    .sum(),
            };
            return Ok((
                number,
                ByteOffset(byte)..ByteOffset(string.bytes().len()),
                string.len() as i64,
            ));
        }
        match self.object_if(object).unwrap() {
            Object::Buffer(buffer) => {
                let range = buffer.accessible();
                let min = buffer.char_of_byte(range.start).map_err(Fault::Error)?.0 as i64 + 1;
                let max = buffer.char_of_byte(range.end).map_err(Fault::Error)?.0 as i64 + 1;
                if number < min || number > max {
                    return Err(error());
                }
                let byte = buffer
                    .byte_of_char(CharOffset(number as usize - 1))
                    .map_err(Fault::Error)?;
                Ok((number, byte..range.end, max))
            }
            Object::ByteBuffer(_) => Err(Fault::Error(
                "property-change queries in unibyte buffers are not implemented".into(),
            )),
            _ => unreachable!(),
        }
    }

    /// The first byte after AT, at most END, where OBJECT's text properties
    /// or, in a buffer, its overlays may change: compute_stop_pos's bound.
    pub(crate) fn next_property_boundary(&self, object: Value, at: ByteOffset, end: ByteOffset) -> Result<ByteOffset> {
        let text = self.property_next_text_boundary(object, at, end);
        if !matches!(self.object_if(object), Some(Object::Buffer(_))) { return Ok(text); }
        Ok(text.min(self.next_overlay_change_byte(object, at)?).min(end))
    }
    fn property_next_text_boundary(
        &self,
        object: Value,
        at: ByteOffset,
        end: ByteOffset,
    ) -> ByteOffset {
        match self.object_if(object) {
            Some(Object::String(s) | Object::RawString(s)) => s.properties.next_change(at, end),
            Some(Object::Buffer(buffer)) => buffer.properties.next_change(at, end),
            _ => end,
        }
    }

    fn property_query_scalar(
        &self,
        object: Value,
        byte: ByteOffset,
    ) -> std::result::Result<i64, crate::prim::Fault> {
        use crate::prim::Fault;
        if let Some(string) = self.str_view(object) {
            if !string.is_multibyte() {
                return Ok(byte.0 as i64);
            }
            let mut prefix = string
                .bytes()
                .get(..byte.0)
                .ok_or_else(|| Fault::Error("invalid property boundary".into()))?;
            let mut count = 0;
            while !prefix.is_empty() {
                let (_, width) = crate::lstring::decode_char(prefix).ok_or_else(|| {
                    Fault::Error("property boundary is not a character boundary".into())
                })?;
                prefix = &prefix[width..];
                count += 1;
            }
            return Ok(count);
        }
        let Object::Buffer(buffer) = self.object(object).map_err(Fault::Error)? else {
            unreachable!()
        };
        Ok(buffer.char_of_byte(byte).map_err(Fault::Error)?.0 as i64 + 1)
    }
}
