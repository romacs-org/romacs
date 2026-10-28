//! Unicode defaults with a thin, buffer-local Lisp case-table override.
use super::*;
use icu_casemap::CaseMapper;

fn simple(code: u32, down: bool) -> u32 {
    let Some(c) = char::from_u32(code) else {
        return code;
    };
    let mapper = CaseMapper::new();
    (if down {
        mapper.simple_lowercase(c)
    } else {
        mapper.simple_uppercase(c)
    }) as u32
}

fn full(text: &str, down: bool) -> String {
    let mapper = CaseMapper::new();
    if down {
        mapper
            .lowercase_to_string(text, &icu_locale_core::LanguageIdentifier::UNKNOWN)
            .into_owned()
    } else {
        mapper
            .uppercase_to_string(text, &icu_locale_core::LanguageIdentifier::UNKNOWN)
            .into_owned()
    }
}

/// GNU's unconditional special casing of CODE (uni-special-uppercase.el,
/// uni-special-lowercase.el): its full Unicode mapping when that is not
/// one character, and fits case_character_impl's six-byte buffer.
fn special(code: u32, down: bool) -> Option<Vec<u32>> {
    let c = char::from_u32(code)?;
    let mapped = full(c.encode_utf8(&mut [0; 4]), down);
    let chars: Vec<u32> = mapped.chars().map(u32::from).collect();
    (chars.len() != 1 && mapped.len() <= 6).then_some(chars)
}

fn byte_code(code: u32, unibyte: bool) -> u32 {
    if unibyte && code >= 128 {
        code + crate::lstring::BYTE8_OFFSET
    } else {
        code
    }
}

impl Leaf<'_> {
    /// The Unicode simple mapping of CODE, the default a case table refines.
    pub(crate) fn unicode_case(code: u32, down: bool) -> u32 {
        simple(code, down)
    }

    pub(crate) fn byte_case_result(code: u32, original: u32, down: bool) -> u8 {
        if code >= crate::lstring::BYTE8_OFFSET + 128 {
            return (code - crate::lstring::BYTE8_OFFSET) as u8;
        }
        if code < 128 {
            return code as u8;
        }
        // Non-ASCII Unicode is not a raw octet. Never truncate it into binary data.
        if original < 128 {
            simple(original, down) as u8
        } else {
            original as u8
        }
    }
}

