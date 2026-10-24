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
    fn annotated_mut(&mut self, buffer: Value) -> Result<&mut annotations::AnnotatedBuffer> {
        let i = buffer.index(0)?.checked_sub(1).ok_or("buffer required")?;
        match self.objects.get_mut(i) {
            Some(Object::Buffer(b)) => Ok(b),
            _ => Err("text buffer required".into()),
        }
    }
    fn marker(&self, v: Value) -> Result<&Marker> {
        match self.object(v)? {
            Object::Marker(m) => Ok(m),
            _ => Err("marker required".into()),
        }
    }
    fn overlay(&self, v: Value) -> Result<&Overlay> {
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
    pub(super) fn marker_position_value(&self, v: Value) -> Result<Value> {
        let m = self.marker(v)?;
        match m.anchor {
            Some(a) => {
                let b = self.annotated(m.buffer)?;
                let pos = b.anchors.resolve(a).ok_or("invalid marker anchor")?;
                Value::integer((b.char_of_byte(ByteOffset(pos as usize))?.0 + 1) as i64)
            }
            None => Ok(Value::NIL),
        }
    }
    fn scalar_position(&self, v: Value) -> Result<i64> {
        if let Ok(n) = v.as_integer() {
            return Ok(n);
        }
        self.marker_position_value(v)?.as_integer()
    }
    fn buffer_position(&self, buffer: Value, v: Value, clip: bool) -> Result<u32> {
        self.require_live_buffer(buffer)?;
        let n = self.scalar_position(v)?;
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
            assert!(self.annotated_mut(buffer)?.anchors.set(a, p, m.bias));
            return Ok(v);
        }
        if let Some(a) = m.anchor {
            self.annotated_mut(m.buffer)?.anchors.remove(a);
        }
        let anchor = destination.map(|p| {
            self.annotated_mut(buffer)
                .unwrap()
                .anchors
                .create(p, m.bias)
                .unwrap()
        });
        let Object::Marker(target) = &mut self.objects[v.index(0)? - 1] else {
            unreachable!()
        };
        target.buffer = if anchor.is_some() { buffer } else { Value::NIL };
        target.anchor = anchor;
        Ok(v)
    }
    fn overlay_range(&self, v: Value) -> Result<Option<(u32, u32)>> {
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
        let mut expired = Vec::new();
        for &v in self.annotated(buffer)?.evaporating.values() {
            if self.overlay_range(v)?.is_some_and(|(a, b)| a == b) {
                expired.push(v);
            }
        }
        expired.sort_unstable_by_key(|v| v.0);
        for v in expired {
            self.delete_overlay_object(v)?;
        }
        Ok(())
    }
    fn overlay_property(&self, v: Value, key: Value) -> Result<Value> {
        Ok(self
            .overlay(v)?
            .properties
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(Value::NIL, |(_, v)| *v))
    }
    pub(super) fn annotation_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        if (p as usize) < MakeMarker as usize {
            let buffer_query = if p == GetCharProperty && (2..=3).contains(&n) {
                let object = if n == 3 && self.stack[start + 2] != Value::NIL {
                    self.stack[start + 2]
                } else {
                    self.buffer_index()?;
                    self.slots[self.buffer_slot].value
                };
                matches!(self.object(object), Ok(Object::Buffer(_)))
            } else {
                false
            };
            if !buffer_query {
                return self.text_annotation_primitive(p, start, n);
            }
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
            CopyMarker => (1..=2).contains(&n),
            MarkerPosition | MarkerBuffer | MarkerInsertionType | DeleteOverlay | OverlayStart
            | OverlayEnd | OverlaysAt | NextOverlayChange => n == 1,
            SetMarkerInsertionType | OverlayGet | OverlaysIn => n == 2,
            OverlayPut => n == 3,
            MakeOverlay => (2..=5).contains(&n),
            RemoveOverlays => n <= 4,
            GetCharProperty => (2..=3).contains(&n),
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
                    self.current_annotation_buffer()?
                };
                self.set_marker_object(x, args[1], buffer)
            }
            CopyMarker => {
                let pos = if x.as_integer().is_ok() {
                    x
                } else {
                    self.marker_position_value(x)?
                };
                let buffer = if x.as_integer().is_ok() {
                    self.current_annotation_buffer()?
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
                let m = self.marker(x)?.clone();
                let b = bias(args[1]);
                if let Some(a) = m.anchor {
                    let tree = &mut self.annotated_mut(m.buffer)?.anchors;
                    let pos = tree.resolve(a).unwrap();
                    assert!(tree.set(a, pos, b));
                }
                let Object::Marker(m) = &mut self.objects[x.index(0)? - 1] else {
                    unreachable!()
                };
                m.bias = b;
                Ok(args[1])
            }
            InsertBeforeMarkers => {
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
                let b = self.annotated_mut(buffer)?;
                let at = b.point;
                b.replace_with_bias(at..at, &bytes, true)?;
                b.properties.copy_into(at, &properties);
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
            OverlayGet => self.overlay_property(x, args[1]),
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
            OverlaysAt | OverlaysIn | NextOverlayChange | RemoveOverlays | GetCharProperty => {
                let buffer = if p == GetCharProperty && n == 3 && args[2] != Value::NIL {
                    args[2]
                } else {
                    self.current_annotation_buffer()?
                };
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
                let mut found = if p == NextOverlayChange {
                    Vec::new()
                } else {
                    target
                        .intervals
                        .query_with_end(
                            a,
                            b,
                            p == OverlaysAt || p == GetCharProperty,
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
                    GetCharProperty => {
                        found.sort_by_key(|&o| {
                            let priority = self
                                .overlay(o)
                                .unwrap()
                                .properties
                                .iter()
                                .find_map(|(k, v)| {
                                    let slot = k.index(7).ok()?.checked_sub(1)?;
                                    (self.names.get("priority") == Some(&slot))
                                        .then(|| v.as_integer().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            let (s, e) = self.overlay_range(o).unwrap().unwrap();
                            (priority, s, std::cmp::Reverse(e), o.index(0).unwrap())
                        });
                        for o in found.into_iter().rev() {
                            let v = self.overlay_property(o, args[1])?;
                            if v != Value::NIL {
                                return Ok(v);
                            }
                        }
                        self.text_annotation_primitive(p, start, n)
                    }
                    _ => Ok(self.list_value(&found)),
                }
            }
            _ => Err("annotation primitive required".into()),
        }
    }
}
