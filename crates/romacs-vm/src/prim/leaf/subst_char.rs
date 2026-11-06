//! editfns.c Fsubst_char_in_region: same-width characters rewritten in
//! place inside modify_text's bracket (prims/subst_char.rs), each recorded
//! as record_change does unless NOUNDO.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

impl Leaf<'_> {
    /// validate_region and the characters' checks, then the character
    /// positions of START..END holding FROM, and END.
    pub fn subst_char_hits(&mut self, start: Value, end: Value, from: Value, to: Value) -> std::result::Result<(Vec<i64>, i64), Fault> {
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
        // Character positions holding FROM.
        let hits: Vec<usize> = match &self.vm.objects[index] {
            data::Object::Buffer(b) => {
                let mut hits = Vec::new();
                for at in (lo - 1) as usize..(hi - 1) as usize {
                    if b.character_of_char(CharOffset(at)).map_err(Fault::Error)? == Some(from_c) { hits.push(at); }
                }
                hits
            }
            data::Object::ByteBuffer(b) => ((lo - 1) as usize..(hi - 1) as usize).filter(|&at| u32::from(b.bytes[at]) == from_c).collect(),
            _ => unreachable!(),
        };
        Ok((hits.into_iter().map(|at| at as i64 + 1).collect(), hi))
    }

    /// modify_text's tail for a change of POS..END once its hooks ran: the
    /// first-change entry (none with NOUNDO, which turns undo off) and the
    /// modification count, which NOUNDO keeps from marking the buffer.
    pub fn modify_text_tail(&mut self, pos: i64, end: i64, noundo: bool) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let buffer = self.current_buffer()?;
        if !noundo && !self.vm.buffer_state(buffer).map_err(Fault::Error)?.modified() {
            self.vm.record_first_change().map_err(Fault::Error)?;
        }
        let state = match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => &mut b.state,
            data::Object::ByteBuffer(b) => &mut b.state,
            _ => unreachable!(),
        };
        state.record_text_change((end - pos) as usize);
        if noundo { keep_unmodified(&mut state.modification); }
        Ok(())
    }

    /// The rewrite of each character at HITS to TO, recorded as a deletion
    /// and an insertion (record_change) unless NOUNDO.
    pub fn subst_char_write(&mut self, hits: &[i64], to: Value, noundo: bool) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let to_c = to.integer_value().expect("checked character") as u32;
        let mut encoded = [0; 5];
        let width = crate::lstring::char_string(to_c, &mut encoded);
        let text = &encoded[..width];
        for &at in hits {
            if !noundo { self.vm.record_change(index, at).map_err(Fault::Error)?; }
            match &mut self.vm.objects[index] {
                data::Object::Buffer(b) => {
                    let byte = b.byte_of_char(CharOffset((at - 1) as usize)).map_err(Fault::Error)?;
                    b.overwrite(byte..ByteOffset(byte.0 + text.len()), text).map_err(Fault::Error)?;
                }
                data::Object::ByteBuffer(b) => b.set_byte((at - 1) as usize, to_c as u8).map_err(Fault::Error)?,
                _ => unreachable!(),
            }
        }
        self.vm.display_state.version += 1;
        Ok(())
    }
}

/// NOUNDO: a buffer unmodified before this change stays unmodified.
fn keep_unmodified(m: &mut crate::modification::Modification) {
    if m.save_modiff == m.modiff - 1 { m.save_modiff = m.modiff; }
    if m.autosave_modiff == m.modiff - 1 { m.autosave_modiff = m.modiff; }
}
