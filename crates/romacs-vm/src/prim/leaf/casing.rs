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

// Map complete Unicode runs, never one scalar at a time: final sigma needs
// context. Non-Unicode GNU character codes are preserved and separate runs.
// In the root locale, contextual casing changes values, not expansion lengths.
// Derive those lengths from the SAME provider (not handwritten Unicode tables).
fn full_mappings(codes: &[u32], down: bool) -> std::result::Result<Vec<Vec<u32>>, Fault> {
    let mut result = Vec::with_capacity(codes.len());
    let mut start = 0;
    while start < codes.len() {
        if char::from_u32(codes[start]).is_none() {
            result.push(vec![codes[start]]);
            start += 1;
            continue;
        }
        let mut end = start;
        let mut text = String::new();
        while end < codes.len() {
            let Some(c) = char::from_u32(codes[end]) else {
                break;
            };
            text.push(c);
            end += 1;
        }
        let mapped = full(&text, down);
        let mut output = mapped.chars();
        for c in text.chars() {
            let mut buf = [0; 4];
            let count = full(c.encode_utf8(&mut buf), down).chars().count();
            let piece: Vec<_> = output.by_ref().take(count).map(u32::from).collect();
            if piece.len() != count {
                return Err(Fault::Error(
                    "Unicode casing expansion length mismatch".into(),
                ));
            }
            result.push(piece);
        }
        if output.next().is_some() {
            return Err(Fault::Error(
                "Unicode casing expansion length mismatch".into(),
            ));
        }
        start = end;
    }
    Ok(result)
}

fn byte_code(code: u32, unibyte: bool) -> u32 {
    if unibyte && code >= 128 {
        code + crate::lstring::BYTE8_OFFSET
    } else {
        code
    }
}

impl Leaf<'_> {
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
    fn case_override(&self, table: Value, code: u32) -> std::result::Result<Option<u32>, Fault> {
        let value = self.char_table_element(table, code as usize)?;
        if value == Value::NIL {
            return Ok(None);
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
                .case_override(table, lookup)?
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
        let table = self.current_case_mapping(down)?;
        let codes: Vec<_> = source.chars().collect();
        let defaults = if source.multibyte {
            full_mappings(&codes, down)?
        } else {
            codes
                .iter()
                .map(|&c| vec![if c < 128 { simple(c, down) } else { c }])
                .collect()
        };
        let mut bytes = Vec::with_capacity(source.bytes.len());
        for (code, default) in codes.into_iter().zip(defaults) {
            let mapped = self.case_override(table, byte_code(code, !source.multibyte))?;
            let mapping = mapped.as_slice();
            let mapped = if mapping.is_empty() {
                default.as_slice()
            } else {
                mapping
            };
            for &c in mapped {
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
