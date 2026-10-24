//! GNU Emacs character encoding (character.h/.c), without Lisp objects or string flags.
//!
//! Characters are `u32` codes `0..=MAX_CHAR`. Multibyte text is GNU's internal
//! form: UTF-8 extended to 5 bytes for codes up to `MAX_5_BYTE_CHAR`, plus the
//! raw-byte characters `0x3FFF80..=0x3FFFFF`, stored as the two bytes
//! `C0/C1 80..BF`. Unibyte text is one byte per character.
//!
//! Validate untrusted forms with `multibyte_length`, or normalize arbitrary
//! bytes with `str_as_multibyte`. Trusted traversal and multibyte-to-unibyte
//! conversion require complete valid forms and character-aligned endpoints.
//! Storage consumers must assemble any form split across chunks or a gap;
//! malformed trusted input has no GNU-compatible repair contract.

mod pos;

pub use pos::{BytePos, CharPos, LispPos};

pub const MAX_CHAR: u32 = 0x3F_FFFF;
pub const MAX_UNICODE_CHAR: u32 = 0x10_FFFF;
pub const MAX_1_BYTE_CHAR: u32 = 0x7F;
pub const MAX_2_BYTE_CHAR: u32 = 0x7FF;
pub const MAX_3_BYTE_CHAR: u32 = 0xFFFF;
pub const MAX_4_BYTE_CHAR: u32 = 0x1F_FFFF;
pub const MAX_5_BYTE_CHAR: u32 = 0x3F_FF7F;
pub const MAX_MULTIBYTE_LENGTH: usize = 5;

const BYTE8_OFFSET: u32 = 0x3F_FF00;

/// `CHAR_VALID_P`: C is a character code.
#[inline]
pub fn char_valid_p(c: i64) -> bool {
    (0..=MAX_CHAR as i64).contains(&c)
}

/// `CHAR_BYTE8_P`: C is the character of a raw 8-bit byte.
#[inline]
pub fn char_byte8_p(c: u32) -> bool {
    c > MAX_5_BYTE_CHAR
}

/// `BYTE8_TO_CHAR`; BYTE must be 0x80..=0xFF.
#[inline]
pub fn byte8_to_char(byte: u8) -> u32 {
    debug_assert!(byte >= 0x80);
    byte as u32 + BYTE8_OFFSET
}

/// `UNIBYTE_TO_CHAR`: ASCII stays, other bytes become raw-byte characters.
#[inline]
pub fn unibyte_to_char(byte: u8) -> u32 {
    if byte < 0x80 {
        byte as u32
    } else {
        byte8_to_char(byte)
    }
}

/// `CHAR_TO_BYTE8`: the byte of a raw-byte character, else C's low 8 bits.
#[inline]
pub fn char_to_byte8(c: u32) -> u8 {
    if char_byte8_p(c) {
        (c - BYTE8_OFFSET) as u8
    } else {
        c as u8
    }
}

/// `CHAR_TO_BYTE_SAFE`: the byte for an ASCII or raw-byte character.
#[inline]
pub fn char_to_byte_safe(c: u32) -> Option<u8> {
    match c {
        0..=MAX_1_BYTE_CHAR => Some(c as u8),
        _ if char_byte8_p(c) => Some((c - BYTE8_OFFSET) as u8),
        _ => None,
    }
}

/// `CHAR_BYTES`: length of C's multibyte form.
#[inline]
pub fn char_bytes(c: u32) -> usize {
    match c {
        _ if c <= MAX_1_BYTE_CHAR => 1,
        _ if c <= MAX_2_BYTE_CHAR => 2,
        _ if c <= MAX_3_BYTE_CHAR => 3,
        _ if c <= MAX_4_BYTE_CHAR => 4,
        _ if c <= MAX_5_BYTE_CHAR => 5,
        _ => 2,
    }
}

/// `CHAR_STRING`: store C's multibyte form in OUT and return its length.
///
/// Panics unless C is a character (GNU signals "Invalid character").
/// Modifier bits are the caller's business, as in `CHAR_STRING`.
pub fn char_string(c: u32, out: &mut [u8; MAX_MULTIBYTE_LENGTH]) -> usize {
    assert!(c <= MAX_CHAR, "invalid character {c:#x}");
    let tail = |shift: u32| 0x80 | ((c >> shift) & 0x3F) as u8;
    match char_bytes(c) {
        1 => out[0] = c as u8,
        2 if char_byte8_p(c) => {
            out[0] = 0xC0 | ((c >> 6) & 1) as u8;
            out[1] = tail(0);
        }
        2 => [out[0], out[1]] = [0xC0 | (c >> 6) as u8, tail(0)],
        3 => [out[0], out[1], out[2]] = [0xE0 | (c >> 12) as u8, tail(6), tail(0)],
        4 => {
            [out[0], out[1], out[2], out[3]] = [0xF0 | (c >> 18) as u8, tail(12), tail(6), tail(0)];
        }
        _ => {
            out[0] = 0xF8;
            out[1] = 0x80 | ((c >> 18) & 0x0F) as u8;
            [out[2], out[3], out[4]] = [tail(12), tail(6), tail(0)];
        }
    }
    char_bytes(c)
}

