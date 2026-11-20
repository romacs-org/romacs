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
//!
//! A unibyte name with a byte above 127 is another name than the
//! multibyte one of the same characters (GNU's `oblookup' tells them apart
//! by character count and bytes), and its `symbol-name' is unibyte: its
//! key starts with the escape and `UNIBYTE`, then its bytes as characters,
//! those above 127 as raw-byte characters. Text decoded from a key drops
//! the mark (`to_internal'); a name's table key and `symbol-name' keep it
//! (`names_key', `unibyte_name').

/// The escape: a noncharacter, reserved for internal use.
pub(crate) const ESC: char = '\u{fdd0}';
const PLANE15: u32 = 0xF_0000;
/// After the escape, the mark of a unibyte name: the Plane 15 character
/// just past those an escaped code uses.
const UNIBYTE: char = '\u{f0800}';

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

/// TEXT made safe to join printer output, which is decoded as keys: a
/// genuine U+FDD0 doubled, everything else as it is.
pub(crate) fn escape_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(ESC) { text.replace(ESC, "\u{fdd0}\u{fdd0}").into() } else { text.into() }
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

/// The key of a unibyte name of BYTES, some above 127.
pub(crate) fn unibyte_key(bytes: &[u8]) -> String {
    let mut key = String::from(ESC);
    key.push(UNIBYTE);
    key.push_str(&from_internal(&crate::text::octets_to_multibyte(bytes)));
    key
}

/// The bytes of the unibyte name KEY stands for, or None when it is not one.
pub(crate) fn unibyte_name(key: &str) -> Option<Vec<u8>> {
    let rest = key.strip_prefix(ESC)?.strip_prefix(UNIBYTE)?;
    let internal = to_internal(rest).unwrap_or_else(|| rest.as_bytes().to_vec());
    let mut bytes = Vec::with_capacity(internal.len());
    let mut at = 0;
    while at < internal.len() {
        let (code, width) = crate::lstring::string_char(&internal[at..]);
        // ASCII as itself, a raw-byte character as its byte.
        bytes.push(if code < 0x80 { code as u8 } else { (code - 0x3F_FF00) as u8 });
        at += width;
    }
    Some(bytes)
}

/// The Lisp name key text KEY stands for: its bytes and whether they are
/// multibyte, or None when KEY is plain text, its own multibyte bytes.
pub(crate) fn lisp_name(key: &str) -> Option<(Vec<u8>, bool)> {
    match unibyte_name(key) {
        Some(bytes) => Some((bytes, false)),
        None => to_internal(key).map(|internal| (internal, true)),
    }
}

/// How many characters internal multibyte BYTES hold.
pub(crate) fn internal_chars(bytes: &[u8]) -> usize {
    let (mut at, mut chars) = (0, 0);
    while at < bytes.len() {
        at += crate::lstring::string_char(&bytes[at..]).1;
        chars += 1;
    }
    chars
}

/// The names table's key (names.rs) of key text KEY, or None when KEY is
/// its own: a unibyte name's bytes behind 0xFF, which internal multibyte
/// text never holds, else the internal bytes.
pub(crate) fn names_key(key: &str) -> Option<Vec<u8>> {
    match unibyte_name(key) {
        Some(bytes) => Some([&[0xFF], &bytes[..]].concat()),
        None => to_internal(key),
    }
}

/// The internal multibyte bytes KEY stands for, or None when KEY is plain
/// Unicode text (its own bytes). A unibyte name's mark is dropped: its
/// bytes above 127 are raw-byte characters in the text.
pub(crate) fn to_internal(key: &str) -> Option<Vec<u8>> {
    if !key.contains(ESC) { return None; }
    let mut out = Vec::with_capacity(key.len());
    let mut chars = key.chars().peekable();
    let mut buf = [0u8; 5];
    while let Some(c) = chars.next() {
        let code = if c != ESC { u32::from(c) } else {
            match chars.peek().copied() {
                Some(ESC) => { chars.next(); u32::from(ESC) }
                Some(UNIBYTE) => { chars.next(); continue; }
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
