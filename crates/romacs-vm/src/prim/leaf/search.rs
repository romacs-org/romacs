//! GNU search.c:2344 string replacement.
use super::*;

impl Leaf<'_> {
    pub fn beginning_of_line(&mut self) -> std::result::Result<bool, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &self.vm.objects[index] {
            data::Object::Buffer(text) => {
                let point = text.point.0;
                Ok(point == text.accessible().start.0 || text.bytes().get(point.wrapping_sub(1)) == Some(&b'\n'))
            }
            data::Object::ByteBuffer(text) => {
                let point = text.point.0;
                Ok(point == text.restriction.start.0 || text.bytes.get(point.wrapping_sub(1)) == Some(&b'\n'))
            }
            _ => Err(Fault::Error("buffer required".into())),
        }
    }

    pub fn replace_match(&mut self, newtext: Value, fixedcase: Value, literal: Value, string: Value, subexp: Value) -> std::result::Result<Value, Fault> {
        self.string_view(newtext)?;
        if string != Value::NIL { self.string_view(string)?; }
        let groups = &self.vm.regexp.groups;
        if !self.vm.regexp.succeeded || groups.is_empty() {
            return Err(Fault::Error("`replace-match' called before any match found".into()));
        }
        let register_count = groups.len();
        let sub = if subexp == Value::NIL { 0 } else {
            let n = self.bigint_value(subexp).ok_or(Fault::Wrong("integerp", subexp))?;
            usize::try_from(n).ok().filter(|&n| n < register_count)
                .ok_or(Fault::Range3(subexp, Value::integer(0).unwrap(), Value::integer(register_count as i64 - 1).map_err(Fault::Error)?))?
        };
        let (start, end) = match groups.get(sub).copied().flatten() {
            Some(range) => range,
            None => {
                let condition = self.intern("error");
                let message = self.string("replace-match subexpression does not exist");
                let tail = self.cons(subexp, Value::NIL);
                let data = self.cons(message, tail);
                return Err(Fault::Signal(condition, data));
            }
        };
        if string == Value::NIL {
            return Err(Fault::Error("replace-match: buffer replacement is not implemented".into()));
        }
        let source = self.string_view(string)?;
        if start > end || end > source.chars {
            return Err(Fault::Range(Value::integer(start as i64).map_err(Fault::Error)?, Value::integer(end as i64).map_err(Fault::Error)?));
        }
        let mut case_action = 0;
        if fixedcase == Value::NIL && self.string_view(newtext)?.chars != 0 {
            let matched: Vec<u32> = source.chars().skip(start).take(end - start).collect();
            let defaults: Vec<u8> = matched.iter().map(|&c| {
                match romacs_regex_engine::standard_syntax_class_for_code(c) {
                    romacs_regex_engine::SyntaxClass::Word => 1,
                    romacs_regex_engine::SyntaxClass::Symbol => 2,
                    _ => 0,
                }
            }).collect();
            let words = self.casing_word_flags(&matched, &defaults)?;
            let mut previous_word = self.casing_word_flags(&[u32::from(b'\n')], &[0])?[0];
            let (mut lowercase, mut uppercase, mut multiletter, mut nonuppercase_initial) = (false, false, false, false);
            for (&code, word) in matched.iter().zip(words) {
                let code = if !source.multibyte && code >= 128 { code + crate::lstring::BYTE8_OFFSET } else { code };
                let upper = self.case_mapping_character(code, true)? != code;
                if !upper && self.case_mapping_character(code, false)? != code {
                    lowercase = true;
                    if previous_word { multiletter = true; } else { nonuppercase_initial = true; }
                } else if upper {
                    uppercase = true;
                    if previous_word { multiletter = true; }
                } else if !previous_word {
                    nonuppercase_initial = true;
                }
                previous_word = word;
            }
            case_action = if !lowercase && multiletter { 1 }
                else if !nonuppercase_initial && multiletter { 2 }
                else if !nonuppercase_initial && uppercase { 1 }
                else { 0 };
        }
        let groups = self.vm.regexp.groups.clone();
        let replacement = self.string_view(newtext)?;
        let literal = literal != Value::NIL || !replacement.bytes.contains(&b'\\');
        let mut pieces = vec![(string, 0, start)];
        if literal {
            pieces.push((newtext, 0, replacement.chars));
        } else {
            let mut chars = Vec::new();
            let mut bytes = replacement.bytes.as_slice();
            while !bytes.is_empty() {
                let (c, n) = if replacement.multibyte { crate::lstring::string_char(bytes) } else { (bytes[0] as u32, 1) };
                chars.push(c); bytes = &bytes[n..];
            }
            let mut i = 0;
            let mut last = 0;
            while i < chars.len() {
                if chars[i] != b'\\' as u32 { i += 1; continue; }
                let escape = i;
                i += 1;
                let c = chars.get(i).copied().ok_or_else(|| Fault::Error("Invalid use of `\\' in replacement text".into()))?;
                i += 1;
                let group = if c == b'&' as u32 { Some(Some((start, end))) }
                    else if (b'1' as u32..=b'9' as u32).contains(&c) { Some(groups.get((c - b'0' as u32) as usize).copied().flatten()) }
                    else { None };
                if let Some(group) = group {
                    pieces.push((newtext, last, escape));
                    if let Some((a, b)) = group { pieces.push((string, a, b)); }
                    last = i;
                } else if c == b'\\' as u32 {
                    pieces.push((newtext, last, i - 1)); last = i;
                } else if c != b'?' as u32 {
                    return Err(Fault::Error("Invalid use of `\\' in replacement text".into()));
                }
            }
            pieces.push((newtext, last, chars.len()));
        }
        let multibyte = source.multibyte || (replacement.multibyte && (literal || pieces.iter().any(|&(v, a, b)| v == newtext && a < b)));
        if case_action != 0 {
            let mut middle_multibyte = false;
            for &(value, a, b) in &pieces[1..] {
                middle_multibyte |= a < b && self.string_view(value)?.multibyte;
            }
            let middle = self.join_replacement_parts(&pieces[1..], middle_multibyte)?;
            let middle = if case_action == 1 { self.case_convert(middle, false)? }
                else { crate::prim::prims::capitalize::capitalized(self, middle, true)? };
            pieces.truncate(1);
            pieces.push((middle, 0, self.string_view(middle)?.chars));
        }
        pieces.push((string, end, source.chars));
        self.join_replacement_parts(&pieces, multibyte)
    }

    fn join_replacement_parts(&mut self, pieces: &[(Value, usize, usize)], multibyte: bool) -> std::result::Result<Value, Fault> {
        let mut output = data::LispString::from_bytes(Vec::new(), multibyte);
        let mut assembled = Vec::new();
        for &(value, a, b) in pieces {
            let view = self.string_view(value)?;
            if a > b || b > view.chars {
                return Err(Fault::Range(Value::integer(a as i64).map_err(Fault::Error)?, Value::integer(b as i64).map_err(Fault::Error)?));
            }
            let byte = |count: usize| {
                if !view.multibyte { return count; }
                let mut at = 0;
                for _ in 0..count { at += crate::lstring::string_char(&view.bytes[at..]).1; }
                at
            };
            let range = coordinates::ByteOffset(byte(a))..coordinates::ByteOffset(byte(b));
            if multibyte && !view.multibyte && !view.bytes[range.start.0..range.end.0].is_ascii() {
                return Err(Fault::Error("replace-match: mixed unibyte non-ASCII replacement is not implemented".into()));
            }
            if let Some(data::Object::String(s) | data::Object::RawString(s)) = self.vm.object_if(value) {
                output.properties.copy_into(coordinates::ByteOffset(assembled.len()), &s.properties.slice(range.clone()));
            }
            assembled.extend_from_slice(&view.bytes[range.start.0..range.end.0]);
        }
        let properties = output.properties;
        let mut output = data::LispString::from_bytes(assembled, multibyte);
        output.properties = properties;
        Ok(self.vm.allocate_object(if output.is_utf8() { data::Object::String(Box::new(output)) } else { data::Object::RawString(Box::new(output)) }))
    }
}