/// `LEADING_CODE_P`: BYTE starts a non-ASCII multibyte form.
#[inline]
pub fn leading_code_p(byte: u8) -> bool {
    byte & 0xC0 == 0xC0
}

/// `TRAILING_CODE_P`.
#[inline]
pub fn trailing_code_p(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// `CHAR_HEAD_P`: BYTE starts a character.
#[inline]
pub fn char_head_p(byte: u8) -> bool {
    byte & 0xC0 != 0x80
}

/// `CHAR_BYTE8_HEAD_P`: BYTE starts a raw-byte character.
#[inline]
pub fn char_byte8_head_p(byte: u8) -> bool {
    byte == 0xC0 || byte == 0xC1
}

/// `BYTES_BY_CHAR_HEAD`: form length from the head byte alone, unvalidated.
#[inline]
pub fn bytes_by_char_head(byte: u8) -> usize {
    match byte {
        _ if byte & 0x80 == 0 => 1,
        _ if byte & 0x20 == 0 => 2,
        _ if byte & 0x10 == 0 => 3,
        _ if byte & 0x08 == 0 => 4,
        _ => 5,
    }
}

/// `multibyte_length` with `check`: the length of the valid multibyte form
/// at the start of S, or 0. ALLOW_8BIT accepts raw-byte forms.
pub fn multibyte_length(s: &[u8], allow_8bit: bool) -> usize {
    // A literal port of GNU's packed range checks.
    let Some(&c) = s.first() else { return 0 };
    if c < 0x80 {
        return 1;
    }
    let Some(&d) = s.get(1) else { return 0 };
    let mut w = (((d & 0xC0) as i64) << 2) + c as i64;
    if (if allow_8bit { 0x2C0 } else { 0x2C2 }..=0x2DF).contains(&w) {
        return 2;
    }
    let Some(&e) = s.get(2) else { return 0 };
    w += ((e & 0xC0) as i64) << 4;
    if (0xAE1..=0xAEF).contains(&(w | ((d & 0x20) as i64 >> 2))) {
        return 3;
    }
    let Some(&f) = s.get(3) else { return 0 };
    w += ((f & 0xC0) as i64) << 6;
    if (0x2AF1..=0x2AF7).contains(&(w | ((d & 0x30) as i64 >> 3))) {
        return 4;
    }
    let Some(&g) = s.get(4) else { return 0 };
    let lw = w + (((g & 0xC0) as i64) << 8);
    let w3 = (lw << 24) + ((d as i64) << 16) + ((e as i64) << 8) + f as i64;
    if (0xAA_F888_8080..=0xAA_F88F_BFBD).contains(&w3) {
        5
    } else {
        0
    }
}

/// `string_char_and_length`: the character at the start of S and its length.
///
/// S must start on a character boundary with a complete, valid GNU multibyte
/// form. This trusted decoder does not validate or repair malformed forms;
/// use `multibyte_length` or `str_as_multibyte` at an untrusted byte boundary.
/// Panics if S is empty or shorter than its head's form.
#[inline]
pub fn string_char(s: &[u8]) -> (u32, usize) {
    debug_assert!(multibyte_length(s, true) > 0, "invalid multibyte form");
    let c = s[0] as u32;
    if c < 0x80 {
        return (c, 1);
    }
    let mut d = (c << 6) + s[1] as u32 - ((0xC0 << 6) + 0x80);
    if c < 0xE0 {
        return (d + if c < 0xC2 { 0x3F_FF80 } else { 0 }, 2);
    }
    d = (d << 6) + s[2] as u32 - ((0x20 << 12) + 0x80);
    if c < 0xF0 {
        return (d, 3);
    }
    d = (d << 6) + s[3] as u32 - ((0x10 << 18) + 0x80);
    if c < 0xF8 {
        return (d, 4);
    }
    d = (d << 6) + s[4] as u32 - ((0x08 << 24) + 0x80);
    (d, 5)
}

/// `STRING_CHAR_ADVANCE` over complete, valid GNU multibyte text.
/// Both slice endpoints must be character boundaries. This is not a streaming
/// decoder: consumers must assemble forms that cross storage segments.
#[inline]
pub fn chars(s: &[u8]) -> impl Iterator<Item = u32> + '_ {
    debug_assert!(multibyte_chars_in_text(s) <= s.len());
    Chars(s)
}

struct Chars<'a>(&'a [u8]);

