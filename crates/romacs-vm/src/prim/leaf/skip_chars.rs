//! Non-collecting character-set motion in current-buffer character coordinates.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

struct CharacterSet {
    negate: bool,
    ranges: Vec<(u32, u32)>,
    /// The ISO C `[:NAME:]` classes, as the regex engine's class bits.
    classes: u32,
    multibyte: bool,
}

/// re_wctype_parse at CHARS: the class bits of a `[:NAME:]` there and the
/// characters it spans, or None when no `:]' closes a `[:'.
fn iso_class(chars: &[u32]) -> Option<std::result::Result<(u32, usize), Fault>> {
    if chars.len() < 4 || chars[..2] != [b'[' as u32, b':' as u32] { return None; }
    let end = 2 + chars[2..].windows(2).position(|w| w == [b':' as u32, b']' as u32])?;
    let name: String = chars[2..end].iter().filter_map(|&c| char::from_u32(c)).collect();
    Some(romacs_regex_engine::char_class_bit(&name)
        .map(|bit| (bit, end + 2))
        .ok_or_else(|| Fault::Error("Invalid ISO C character class".into())))
}

impl CharacterSet {
    fn parse(string: crate::lstring::OwnedStr) -> std::result::Result<Self, Fault> {
        let chars: Vec<u32> = string.chars().collect();
        let negate = chars.first() == Some(&(b'^' as u32));
        let mut at = usize::from(negate);
        let mut ranges = Vec::new();
        let mut classes = 0;
        while at < chars.len() {
            if let Some(class) = iso_class(&chars[at..]) {
                let (bit, used) = class?;
                classes |= bit;
                at += used;
                continue;
            }
            let mut start = chars[at];
            at += 1;
            if start == b'\\' as u32 {
                if at == chars.len() { break; }
                start = chars[at];
                at += 1;
            }
            let mut end = start;
            if at + 1 < chars.len() && chars[at] == b'-' as u32 {
                end = chars[at + 1];
                at += 2;
                if end == b'\\' as u32 && at < chars.len() {
                    end = chars[at];
                    at += 1;
                }
            }
            if start <= end { ranges.push((start, end)); }
        }
        Ok(Self { negate, ranges, classes, multibyte: string.is_multibyte() })
    }

    /// SYNTAX is the buffer's table, present when the set has classes.
    fn contains(&self, code: u32, buffer_multibyte: bool, syntax: Option<&dyn romacs_regex_engine::SyntaxLookup>) -> bool {
        if let Some(syntax) = syntax && romacs_regex_engine::char_in_classes(code, self.classes, syntax) {
            return !self.negate;
        }
        let comparable = if self.multibyte == buffer_multibyte || code < 128 {
            Some(code)
        } else if buffer_multibyte {
            code.checked_sub(crate::lstring::BYTE8_OFFSET).filter(|&c| (128..=255).contains(&c))
        } else {
            Some(code + crate::lstring::BYTE8_OFFSET)
        };
        comparable.is_some_and(|c| self.ranges.iter().any(|&(a, b)| a <= c && c <= b)) != self.negate
    }
}

impl Leaf<'_> {
    pub fn skip_characters(&mut self, string: Value, limit: Value, forward: bool) -> super::super::Out {
        let string = self.string_view(string)?;
        let limit = if limit == Value::NIL { None } else if let Some(n) = self.bigint_value(limit) {
            Some(i64::try_from(&n).unwrap_or(if n.sign() == num_bigint::Sign::Minus { i64::MIN } else { i64::MAX }))
        } else {
            Some(match self.marker_position(limit) {
                Some(Some(n)) => n,
                Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
                None => return Err(Fault::Wrong("integer-or-marker-p", limit)),
            })
        };
        let set = CharacterSet::parse(string)?;
        let source = if set.classes != 0 { Some(self.vm.word_motion_syntax()?) } else { None };
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let syntax = source.map(|source| crate::regexp::VmSyntax::new(self.vm, source));
        let syntax = syntax.as_ref().map(|s| s as &dyn romacs_regex_engine::SyntaxLookup);
        let (start, at) = match &self.vm.objects[index] {
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
                    let Some(code) = buffer.character_of_char(CharOffset(character)).map_err(Fault::Error)? else { break; };
                    if !set.contains(code as u32, true, syntax) { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                (start, at)
            }
            data::Object::ByteBuffer(buffer) => {
                let low = buffer.restriction.start.0;
                let high = buffer.restriction.end.0;
                let start = buffer.point.0;
                let bound = limit.unwrap_or(if forward { high as i64 + 1 } else { low as i64 + 1 })
                    .clamp(low as i64 + 1, high as i64 + 1) as usize - 1;
                let mut at = start;
                while if forward { at < bound } else { at > bound } {
                    let character = if forward { at } else { at - 1 };
                    if !set.contains(buffer.bytes[character] as u32, false, syntax) { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                (start, at)
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        match &mut self.vm.objects[index] {
            data::Object::Buffer(buffer) => buffer.point = buffer.byte_of_char(CharOffset(at)).map_err(Fault::Error)?,
            data::Object::ByteBuffer(buffer) => buffer.point = ByteOffset(at),
            _ => unreachable!(),
        }
        Value::integer(at as i64 - start as i64).map_err(Fault::Error)
    }
}
