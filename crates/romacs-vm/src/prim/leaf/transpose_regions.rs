//! editfns.c Ftranspose_regions: the two regions' text swapped in place.
//! The span from the first region's start to the second's end keeps its
//! length, so the swap is one overwrite of it (prims/transpose_regions.rs
//! runs modify_text's bracket around it).  Text properties travel with
//! their text, point and markers follow transpose_markers (or keep their
//! characters with LEAVE-MARKERS), and overlays keep their characters, as
//! GNU's overlay tree is not in the marker chain.
use super::*;
use crate::anchors::Anchor;
use crate::coordinates::{ByteOffset, CharOffset};

/// transpose_markers' move of the character position AT for regions
/// S1..E1 and S2..E2 (S1 <= E1 <= S2 <= E2).
fn transposed(at: i64, [s1, e1, s2, e2]: [i64; 4]) -> i64 {
    if at < s1 || at >= e2 { at }
    else if at < e1 { at + (e2 - s2) + (s2 - e1) }
    else if at < s2 { at + (e2 - s2) - (e1 - s1) }
    else { at - (e1 - s1) - (s2 - e1) }
}

/// A position inside the span to place again once the text is swapped:
/// a marker's anchor, an overlay's (its buffer, value and both ends), or
/// a sharing buffer's narrowing and point.
enum Place {
    Marker(Anchor, crate::anchors::Bias, i64),
    Overlay(Value, Value, i64, i64),
    /// A sharing buffer's narrowing and point, which GNU keeps in markers
    /// of the shared text (begv_marker, pt_marker, zv_marker): all three
    /// move as markers do.
    View(usize, [i64; 3]),
}