impl Iterator for Chars<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        if self.0.is_empty() {
            return None;
        }
        let (c, len) = string_char(self.0);
        self.0 = &self.0[len..];
        Some(c)
    }

    #[inline]
    fn fold<B, F>(mut self, mut acc: B, mut f: F) -> B
    where
        F: FnMut(B, u32) -> B,
    {
        while let Some(&head) = self.0.first() {
            if head < 0x80 {
                while let Some(block) = self.0.first_chunk::<8>() {
                    if u64::from_ne_bytes(*block) & 0x8080_8080_8080_8080 != 0 {
                        break;
                    }
                    for &byte in block {
                        acc = f(acc, byte as u32);
                    }
                    self.0 = &self.0[8..];
                }
                if self.0.is_empty() {
                    break;
                }
            }
            let (c, len) = string_char(self.0);
            acc = f(acc, c);
            self.0 = &self.0[len..];
        }
        acc
    }
}

/// `raw_prev_char_len`: length of the character ending at S's end.
/// S must be nonempty valid multibyte text, with both endpoints on character
/// boundaries. Empty/malformed slices do not have a GNU traversal contract.
#[inline]
pub fn prev_char_len(s: &[u8]) -> usize {
    debug_assert!(!s.is_empty() && multibyte_chars_in_text(s) <= s.len());
    s.iter()
        .rev()
        .position(|&b| char_head_p(b))
        .map_or(s.len(), |i| i + 1)
}

/// `multibyte_chars_in_text`: characters in valid multibyte text.
/// Panics on an invalid form (GNU aborts).
pub fn multibyte_chars_in_text(mut s: &[u8]) -> usize {
    let mut chars = 0;
    while !s.is_empty() {
        let len = multibyte_length(s, true);
        assert!(len > 0, "invalid multibyte form");
        s = &s[len..];
        chars += 1;
    }
    chars
}

/// `count_size_as_multibyte`: byte length of `str_to_multibyte (S)`.
pub fn count_size_as_multibyte(s: &[u8]) -> usize {
    s.len() + s.iter().filter(|&&b| b >= 0x80).count()
}

/// `str_to_multibyte` (`string-to-multibyte`): every byte >= 0x80 becomes a
/// raw-byte character.
pub fn str_to_multibyte(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(count_size_as_multibyte(s));
    for &b in s {
        if b < 0x80 {
            out.push(b);
        } else {
            out.extend([0xC0 | ((b >> 6) & 1), 0x80 | (b & 0x3F)]);
        }
    }
    out
}

/// `str_as_multibyte` (`string-as-multibyte`): valid forms are kept, other
/// bytes become raw-byte characters. Returns the text and its char count.
pub fn str_as_multibyte(mut s: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(s.len());
    let mut chars = 0;
    while let Some(&b) = s.first() {
        let len = multibyte_length(s, false);
        if len > 0 {
            out.extend_from_slice(&s[..len]);
            s = &s[len..];
        } else {
            out.extend([0xC0 | ((b >> 6) & 1), 0x80 | (b & 0x3F)]);
            s = &s[1..];
        }
        chars += 1;
    }
    (out, chars)
}

/// `str_as_unibyte` (`string-as-unibyte`): raw-byte characters become their
/// bytes; all other bytes are kept. S must contain complete valid multibyte
/// text, with both endpoints on character boundaries. Truncated-tail clamping
/// is defensive behavior, not a GNU-compatible malformed-input repair.
pub fn str_as_unibyte(mut s: &[u8]) -> Vec<u8> {
    debug_assert!(multibyte_chars_in_text(s) <= s.len());
    let mut out = Vec::with_capacity(s.len());
    while let Some(&b) = s.first() {
        if char_byte8_head_p(b) && s.len() >= 2 {
            out.push(0x80 | (b & 1) << 6 | (s[1] & 0x3F));
            s = &s[2..];
        } else {
            let len = bytes_by_char_head(b).min(s.len());
            out.extend_from_slice(&s[..len]);
            s = &s[len..];
        }
    }
    out
}

/// `string-to-unibyte`: ASCII and raw-byte characters to bytes, or the
/// index of the first other character. S must contain complete valid multibyte
/// text, with both endpoints on character boundaries.
pub fn str_to_unibyte(mut s: &[u8]) -> Result<Vec<u8>, usize> {
    debug_assert!(multibyte_chars_in_text(s) <= s.len());
    let mut out = Vec::with_capacity(s.len());
    while let Some(&b) = s.first() {
        if b < 0x80 {
            out.push(b);
            s = &s[1..];
        } else if char_byte8_head_p(b) && s.len() >= 2 {
            out.push(0x80 | (b & 1) << 6 | (s[1] & 0x3F));
            s = &s[2..];
        } else {
            return Err(out.len());
        }
    }
    Ok(out)
}