impl Leaf<'_> {
    /// TABLE's mapping of CODE, None for the Unicode default. A nil entry
    /// is identity (buffer.h downcase/upcase), except above ASCII in the
    /// native startup tables before GNU's Lisp enriches them
    /// (VM2-COMPLETION-CASE).
    fn case_override(&self, table: Value, code: u32, down: bool) -> std::result::Result<Option<u32>, Fault> {
        let value = self.char_table_element(table, code as usize)?;
        if value == Value::NIL {
            return Ok((code < 128 || !self.native_case_mapping(table, if down { 0 } else { 1 })).then_some(code));
        }
        // GNU ignores non-natural-number entries, but they remain explicit
        // identity overrides rather than requests for the Unicode default.
        let Some(mapped) = value.integer_value().filter(|n| *n >= 0) else {
            return Ok(Some(code));
        };
        if mapped > i64::from(crate::lstring::MAX_CHAR) {
            return Err(Fault::Wrong("characterp", value));
        }
        Ok(Some(mapped as u32))
    }

    /// Each of CODES as casefiddle.c's case_character cases it for upcase
    /// or downcase: in multibyte text the special casing first, then the
    /// case table's mapping, else the Unicode default, and a capital sigma
    /// changed after a word character and before none becomes final sigma.
    /// IN_BUFFER, a character with the syntax prefix flag starts no word.
    pub(crate) fn case_mappings(&mut self, codes: &[u32], multibyte: bool, down: bool, in_buffer: bool) -> std::result::Result<Vec<Vec<u32>>, Fault> {
        const SIGMA: u32 = 0x3a3;
        let table = self.current_case_mapping(down)?;
        // Only a capital sigma reads word syntax.
        let (words, prefixes) = if multibyte && codes.contains(&SIGMA) {
            let prefixes = if in_buffer { self.casing_prefix_flags(codes)? } else { vec![false; codes.len()] };
            (crate::prim::prims::capitalize::word_flags(self, codes)?, prefixes)
        } else { (Vec::new(), Vec::new()) };
        let mut inword = false;
        let mut mappings = Vec::with_capacity(codes.len());
        for (i, &code) in codes.iter().enumerate() {
            let was_inword = inword;
            inword = words.get(i).is_some_and(|&word| word && (was_inword || !prefixes[i]));
            if let Some(special) = multibyte.then(|| special(code, down)).flatten() {
                mappings.push(special);
                continue;
            }
            let default = if multibyte || code < 128 { simple(code, down) } else { code };
            let cased = self.case_override(table, byte_code(code, !multibyte), down)?.unwrap_or(default);
            let last = !words.get(i + 1).copied().unwrap_or(false);
            mappings.push(vec![if was_inword && code == SIGMA && cased != code && last { 0x3c2 } else { cased }]);
        }
        Ok(mappings)
    }

    pub fn case_convert(&mut self, value: Value, down: bool) -> super::super::Out {
        if let Some(n) = value.integer_value().filter(|n| *n >= 0) {
            // Preserve event modifiers and GNU's non-character integer domain.
            const FLAGS: i64 = 0x0fc0_0000;
            if n > FLAGS + i64::from(crate::lstring::MAX_CHAR) {
                return Ok(value);
            }
            let code = (n & !FLAGS) as u32;
            let table = self.current_case_mapping(down)?;
            let unibyte = code < 256 && !self.casing_multibyte()?;
            let lookup = byte_code(code, unibyte);
            let mapped = self
                .case_override(table, lookup, down)?
                .unwrap_or_else(|| simple(lookup, down));
            let mapped = if unibyte {
                Self::byte_case_result(mapped, code, down) as u32
            } else {
                mapped
            };
            return Value::integer(i64::from(mapped) | (n & FLAGS)).map_err(Fault::Error);
        }
        if !matches!(self.kind(value), Kind::String | Kind::Bytes) {
            return Err(Fault::Wrong("char-or-string-p", value));
        }
        let source = self.string_view(value)?;
        if source.chars == 0 {
            return Ok(value);
        }
        let codes: Vec<_> = source.chars().collect();
        let mappings = self.case_mappings(&codes, source.multibyte, down, false)?;
        let mut bytes = Vec::with_capacity(source.bytes.len());
        for (code, mapped) in codes.into_iter().zip(mappings) {
            for &c in &mapped {
                if source.multibyte {
                    let mut buf = [0; 5];
                    let len = crate::lstring::char_string(c, &mut buf);
                    bytes.extend_from_slice(&buf[..len]);
                } else {
                    // A byte string cannot silently become Unicode or truncate
                    // an unrepresentable override. Fall back to its byte default.
                    bytes.push(Self::byte_case_result(c, code, down));
                }
            }
        }
        let mut result = data::LispString::from_bytes(bytes, source.multibyte);
        // GNU's multibyte case path constructs fresh unpropertied text; its
        // unibyte path copies the string, including all property identities.
        if !source.multibyte {
            if let Some(data::Object::String(s) | data::Object::RawString(s)) =
                self.vm.object_if(value)
            {
                result.properties = s.properties.clone();
            }
        }
        Ok(self.vm.allocate_object(if result.is_utf8() {
            data::Object::String(Box::new(result))
        } else {
            data::Object::RawString(Box::new(result))
        }))
    }
}

/// The case_action of casify_region this VM implements.
#[derive(Clone, Copy)]
pub(crate) enum RegionCase {
    Up,
    Down,
    Capitalize,
    InitialsUp,
}