impl Leaf<'_> {
    /// validate_region on both regions, then their order, the overlap
    /// error and the no-op: the regions as given, each ordered, and in
    /// buffer order [S1, E1, S2, E2]; None when nothing changes.
    pub fn transpose_regions_span(&mut self, startr1: Value, endr1: Value, startr2: Value, endr2: Value) -> std::result::Result<Option<([i64; 4], [i64; 4])>, Fault> {
        let (a1, b1) = self.validate_region(startr1, endr1)?;
        let (a2, b2) = self.validate_region(startr2, endr2)?;
        let [s1, e1, s2, e2] = if a2 < b1 { [a2, b2, a1, b1] } else { [a1, b1, a2, b2] };
        if s2 < e1 { return Err(Fault::Error("Transposed regions overlap".into())); }
        if (s1 == e1 || s2 == e2) && e1 == s2 { return Ok(None); }
        Ok(Some(([a1, b1, a2, b2], [s1, e1, s2, e2])))
    }

    /// The swap of S1..E1 with S2..E2, after modify_text's hooks: undo
    /// recorded as record_change does (one change of the span when the
    /// regions' lengths differ, else one per region), the text and its
    /// properties moved, then point, markers and overlays placed.
    /// ARGS are the regions as given: GNU clears their properties in that
    /// order, and over STARTR1..ENDR2 when the lengths differ.
    pub fn transpose_regions_swap(&mut self, args: [i64; 4], span: [i64; 4], leave_markers: bool) -> std::result::Result<(), Fault> {
        let [s1, e1, s2, e2] = span;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if e1 - s1 == e2 - s2 && e1 != s2 {
            self.vm.record_change_span(index, s1, e1 - s1).map_err(Fault::Error)?;
            self.vm.record_change_span(index, s2, e2 - s2).map_err(Fault::Error)?;
        } else {
            self.vm.record_change_span(index, s1, e2 - s1).map_err(Fault::Error)?;
        }
        // set_text_properties_1 clears the regions as given (from the
        // first's start to the second's end when the lengths differ) before
        // the graft: set_properties records each removed property of each run.
        let [a1, b1, a2, b2] = args;
        if e1 - s1 == e2 - s2 {
            self.record_properties_cleared(index, a1, b1)?;
            self.record_properties_cleared(index, a2, b2)?;
        } else {
            self.record_properties_cleared(index, a1.min(b2), a1.max(b2))?;
        }
        let places = self.transpose_places(index, span)?;
        let moved = |at: i64| if leave_markers { at } else { transposed(at, span) };
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let byte = |b: &crate::annotations::AnnotatedBuffer, c: i64| b.byte_of_char(CharOffset((c - 1) as usize));
                let mut offsets = [ByteOffset(0); 4];
                for (o, c) in offsets.iter_mut().zip(span) { *o = byte(b, c).map_err(Fault::Error)?; }
                let [b1, f1, b2, f2] = offsets;
                let text = [b2..f2, f1..b2, b1..f1].map(|r| b.internal(r).map(|t| t.into_owned()));
                let text = text.into_iter().collect::<crate::Result<Vec<_>>>().map_err(Fault::Error)?.concat();
                let pieces = [b2..f2, f1..b2, b1..f1].map(|r| (r.end.0 - r.start.0, b.properties.slice(r)));
                let point = b.char_of_byte(b.point).map_err(Fault::Error)?.0 as i64 + 1;
                b.overwrite(b1..f2, &text).map_err(Fault::Error)?;
                for key in b.properties.properties_keys() { b.properties.remove(b1..f2, &key); }
                let mut at = b1.0;
                for (len, properties) in &pieces {
                    b.properties.copy_into(ByteOffset(at), properties);
                    at += len;
                }
                // Point as a marker would move; LEAVE-MARKERS keeps its character.
                b.point = byte(b, moved(point)).map_err(Fault::Error)?;
            }
            data::Object::ByteBuffer(b) => {
                let [b1, f1, b2, f2] = span.map(|c| (c - 1) as usize);
                let swapped = [&b.bytes[b2..f2], &b.bytes[f1..b2], &b.bytes[b1..f1]].concat();
                for (i, &byte) in swapped.iter().enumerate() { b.set_byte(b1 + i, byte).map_err(Fault::Error)?; }
                let at = |p: usize| (p + 1) as i64;
                let moved_properties: Vec<_> = b.properties.range(ByteOffset(b1)..ByteOffset(f2))
                    .map(|(&p, v)| (ByteOffset((transposed(at(p.0), [at(b1), at(f1), at(b2), at(f2)]) - 1) as usize), v.clone())).collect();
                b.properties.retain(|p, _| !(b1..f2).contains(&p.0));
                b.properties.extend(moved_properties);
                b.point = ByteOffset((moved(b.point.0 as i64 + 1) - 1) as usize);
            }
            _ => return Err(Fault::Error("buffer required".into())),
        }
        self.transpose_place(index, places, moved)?;
        self.vm.display_state.version += 1;
        Ok(())
    }

    /// set_properties' undo records for clearing FROM..TO's properties:
    /// (nil PROP VALUE BEG . END) for each property of each run, in order.
    fn record_properties_cleared(&mut self, index: usize, from: i64, to: i64) -> std::result::Result<(), Fault> {
        if from == to || self.vm.undo_list_of(index) == Value::TRUE { return Ok(()); }
        let runs: Vec<(i64, i64, Vec<(String, Value)>)> = match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let range = b.byte_of_char(CharOffset((from - 1) as usize)).map_err(Fault::Error)?..b.byte_of_char(CharOffset((to - 1) as usize)).map_err(Fault::Error)?;
                let char_at = |p: ByteOffset| b.char_of_byte(p).map(|c| c.0 as i64 + 1);
                b.properties.runs(range).into_iter().filter(|(_, ps)| !ps.is_empty())
                    .map(|(r, ps)| Ok((char_at(r.start)?, char_at(r.end)?, ps))).collect::<crate::Result<_>>().map_err(Fault::Error)?
            }
            data::Object::ByteBuffer(b) => {
                let mut runs: Vec<(i64, i64, Vec<(String, Value)>)> = Vec::new();
                for at in (from - 1) as usize..(to - 1) as usize {
                    let ps: Vec<(String, Value)> = b.properties.get(&ByteOffset(at)).map(|m| m.iter().map(|(k, &v)| (k.clone(), v)).collect()).unwrap_or_default();
                    match runs.last_mut() {
                        Some(run) if run.1 == at as i64 + 1 && run.2 == ps => run.1 += 1,
                        _ => runs.push((at as i64 + 1, at as i64 + 2, ps)),
                    }
                }
                runs.retain(|(_, _, ps)| !ps.is_empty());
                runs
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        for (beg, end, properties) in runs {
            for (key, value) in properties {
                let prop = self.vm.property_key_symbol(&key);
                self.vm.record_property_change(beg, end, prop, value).map_err(Fault::Error)?;
            }
        }
        Ok(())
    }

    /// The markers, overlay ends and sharing buffers' views inside the
    /// span, at their characters before the swap.
    fn transpose_places(&mut self, index: usize, [s1, _, _, e2]: [i64; 4]) -> std::result::Result<Vec<Place>, Fault> {
        let family = match self.vm.text_family(index) { f if f.is_empty() => vec![index], f => f };
        let buffers: Vec<Value> = family.iter().map(|&i| Value::reference(i + 1, 0)).collect();
        let mut members: Vec<usize> = family.iter().flat_map(|&i| self.vm.buffer_referrers(i)).collect();
        members.sort_unstable();
        members.dedup();
        let char_at = |vm: &Vm, byte: u32| -> crate::Result<i64> {
            Ok(match &vm.objects[index] {
                data::Object::Buffer(b) => b.char_of_byte(ByteOffset(byte as usize))?.0 as i64 + 1,
                _ => byte as i64 + 1,
            })
        };
        let resolve = |vm: &Vm, anchor: Anchor| match &vm.objects[index] {
            data::Object::Buffer(b) => b.anchors.resolve(anchor),
            data::Object::ByteBuffer(b) => b.anchors.resolve(anchor),
            _ => None,
        };
        let inside = |c: i64| c > s1 && c < e2;
        let mut places = Vec::new();
        for i in members {
            match &self.vm.objects[i] {
                data::Object::Marker(m) if buffers.contains(&m.buffer) => {
                    let Some(anchor) = m.anchor else { continue };
                    let Some(byte) = resolve(self.vm, anchor) else { continue };
                    let at = char_at(self.vm, byte).map_err(Fault::Error)?;
                    if at >= s1 && at < e2 { places.push(Place::Marker(anchor, m.bias, at)); }
                }
                data::Object::Overlay(o) if buffers.contains(&o.buffer) => {
                    let (Some(a), Some(z)) = (o.start, o.end) else { continue };
                    let (Some(a), Some(z)) = (resolve(self.vm, a), resolve(self.vm, z)) else { continue };
                    let (a, z) = (char_at(self.vm, a).map_err(Fault::Error)?, char_at(self.vm, z).map_err(Fault::Error)?);
                    if inside(a) || inside(z) { places.push(Place::Overlay(o.buffer, Value::reference(i + 1, 0), a, z)); }
                }
                _ => {}
            }
        }
        for &other in family.iter().filter(|&&i| i != index) {
            let data::Object::Buffer(b) = &self.vm.objects[other] else { continue };
            let r = b.accessible();
            let view = [r.start, b.point, r.end].map(|p| b.char_of_byte(p).map(|c| c.0 as i64 + 1));
            let [begv, pt, zv] = view.into_iter().collect::<crate::Result<Vec<_>>>().map_err(Fault::Error)?[..] else { unreachable!() };
            if [begv, pt, zv].iter().any(|&c| c >= s1 && c < e2) { places.push(Place::View(other, [begv, pt, zv])); }
        }
        Ok(places)
    }

    /// PLACES at their characters after the swap: markers and sharing
    /// buffers' views moved by MOVED, overlays where they were.
    fn transpose_place(&mut self, index: usize, places: Vec<Place>, moved: impl Fn(i64) -> i64) -> std::result::Result<(), Fault> {
        let byte = |vm: &Vm, c: i64| -> crate::Result<u32> {
            Ok(match &vm.objects[index] {
                data::Object::Buffer(b) => b.byte_of_char(CharOffset((c - 1) as usize))?.0 as u32,
                _ => (c - 1) as u32,
            })
        };
        for place in places {
            match place {
                Place::Marker(anchor, bias, at) => {
                    let to = byte(self.vm, moved(at)).map_err(Fault::Error)?;
                    match &mut self.vm.objects[index] {
                        data::Object::Buffer(b) => { b.anchors.set(anchor, to, bias); }
                        data::Object::ByteBuffer(b) => { b.anchors.set(anchor, to, bias); }
                        _ => {}
                    }
                }
                Place::Overlay(buffer, overlay, a, z) => {
                    let (a, z) = (byte(self.vm, a).map_err(Fault::Error)?, byte(self.vm, z).map_err(Fault::Error)?);
                    let Some(data::Object::Overlay(o)) = self.vm.object_if(overlay) else { continue };
                    let (Some(start), Some(end), front, rear) = (o.start, o.end, o.front, o.rear) else { continue };
                    let target = self.vm.annotated_mut(buffer).map_err(Fault::Error)?;
                    target.intervals.remove(start, &target.anchors);
                    target.anchors.set(start, a, front);
                    target.anchors.set(end, z, rear);
                    target.intervals.insert(overlay, start, end, &target.anchors);
                }
                Place::View(other, [begv, pt, zv]) => {
                    let [begv, pt, zv] = [begv, pt, zv].map(|c| byte(self.vm, moved(c)).map(|b| ByteOffset(b as usize)));
                    let (begv, pt, zv) = (begv.map_err(Fault::Error)?, pt.map_err(Fault::Error)?, zv.map_err(Fault::Error)?);
                    // Moved apart, they still make a view: point inside its
                    // narrowing.  GNU fetches the three markers as they are,
                    // which can leave BEGV after ZV; Romacs keeps a view.
                    let (begv, zv) = (begv.min(zv), begv.max(zv));
                    if let data::Object::Buffer(b) = &mut self.vm.objects[other] { b.set_view(begv..zv, pt.clamp(begv, zv)); }
                }
            }
        }
        Ok(())
    }
}
