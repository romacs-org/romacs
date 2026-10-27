//! editfns.c Ftranslate_region_internal over a multibyte buffer. Like the
//! other text primitives it runs no change hooks and records no undo
//! (DIVERGENCES.md VM2-EDIT-HOOKS1).
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

/// Fconcat's and Fmake_string's check on a replacement character.
fn character(v: Value) -> std::result::Result<u32, Fault> {
    v.integer_value().filter(|&n| (0..=crate::lstring::MAX_CHAR as i64).contains(&n))
        .map(|n| n as u32).ok_or(Fault::Wrong("characterp", v))
}

/// What a table says to put in place of one character.
enum Mapping {
    Same,
    /// One character for one.
    Char(u32),
    /// A char-table vector value: these characters for one.
    Many(Vec<u32>),
    /// A char-table alist value: ((FROM-VECTOR . TO) ...).
    Sequences(Value),
}

impl Leaf<'_> {
    fn translate_char(&mut self, table: Value, string: Option<&[u32]>, c: u32) -> std::result::Result<Mapping, Fault> {
        if let Some(chars) = string {
            return Ok(match chars.get(c as usize) { Some(&n) if n != c => Mapping::Char(n), _ => Mapping::Same });
        }
        let value = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
        if let Some(n) = value.integer_value().filter(|&n| (0..=crate::lstring::MAX_CHAR as i64).contains(&n)) {
            return Ok(if n as u32 == c { Mapping::Same } else { Mapping::Char(n as u32) });
        }
        if let Ok(items) = self.vector_values(value) {
            return Ok(Mapping::Many(items.into_iter().map(character).collect::<std::result::Result<_, _>>()?));
        }
        Ok(if self.pair(value).is_some() { Mapping::Sequences(value) } else { Mapping::Same })
    }

    /// check_translation: the first ((FROM ...) . TO) whose FROM characters
    /// are the buffer's at AT (all before END); its length and TO.
    fn translation_match(&mut self, alist: Value, chars: &[u32], at: usize, end: usize) -> std::result::Result<Option<(usize, Value)>, Fault> {
        let mut tail = alist;
        while let Some((entry, rest)) = self.pair(tail) {
            tail = rest;
            let Some((from, to)) = self.pair(entry) else { continue };
            let Ok(from) = self.vector_values(from) else { continue };
            if from.len() <= end - at
                && from.iter().zip(&chars[at..]).all(|(f, &c)| f.integer_value() == Some(i64::from(c))) {
                return Ok(Some((from.len(), to)));
            }
        }
        Ok(None)
    }

    pub fn translate_region_internal(&mut self, start: Value, end: Value, table: Value) -> super::super::Out {
        let position = |leaf: &Self, v: Value| -> std::result::Result<i64, Fault> {
            match leaf.marker_position(v) {
                Some(Some(n)) => Ok(n),
                Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
                None => v.integer_value().ok_or(Fault::Wrong("integer-or-marker-p", v)),
            }
        };
        let (mut lo, mut hi) = (position(self, start)?, position(self, end)?);
        if lo > hi { std::mem::swap(&mut lo, &mut hi); }
        let string = if let Ok(view) = self.string_view(table) {
            // A unibyte table's non-ASCII bytes are raw bytes (BYTE8_STRING).
            let raw = !view.is_multibyte();
            Some(view.chars().map(|c| if raw && c >= 128 { crate::lstring::BYTE8_OFFSET + c } else { c }).collect::<Vec<u32>>())
        } else {
            let purpose = self.intern("translation-table");
            let valid = matches!(self.vm.object_if(table), Some(data::Object::CharTable(t)) if t.purpose == purpose);
            if !valid { return Err(Fault::Error("Not a translation table".into())); }
            None
        };
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let buffer = self.vm.slots[self.vm.buffer_slot].value;
        let data::Object::Buffer(b) = &self.vm.objects[index] else {
            return Err(Fault::Error("VM2 translate-region-internal in a unibyte buffer is unsupported".into()));
        };
        let accessible = (b.char_of_byte(b.accessible().start).map_err(Fault::Error)?.0 as i64 + 1,
                          b.char_of_byte(b.accessible().end).map_err(Fault::Error)?.0 as i64 + 1);
        // validate_region: args_out_of_range_3 (current_buffer, START, END).
        if lo < accessible.0 || hi > accessible.1 {
            let data = self.cons(end, Value::NIL);
            let data = self.cons(start, data);
            let data = self.cons(buffer, data);
            return Err(Fault::Signal(self.intern("args-out-of-range"), data));
        }
        // modify_text: prepare_to_modify_buffer's read-only checks.
        let range = {
            let data::Object::Buffer(b) = &self.vm.objects[index] else { unreachable!() };
            b.byte_of_char(CharOffset((lo - 1) as usize)).map_err(Fault::Error)?..b.byte_of_char(CharOffset((hi - 1) as usize)).map_err(Fault::Error)?
        };
        // barf_if_buffer_read_only, then verify_interval_modification: an
        // empty region is checked as an insertion point.
        let at = range.start;
        let empty = range.is_empty();
        self.vm.check_text_property_edit(buffer, range, false)?;
        if empty {
            let inhibit = self.intern("inhibit-read-only");
            let inhibit = self.variable_value(inhibit)?.unwrap_or(Value::NIL);
            self.check_insertion_read_only(at, inhibit)?;
        }
        // modify_text: the region counts as modified whatever changes.
        if let data::Object::Buffer(b) = &mut self.vm.objects[index] {
            b.state.modification.changed((hi - lo) as usize, true);
        }
        let mut chars: Vec<u32> = {
            let data::Object::Buffer(b) = &self.vm.objects[index] else { unreachable!() };
            ((lo - 1) as usize..(hi - 1) as usize)
                .map(|at| b.character_of_char(CharOffset(at)).map(|c| c.map_or(0, |c| c as u32)))
                .collect::<crate::Result<_>>().map_err(Fault::Error)?
        };
        let (base, mut at, mut changed) = ((lo - 1) as usize, 0, 0);
        while at < chars.len() {
            let (len, replacement, scalar) = match self.translate_char(table, string.as_deref(), chars[at])? {
                Mapping::Same => { at += 1; continue; }
                Mapping::Char(c) => (1, vec![c], true),
                Mapping::Many(cs) => (1, cs, false),
                Mapping::Sequences(alist) => match self.translation_match(alist, &chars, at, chars.len())? {
                    None => { at += 1; continue; }
                    Some((len, to)) => {
                        let to = match self.vector_values(to) {
                            Ok(items) => items.into_iter().map(character).collect::<std::result::Result<_, _>>()?,
                            Err(_) => vec![character(to)?],
                        };
                        (len, to, false)
                    }
                },
            };
            // A VM2 multibyte buffer holds Unicode text, so no raw byte.
            if replacement.iter().any(|&c| c > 0x10ffff) {
                return Err(Fault::Error("VM2 translate-region-internal cannot put a raw byte in a multibyte buffer".into()));
            }
            let mut bytes = Vec::new();
            for &c in &replacement {
                let mut encoded = [0; 5];
                let n = crate::lstring::char_string(c, &mut encoded);
                bytes.extend_from_slice(&encoded[..n]);
            }
            let data::Object::Buffer(b) = &mut self.vm.objects[index] else { unreachable!() };
            let from = b.byte_of_char(CharOffset(base + at)).map_err(Fault::Error)?;
            let to = b.byte_of_char(CharOffset(base + at + len)).map_err(Fault::Error)?;
            if scalar && to.0 - from.0 == bytes.len() {
                // One character for one of the same bytes: rewritten in
                // place, properties, point and markers kept.
                let text = std::str::from_utf8(&bytes).map_err(|e| Fault::Error(e.to_string()))?.to_owned();
                b.overwrite(from..ByteOffset(from.0 + bytes.len()), &text).map_err(Fault::Error)?;
            } else {
                // replace_range: markers at or after the old end follow the
                // new text, markers inside collapse to its start, and point
                // inside or at the old end ends up after it.
                let pt = b.char_of_byte(b.point).map_err(Fault::Error)?.0;
                let (start, old_end) = (base + at, base + at + len);
                b.replace_with_bias(to..to, &bytes, true).map_err(Fault::Error)?;
                b.replace_with_bias(from..to, &[], false).map_err(Fault::Error)?;
                let new_end = start + replacement.len();
                let new_pt = if pt >= old_end { pt + replacement.len() - len } else if pt > start { new_end } else { pt };
                b.point = b.byte_of_char(CharOffset(new_pt)).map_err(Fault::Error)?;
            }
            chars.splice(at..at + len, replacement.iter().copied());
            changed += if len == 1 && replacement.len() == 1 { 1 } else { replacement.len() };
            at += replacement.len();
        }
        self.vm.finish_annotation_edit(buffer).map_err(Fault::Error)?;
        Ok(Value::integer(changed as i64).unwrap())
    }
}
