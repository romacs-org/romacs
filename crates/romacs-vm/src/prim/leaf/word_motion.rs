//! Non-collecting word motion through the ordinary current syntax table.
//!
//! Scope: syntax word runs, including Unicode and syntax text properties.
//! GNU field constraints, words-include-escapes, script/category transitions,
//! and find-word-boundary-function-table callbacks are not implemented here.
//! The regex default boundary lookup is also a no-op, not a script provider.
use super::*;
use crate::coordinates::ByteOffset;
use romacs_regex_engine::{SyntaxClass, SyntaxLookup};

impl Leaf<'_> {
    pub fn move_words(&mut self, count: Value) -> super::super::Out {
        let count = if count == Value::NIL {
            1
        } else {
            count.integer_value().ok_or(Fault::Wrong("fixnump", count))?
        };
        if count == 0 {
            return Ok(Value::TRUE);
        }
        let syntax = self.vm.word_motion_syntax()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        // Byte offsets stay relative to the accessible region, matching the
        // syntax snapshot. No buffer mutation or moving-GC value is retained.
        let (characters, start, end): (Vec<(usize, char)>, usize, usize) =
            match &self.vm.objects[index] {
                data::Object::Buffer(buffer) => {
                    let r = buffer.accessible();
                    (
                        buffer.as_str()[r.start.0..r.end.0].char_indices().collect(),
                        buffer.point.0 - r.start.0,
                        r.end.0 - r.start.0,
                    )
                }
                data::Object::ByteBuffer(buffer) => {
                    let r = &buffer.restriction;
                    (
                        buffer.bytes[r.start.0..r.end.0]
                            .iter().enumerate().map(|(i, &c)| (i, char::from(c))).collect(),
                        buffer.point.0 - r.start.0,
                        r.end.0 - r.start.0,
                    )
                }
                _ => return Err(Fault::Error("buffer required".into())),
            };
        let word = |i: usize| {
            let (offset, ch) = characters[i];
            syntax.char_syntax_at(ch, offset) == SyntaxClass::Word
        };
        let mut at = characters.partition_point(|&(offset, _)| offset < start);
        let mut remaining = count.unsigned_abs();
        let forward = count > 0;
        while remaining != 0 {
            if forward {
                while at < characters.len() && !word(at) { at += 1; }
                if at == characters.len() { break; }
                at += 1;
                while at < characters.len() && word(at) { at += 1; }
            } else {
                while at > 0 && !word(at - 1) { at -= 1; }
                if at == 0 { break; }
                at -= 1;
                while at > 0 && word(at - 1) { at -= 1; }
            }
            remaining -= 1;
        }
        let offset = characters.get(at).map_or(end, |&(offset, _)| offset);
        match &mut self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                buffer.point = ByteOffset(buffer.accessible().start.0 + offset);
            }
            data::Object::ByteBuffer(buffer) => {
                buffer.point = ByteOffset(buffer.restriction.start.0 + offset);
            }
            _ => unreachable!("buffer_index validates"),
        }
        Ok(if remaining == 0 { Value::TRUE } else { Value::NIL })
    }
}
