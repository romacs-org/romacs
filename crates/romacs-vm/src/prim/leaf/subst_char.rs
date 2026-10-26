//! editfns.c Fsubst_char_in_region: same-width characters rewritten in
//! place. VM2 records no undo entry and runs no change hooks for any text
//! change yet (DIVERGENCES.md VM2-EDIT-HOOKS1), so neither happens here;
//! the modification count and NOUNDO's save adjustment follow GNU.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

impl Leaf<'_> {
    pub fn subst_char_in_region(&mut self, start: Value, end: Value, from: Value, to: Value, noundo: Value) -> super::super::Out {
        let position = |leaf: &Self, v: Value| -> std::result::Result<i64, Fault> {
            match leaf.marker_position(v) {
                Some(Some(n)) => Ok(n),
                Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
                None => v.integer_value().ok_or(Fault::Wrong("integer-or-marker-p", v)),
            }
        };
        let (mut lo, mut hi) = (position(self, start)?, position(self, end)?);
        if lo > hi { std::mem::swap(&mut lo, &mut hi); }
        let character = |v: Value| v.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c))
            .map(|c| c as u32).ok_or(Fault::Wrong("characterp", v));
        let (from_c, to_c) = (character(from)?, character(to)?);
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let multibyte = matches!(self.vm.objects[index], data::Object::Buffer(_));
        let (begv, zv) = match &self.vm.objects[index] {
            data::Object::Buffer(b) => (b.char_of_byte(b.accessible().start).map_err(Fault::Error)?.0 as i64 + 1, b.char_of_byte(b.accessible().end).map_err(Fault::Error)?.0 as i64 + 1),
            data::Object::ByteBuffer(b) => (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
            _ => return Err(Fault::Error("buffer required".into())),
        };
        // validate_region: args_out_of_range_3 (current_buffer, START, END).
        if lo < begv || hi > zv {
            let buffer = self.vm.slots[self.vm.buffer_slot].value;
            let data = self.cons(end, Value::NIL);
            let data = self.cons(start, data);
            let data = self.cons(buffer, data);
            return Err(Fault::Signal(self.intern("args-out-of-range"), data));
        }
        if multibyte && crate::lstring::char_string(from_c, &mut [0; 5]) != crate::lstring::char_string(to_c, &mut [0; 5]) {
            return Err(Fault::Error("Characters in \u{2018}subst-char-in-region\u{2019} have different byte-lengths".into()));
        }
        let (from_ch, to_ch) = (char::from_u32(from_c), char::from_u32(to_c));
        // Character positions holding FROM.
        let hits: Vec<usize> = match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let mut hits = Vec::new();
                for at in (lo - 1) as usize..(hi - 1) as usize {
                    if from_ch.is_some() && b.character_of_char(CharOffset(at)).map_err(Fault::Error)? == from_ch { hits.push(at); }
                }
                hits
            }
            data::Object::ByteBuffer(b) => ((lo - 1) as usize..(hi - 1) as usize).filter(|&at| u32::from(b.bytes[at]) == from_c).collect(),
            _ => unreachable!(),
        };
        let Some(&first) = hits.first() else { return Ok(Value::NIL); };
        // prepare_to_modify_buffer: barf_if_buffer_read_only.
        let (read_only, inhibit) = (self.intern("buffer-read-only"), self.intern("inhibit-read-only"));
        if self.variable_value(read_only)?.is_some_and(|v| v != Value::NIL) && self.variable_value(inhibit)?.is_none_or(|v| v == Value::NIL) {
            let buffer = self.vm.slots[self.vm.buffer_slot].value;
            let data = self.cons(buffer, Value::NIL);
            return Err(Fault::Signal(self.intern("buffer-read-only"), data));
        }
        let changed = (hi - 1) as usize - first;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let text = to_ch.map(String::from).unwrap_or_default();
                for &at in &hits {
                    let byte = b.byte_of_char(CharOffset(at)).map_err(Fault::Error)?;
                    b.overwrite(byte..ByteOffset(byte.0 + text.len()), &text).map_err(Fault::Error)?;
                }
                b.state.record_text_change(changed);
                if noundo != Value::NIL { keep_unmodified(&mut b.state.modification); }
            }
            data::Object::ByteBuffer(b) => {
                for &at in &hits { b.bytes[at] = to_c as u8; }
                b.state.record_text_change(changed);
                if noundo != Value::NIL { keep_unmodified(&mut b.state.modification); }
            }
            _ => unreachable!(),
        }
        Ok(Value::NIL)
    }
}

/// NOUNDO: a buffer unmodified before this change stays unmodified.
fn keep_unmodified(m: &mut crate::modification::Modification) {
    if m.save_modiff == m.modiff - 1 { m.save_modiff = m.modiff; }
    if m.autosave_modiff == m.modiff - 1 { m.autosave_modiff = m.modiff; }
}
