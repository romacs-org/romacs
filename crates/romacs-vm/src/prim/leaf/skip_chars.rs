//! Non-collecting character-set motion in current-buffer character coordinates.
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

struct CharacterSet {
    negate: bool,
    ranges: Vec<(u32, u32)>,
    multibyte: bool,
}

impl CharacterSet {
    fn parse(string: crate::lstring::OwnedStr) -> std::result::Result<Self, Fault> {
        let chars: Vec<u32> = string.chars().collect();
        let negate = chars.first() == Some(&(b'^' as u32));
        let mut at = usize::from(negate);
        let mut ranges = Vec::new();
        while at < chars.len() {
            if chars[at..].starts_with(&[b'[' as u32, b':' as u32])
                && chars[at + 2..].windows(2).any(|w| w == [b':' as u32, b']' as u32])
            {
                return Err(Fault::Error("skip-chars: ISO C character classes are not implemented".into()));
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
        Ok(Self { negate, ranges, multibyte: string.is_multibyte() })
    }

    fn contains(&self, code: u32, buffer_multibyte: bool) -> bool {
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
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let distance = match &mut self.vm.objects[index] {
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
                    if !set.contains(code as u32, true) { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                buffer.point = buffer.byte_of_char(CharOffset(at)).map_err(Fault::Error)?;
                at as i64 - start as i64
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
                    if !set.contains(buffer.bytes[character] as u32, false) { break; }
                    if forward { at += 1; } else { at -= 1; }
                }
                buffer.point = ByteOffset(at);
                at as i64 - start as i64
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        Value::integer(distance).map_err(Fault::Error)
    }
}