impl Leaf<'_> {
    /// What each character of START..END of the current buffer becomes,
    /// as casify_region cases it: a multibyte buffer's like a multibyte
    /// string's (word starts after a syntax prefix flag aside), a unibyte
    /// buffer's byte for byte like a unibyte string's.
    fn region_casing(&mut self, start: i64, end: i64, case: RegionCase) -> std::result::Result<(bool, Vec<u32>, Vec<Vec<u32>>), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let range = (start - 1) as usize..(end - 1) as usize;
        let (multibyte, codes) = match &self.vm.objects[index] {
            data::Object::Buffer(b) => (true, range.map(|at| b.character_of_char(crate::coordinates::CharOffset(at))
                .map(|c| c.map_or(0, u32::from))).collect::<crate::Result<Vec<_>>>().map_err(Fault::Error)?),
            data::Object::ByteBuffer(b) => (false, b.bytes[range].iter().map(|&b| u32::from(b)).collect()),
            _ => return Err(Fault::Error("buffer required".into())),
        };
        let cased = match case {
            RegionCase::Up => self.case_mappings(&codes, multibyte, false, true)?,
            RegionCase::Down => self.case_mappings(&codes, multibyte, true, true)?,
            RegionCase::Capitalize | RegionCase::InitialsUp => {
                let chars: Vec<u32> = codes.iter().map(|&c| byte_code(c, !multibyte)).collect();
                let prefixes = self.casing_prefix_flags(&chars)?;
                let initials = matches!(case, RegionCase::InitialsUp);
                crate::prim::prims::capitalize::capitalized_chars(self, &chars, Some(&prefixes), initials)?
            }
        };
        Ok((multibyte, codes, cased))
    }

    /// casify_region's change of START..END of the current buffer once
    /// modify_text's hooks ran: the region's undo records, then each
    /// character cased in place. Returns the first and last changed
    /// positions and the characters added, or None when none changed.
    pub(crate) fn casify_region_edit(&mut self, start: i64, end: i64, case: RegionCase) -> std::result::Result<Option<(i64, i64, i64)>, Fault> {
        let (_, begv, zv) = self.point_extent()?;
        let (start, end) = (start.max(begv), end.min(zv).max(start.max(begv)));
        let (multibyte, codes, cased) = self.region_casing(start, end, case)?;
        // A VM2 multibyte buffer holds Unicode text: refuse a raw byte
        // before anything changes.
        if multibyte && cased.iter().flatten().any(|&c| char::from_u32(c).is_none()) {
            return Err(Fault::Error("VM2 casing cannot put a raw byte in a multibyte buffer".into()));
        }
        self.modify_text_tail(start, end, false)?;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let buffer = self.current_buffer()?;
        let recording = self.vm.undo_list_of(index) != Value::TRUE;
        if recording {
            let text = self.vm.buffer_text(buffer, start, end)?;
            self.vm.record_delete(index, start, text, false).map_err(Fault::Error)?;
            if !multibyte { self.vm.record_insert(index, start, end - start, false).map_err(Fault::Error)?; }
        }
        let (mut first, mut last, mut added) = (None, 0, 0);
        for (i, (&code, out)) in codes.iter().zip(&cased).enumerate() {
            let at = start - 1 + i as i64 + added;
            match &mut self.vm.objects[index] {
                data::Object::Buffer(b) => {
                    if out.as_slice() == [code] { continue; }
                    last = at + 1 + out.len() as i64;
                    let text: String = out.iter().filter_map(|&c| char::from_u32(c)).collect();
                    let from = b.byte_of_char(crate::coordinates::CharOffset(at as usize)).map_err(Fault::Error)?;
                    let to = b.byte_of_char(crate::coordinates::CharOffset(at as usize + 1)).map_err(Fault::Error)?;
                    if out.len() == 1 && text.len() == to.0 - from.0 {
                        b.overwrite(from..to, &text).map_err(Fault::Error)?;
                    } else {
                        // replace_range_2: one character for others,
                        // keeping its text properties.
                        b.rewrite_character(from..to, &text).map_err(Fault::Error)?;
                        b.state.record_text_change(out.len() + 1);
                        added += out.len() as i64 - 1;
                    }
                }
                data::Object::ByteBuffer(b) => {
                    let byte = Self::byte_case_result(out[0], code, matches!(case, RegionCase::Down));
                    if u32::from(byte) == code { continue; }
                    b.bytes[at as usize] = byte;
                    last = at + 2;
                }
                _ => unreachable!("buffer_index validates"),
            }
            first.get_or_insert(at + 1);
        }
        self.vm.display_state.version += 1;
        if recording && multibyte {
            self.vm.record_insert(index, start, end - start + added, false).map_err(Fault::Error)?;
        }
        Ok(first.map(|first| (first, last, added)))
    }
}
