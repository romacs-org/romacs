//! Rust-side symbol keys for names that are not Unicode text.
//!
//! A symbol's name is a Lisp string in GNU's internal character encoding,
//! which also holds raw-byte characters and characters beyond Unicode.
//! VM2 keys its symbol table, its read forms and its printer output by
//! Rust `String`s, so such a name is carried in them through an injective
//! escape: every Unicode scalar stands for itself except the noncharacter
//! U+FDD0, which is doubled; any other character is U+FDD0 followed by two
//! Plane 15 private-use characters holding its 22-bit code. Plain Unicode
//! names are therefore their own keys. Every place a name becomes Lisp
//! text again (`symbol-name', the printer's output) decodes the key, so
//! Lisp never sees the escape.

/// The escape: a noncharacter, reserved for internal use.
pub(crate) const ESC: char = '\u{fdd0}';
const PLANE15: u32 = 0xF_0000;

/// Append internal character CODE to KEY.
pub(crate) fn push_char(key: &mut String, code: u32) {
    match char::from_u32(code) {
        Some(ESC) => { key.push(ESC); key.push(ESC); }
        Some(c) => key.push(c),
        None => {
            key.push(ESC);
            key.push(char::from_u32(PLANE15 + (code >> 11)).unwrap());
            key.push(char::from_u32(PLANE15 + (code & 0x7ff)).unwrap());
        }
    }
}

/// The key of a name given as internal multibyte BYTES.
pub(crate) fn from_internal(bytes: &[u8]) -> String {
    let mut key = String::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let (code, width) = crate::lstring::string_char(&bytes[at..]);
        push_char(&mut key, code);
        at += width;
    }
    key
}

/// The internal multibyte bytes KEY stands for, or None when KEY is plain
/// Unicode text (its own bytes).
pub(crate) fn to_internal(key: &str) -> Option<Vec<u8>> {
    if !key.contains(ESC) { return None; }
    let mut out = Vec::with_capacity(key.len());
    let mut chars = key.chars().peekable();
    let mut buf = [0u8; 5];
    while let Some(c) = chars.next() {
        let code = if c != ESC { u32::from(c) } else {
            match chars.peek().copied() {
                Some(ESC) => { chars.next(); u32::from(ESC) }
                Some(hi) if (PLANE15..PLANE15 + 0x800).contains(&u32::from(hi)) => {
                    chars.next();
                    let lo = chars.next().map_or(0, u32::from).wrapping_sub(PLANE15) & 0x7ff;
                    ((u32::from(hi) - PLANE15) << 11) | lo
                }
                // A lone escape is a genuine U+FDD0 from text no key wrote.
                _ => u32::from(ESC),
            }
        };
        let n = crate::lstring::char_string(code.min(crate::lstring::MAX_CHAR), &mut buf);
        out.extend_from_slice(&buf[..n]);
    }
    Some(out)
}

impl crate::Vm {
    /// Printer output TEXT as a Lisp string: the characters escaped symbol
    /// names carried decoded, otherwise the text as the printer's callers
    /// always made it.
    pub(crate) fn printed_value(&mut self, text: &str) -> crate::Value {
        match to_internal(text) {
            Some(internal) => self.make_lisp_string(internal, true),
            None => self.allocate_object(crate::data::Object::String(Box::new(crate::data::LispString::new(text)))),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_round_trip_every_kind_of_character() {
        for codes in [vec![114, 97, 119, 0x3f_ffff], vec![0xfdd0, 0x41], vec![0x11_4019, 0xd800], vec![0xfdd0, 0xfdd0]] {
            let mut internal = Vec::new();
            let mut key = String::new();
            for &c in &codes {
                let mut b = [0; 5];
                let n = crate::lstring::char_string(c, &mut b);
                internal.extend_from_slice(&b[..n]);
                super::push_char(&mut key, c);
            }
            assert_eq!(super::from_internal(&internal), key);
            assert_eq!(super::to_internal(&key).unwrap_or_else(|| key.as_bytes().to_vec()), internal, "{codes:x?}");
        }
        assert_eq!(super::from_internal("abc".as_bytes()), "abc");
        assert_eq!(super::to_internal("abc"), None);
    }
}
