//! Non-collecting word motion through the ordinary current syntax table.
//!
//! Scope: syntax word runs, including Unicode and syntax text properties,
//! constrained to point's field as syntax.c Fforward_word constrains them.
//! GNU words-include-escapes and script/category transitions are not
//! implemented here. A find-word-boundary-function-table with entries takes
//! `forward-word' and the case commands to lisp/romacs/vm/prelude.el's scan.
//! The regex default boundary lookup is also a no-op, not a script provider.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};
use romacs_regex::{SyntaxClass, SyntaxLookup};

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
        let origin = Value::integer(self.point()?).map_err(Fault::Error)?;
        let (reached, complete) = self.scan_words(count)?;
        self.goto_char_position(reached)?;
        // Avoid jumping out of an input field.
        let reached = Value::integer(reached).map_err(Fault::Error)?;
        let kept = self.constrain_to_field(reached, origin, Value::NIL, Value::NIL, Value::NIL)?;
        if kept != reached {
            self.goto_char_position(kept.integer_value().expect("a position"))?;
        }
        Ok(if complete && kept == reached { Value::TRUE } else { Value::NIL })
    }

    /// Whether `find-word-boundary-function-table' has any entry.
    pub(crate) fn word_boundary_functions(&mut self) -> std::result::Result<bool, Fault> {
        let name = self.intern("find-word-boundary-function-table");
        let table = self.variable_value(name)?.unwrap_or(Value::NIL);
        Ok(matches!(self.vm.object(table), Ok(data::Object::CharTable(table)) if !table.is_empty()))
    }

    /// syntax.c scan_words from point: the position COUNT words away
    /// (back for a negative COUNT), and whether all COUNT were crossed;
    /// where an edge of the accessible text comes first, that edge's
    /// position with false. Point does not move.
    pub(crate) fn scan_words(&mut self, count: i64) -> std::result::Result<(i64, bool), Fault> {
        let source = self.vm.word_motion_syntax()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let syntax = crate::regexp::VmSyntax::new(self.vm, source);
        // Byte offsets stay relative to the accessible region, matching the
        // syntax properties. Characters are read where they lie, from
        // point on, so a motion costs the text it crosses.
        let (low, start, end) = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                let r = buffer.accessible();
                (r.start.0, buffer.point.0 - r.start.0, r.end.0 - r.start.0)
            }
            data::Object::ByteBuffer(buffer) => {
                let r = &buffer.restriction;
                (r.start.0, buffer.point.0 - r.start.0, r.end.0 - r.start.0)
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        let objects = &self.vm.objects;
        // The code of the character at OFFSET, a raw byte's as itself, and
        // the offset after it.
        let after = |offset: usize| -> (u32, usize) {
            match &objects[index] {
                data::Object::Buffer(buffer) => {
                    let at = low + offset;
                    let code = buffer.character_at(ByteOffset(at)).ok().flatten().map_or(0, |code| code as u32);
                    (code, offset + crate::lstring::char_string(code, &mut [0; 5]))
                }
                data::Object::ByteBuffer(buffer) => (u32::from(buffer.bytes[low + offset]), offset + 1),
                _ => unreachable!("buffer_index validates"),
            }
        };
        let before = |offset: usize| -> usize {
            match &objects[index] {
                data::Object::Buffer(buffer) => buffer.char_start_before(low + offset) - low,
                _ => offset - 1,
            }
        };
        let word = |offset: usize| syntax.code_syntax_at(after(offset).0, offset) == SyntaxClass::Word;
        let next = |offset: usize| after(offset).1;
        let mut at = start;
        let mut remaining = count.unsigned_abs();
        let forward = count > 0;
        while remaining != 0 {
            if forward {
                while at < end && !word(at) { at = next(at); }
                if at == end { break; }
                at = next(at);
                while at < end && word(at) { at = next(at); }
            } else {
                while at > 0 && !word(before(at)) { at = before(at); }
                if at == 0 { break; }
                at = before(at);
                while at > 0 && word(before(at)) { at = before(at); }
            }
            remaining -= 1;
        }
        drop(syntax);
        let position = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => buffer.char_of_byte(ByteOffset(low + at)).map_err(Fault::Error)?.0 as i64 + 1,
            _ => (low + at) as i64 + 1,
        };
        Ok((position, remaining == 0))
    }

    /// syntax.c skip_syntaxes: move point across characters whose syntax
    /// class is (or with a leading ^ is not) named in SYNTAX, up to LIMIT.
    pub fn skip_syntax(&mut self, syntax: Value, limit: Value, forward: bool) -> super::super::Out {
        // A multibyte spec is made unibyte first (string_make_unibyte): the
        // low byte of each character, read where the string lies.
        // The classes named, a bit each, and whether a leading ^ has the
        // skip pass every other class instead.
        let (named, negate) = {
            let spec = self.vm.str_view(syntax).ok_or(Fault::Wrong("stringp", syntax))?;
            let (mut named, mut negate, mut first) = (0_u32, false, true);
            for c in spec.chars() {
                let designator = c as u8;
                if std::mem::take(&mut first) && designator == b'^' {
                    negate = true;
                } else if let Some(class) = syntax_spec_code(designator) {
                    named |= 1 << class;
                }
            }
            (named, negate)
        };
        let skipped = |class: SyntaxClass| ((named >> (class as u32)) & 1 == 1) != negate;
        let limit = if limit == Value::NIL { None } else if let Some(n) = limit.integer_value() { Some(n) } else {
            match self.marker_position(limit) {
                Some(Some(n)) => Some(n),
                Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
                None => return Err(Fault::Wrong("integer-or-marker-p", limit)),
            }
        };
        let source = self.vm.word_motion_syntax()?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let syntax = crate::regexp::VmSyntax::new(self.vm, source);
        // Walked from point a character at a time, in bytes: nothing is
        // counted but the characters crossed, and only a LIMIT, a
        // position Lisp gave, is converted. The syntax lookup takes byte
        // offsets relative to the accessible start.
        let (moved, point) = match &self.vm.objects[index] {
            data::Object::Buffer(buffer) => {
                let accessible = buffer.accessible();
                let (low, high) = (accessible.start.0, accessible.end.0);
                let bound = match limit {
                    None => if forward { high } else { low },
                    Some(limit) => {
                        let before = usize::try_from(limit - 1).unwrap_or(0);
                        let byte = if before < buffer.len_chars() { buffer.byte_of_char(CharOffset(before)).map_err(Fault::Error)?.0 } else { buffer.len_bytes() };
                        byte.clamp(low, high)
                    }
                };
                let class_at = |byte: usize| -> std::result::Result<Option<(u32, bool)>, Fault> {
                    let code = buffer.character_at(ByteOffset(byte)).map_err(Fault::Error)?;
                    Ok(code.map(|code| (code as u32, skipped(syntax.code_syntax_at(code as u32, byte - low)))))
                };
                let (mut at, mut moved) = (buffer.point.0, 0_i64);
                if forward {
                    while at < bound {
                        let Some((code, true)) = class_at(at)? else { break };
                        at += crate::lstring::char_string(code, &mut [0; 5]);
                        moved += 1;
                    }
                } else {
                    while at > bound {
                        let before = buffer.char_start_before(at);
                        let Some((_, true)) = class_at(before)? else { break };
                        at = before;
                        moved -= 1;
                    }
                }
                (moved, ByteOffset(at))
            }
            data::Object::ByteBuffer(buffer) => {
                let (low, high) = (buffer.restriction.start.0, buffer.restriction.end.0);
                let start = buffer.point.0;
                let bound = limit.unwrap_or(if forward { high as i64 + 1 } else { low as i64 + 1 })
                    .clamp(low as i64 + 1, high as i64 + 1) as usize - 1;
                let mut at = start;
                while if forward { at < bound } else { at > bound } {
                    let character = if forward { at } else { at - 1 };
                    if !skipped(syntax.char_syntax_at(char::from(buffer.bytes[character]), character - low)) { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                (at as i64 - start as i64, ByteOffset(at))
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        drop(syntax);
        match self.vm.objects.entry_mut(index) {
            data::Object::Buffer(buffer) => buffer.point = point,
            data::Object::ByteBuffer(buffer) => buffer.point = point,
            _ => unreachable!("matched above"),
        }
        Value::integer(moved).map_err(Fault::Error)
    }
}

/// syntax.c `syntax_spec_code`: a designator's class, if it names one.
fn syntax_spec_code(c: u8) -> Option<u32> {
    Some(match c {
        b' ' | b'-' => 0, b'.' => 1, b'w' => 2, b'_' => 3, b'(' => 4, b')' => 5,
        b'\'' => 6, b'"' => 7, b'$' => 8, b'\\' => 9, b'/' => 10, b'<' => 11,
        b'>' => 12, b'@' => 13, b'!' => 14, b'|' => 15, _ => return None,
    })
}
