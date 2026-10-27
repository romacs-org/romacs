//! Non-collecting word motion through the ordinary current syntax table.
//!
//! Scope: syntax word runs, including Unicode and syntax text properties.
//! GNU field constraints, words-include-escapes, script/category transitions,
//! and find-word-boundary-function-table callbacks are not implemented here.
//! The regex default boundary lookup is also a no-op, not a script provider.
use super::*;
use crate::prim::leaf::Kind;
use crate::coordinates::{ByteOffset, CharOffset};
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
        let source = self.vm.word_motion_syntax()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let syntax = crate::regexp::VmSyntax::new(self.vm, source);
        // Byte offsets stay relative to the accessible region, matching the
        // syntax properties.
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
        drop(syntax);
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

    /// syntax.c skip_syntaxes: move point across characters whose syntax
    /// class is (or with a leading ^ is not) named in SYNTAX, up to LIMIT.
    pub fn skip_syntax(&mut self, syntax: Value, limit: Value, forward: bool) -> super::super::Out {
        if !matches!(self.kind(syntax), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", syntax)); }
        // A multibyte spec is made unibyte first (string_make_unibyte).
        let spec: Vec<u8> = self.string_view(syntax)?.chars().map(|c| c as u8).collect();
        let limit = if limit == Value::NIL { None } else if let Some(n) = limit.integer_value() { Some(n) } else {
            match self.marker_position(limit) {
                Some(Some(n)) => Some(n),
                Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
                None => return Err(Fault::Wrong("integer-or-marker-p", limit)),
            }
        };
        let negate = spec.first() == Some(&b'^');
        let mut classes = [negate; 256];
        for &c in &spec[usize::from(negate)..] { classes[syntax_spec_code(c)] = !negate; }
        let source = self.vm.word_motion_syntax()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let syntax = crate::regexp::VmSyntax::new(self.vm, source);
        // Character positions walked from point, as skip_chars does; the
        // syntax lookup takes byte offsets relative to the accessible start.
        let (start, at, point) = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                let accessible = buffer.accessible();
                let low = buffer.char_of_byte(accessible.start).map_err(Fault::Error)?.0;
                let high = buffer.char_of_byte(accessible.end).map_err(Fault::Error)?.0;
                let start = buffer.char_of_byte(buffer.point).map_err(Fault::Error)?.0;
                let bound = limit.unwrap_or(if forward { high as i64 + 1 } else { low as i64 + 1 })
                    .clamp(low as i64 + 1, high as i64 + 1) as usize - 1;
                let mut at = start;
                while if forward { at < bound } else { at > bound } {
                    let character = if forward { at } else { at - 1 };
                    let Some(ch) = buffer.character_of_char(CharOffset(character)).map_err(Fault::Error)? else { break; };
                    let offset = buffer.byte_of_char(CharOffset(character)).map_err(Fault::Error)?.0 - accessible.start.0;
                    if !classes[syntax.char_syntax_at(ch, offset) as usize] { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                (start, at, buffer.byte_of_char(CharOffset(at)).map_err(Fault::Error)?)
            }
            data::Object::ByteBuffer(buffer) => {
                let (low, high) = (buffer.restriction.start.0, buffer.restriction.end.0);
                let start = buffer.point.0;
                let bound = limit.unwrap_or(if forward { high as i64 + 1 } else { low as i64 + 1 })
                    .clamp(low as i64 + 1, high as i64 + 1) as usize - 1;
                let mut at = start;
                while if forward { at < bound } else { at > bound } {
                    let character = if forward { at } else { at - 1 };
                    if !classes[syntax.char_syntax_at(char::from(buffer.bytes[character]), character - low) as usize] { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                (start, at, ByteOffset(at))
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        drop(syntax);
        match &mut self.vm.objects[index] {
            data::Object::Buffer(buffer) => buffer.point = point,
            data::Object::ByteBuffer(buffer) => buffer.point = point,
            _ => unreachable!("matched above"),
        }
        Value::integer(at as i64 - start as i64).map_err(Fault::Error)
    }
}

/// syntax.c syntax_spec_code: a designator's class, 255 for none.
fn syntax_spec_code(c: u8) -> usize {
    match c {
        b' ' | b'-' => 0, b'.' => 1, b'w' => 2, b'_' => 3, b'(' => 4, b')' => 5,
        b'\'' => 6, b'"' => 7, b'$' => 8, b'\\' => 9, b'/' => 10, b'<' => 11,
        b'>' => 12, b'@' => 13, b'!' => 14, b'|' => 15, _ => 255,
    }
}
