//! Local, canonical GNU internal string encoding. No heap Values live here.
pub(crate) const MAX_CHAR: u32 = 0x3f_ffff;
pub(crate) const MAX_5_BYTE_CHAR: u32 = 0x3f_ff7f;
pub(crate) const BYTE8_OFFSET: u32 = 0x3f_ff00;

pub(crate) fn char_string(code: u32, out: &mut [u8]) -> usize {
    assert!(code <= MAX_CHAR, "character out of range");
    let (value, width, lead) = match code {
        0..=0x7f => {
            out[0] = code as u8;
            return 1;
        }
        0x80..=0x7ff => (code, 2, 0xc0),
        0x800..=0xffff => (code, 3, 0xe0),
        0x10000..=0x1fffff => (code, 4, 0xf0),
        0x200000..=MAX_5_BYTE_CHAR => (code, 5, 0xf8),
        _ => (code - 0x3f_ff80, 2, 0xc0),
    };
    out[0] = lead | (value >> (6 * (width - 1))) as u8;
    for (i, byte) in out.iter_mut().enumerate().take(width).skip(1) {
        *byte = 0x80 | ((value >> (6 * (width - i - 1))) & 0x3f) as u8;
    }
    width
}

/// Reject truncated, noncanonical and out-of-range encodings, but allow the
/// surrogate codes: they are GNU characters even though not Unicode scalars.
pub(crate) fn decode_char(bytes: &[u8]) -> Option<(u32, usize)> {
    let &lead = bytes.first()?;
    let (width, mut code) = match lead {
        0..=0x7f => return Some((lead as u32, 1)),
        0xc0..=0xdf => (2, (lead & 0x1f) as u32),
        0xe0..=0xef => (3, (lead & 0x0f) as u32),
        0xf0..=0xf7 => (4, (lead & 7) as u32),
        0xf8 => (5, 0),
        _ => return None,
    };
    for &byte in bytes.get(1..width)? {
        if byte & 0xc0 != 0x80 {
            return None;
        }
        code = (code << 6) | (byte & 0x3f) as u32;
    }
    if lead <= 0xc1 {
        code += 0x3f_ff80;
    }
    if code > MAX_CHAR {
        return None;
    }
    let mut canonical = [0; 5];
    let n = char_string(code, &mut canonical);
    (n == width && canonical[..n] == bytes[..width]).then_some((code, width))
}

/// Decode storage already validated at construction.
pub(crate) fn string_char(bytes: &[u8]) -> (u32, usize) {
    let lead = bytes[0];
    if lead < 0x80 {
        return (lead as u32, 1);
    }
    let (width, mut code) = match lead {
        0xc0..=0xdf => (2, (lead & 0x1f) as u32),
        0xe0..=0xef => (3, (lead & 0x0f) as u32),
        0xf0..=0xf7 => (4, (lead & 7) as u32),
        0xf8 => (5, 0),
        _ => panic!("invalid internal string lead"),
    };
    for &byte in &bytes[1..width] {
        code = (code << 6) | (byte & 0x3f) as u32;
    }
    if lead <= 0xc1 {
        code += 0x3f_ff80;
    }
    (code, width)
}

#[derive(Clone, Copy)]
pub(crate) struct StrView<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) chars: usize,
    pub(crate) multibyte: bool,
}
impl<'a> StrView<'a> {
    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub(crate) fn len(&self) -> usize {
        self.chars
    }
    pub(crate) fn is_multibyte(&self) -> bool {
        self.multibyte
    }
    pub(crate) fn as_str(&self) -> Option<&'a str> {
        if !self.multibyte && !self.bytes.is_ascii() {
            return None;
        }
        std::str::from_utf8(self.bytes).ok()
    }
    pub(crate) fn chars(&self) -> impl Iterator<Item = u32> + 'a + use<'a> {
        let mut bytes = self.bytes;
        let multibyte = self.multibyte;
        std::iter::from_fn(move || {
            if bytes.is_empty() {
                return None;
            }
            let (code, width) = if multibyte {
                string_char(bytes)
            } else {
                (bytes[0] as u32, 1)
            };
            bytes = &bytes[width..];
            Some(code)
        })
    }
    pub(crate) fn to_owned(&self) -> OwnedStr {
        OwnedStr {
            bytes: self.bytes.to_vec(),
            chars: self.chars,
            multibyte: self.multibyte,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OwnedStr {
    pub(crate) bytes: Vec<u8>,
    pub(crate) chars: usize,
    pub(crate) multibyte: bool,
}
impl OwnedStr {
    pub(crate) fn view(&self) -> StrView<'_> {
        StrView {
            bytes: &self.bytes,
            chars: self.chars,
            multibyte: self.multibyte,
        }
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn len(&self) -> usize {
        self.chars
    }
    pub(crate) fn is_multibyte(&self) -> bool {
        self.multibyte
    }
    pub(crate) fn as_str(&self) -> Option<&str> {
        self.view().as_str()
    }
    pub(crate) fn chars(&self) -> impl Iterator<Item = u32> + '_ {
        self.view().chars()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Additional validation of the new internal API, not red-first surface evidence.
    #[test]
    fn exhaustive_codec_roundtrip_matches_regex_engine() {
        for code in 0..=MAX_CHAR {
            let mut ours = [0; 5];
            let mut engine = [0; 5];
            let n = char_string(code, &mut ours);
            assert_eq!(
                string_char(&ours[..n]),
                (code, n),
                "storage decoder {code:#x}"
            );
            assert_eq!(
                decode_char(&ours[..n]),
                Some((code, n)),
                "roundtrip {code:#x}"
            );
            let m = romacs_regex_engine::emacs_char::char_string(code, &mut engine);
            assert_eq!(&ours[..n], &engine[..m], "encoder disagreement {code:#x}");
            assert_eq!(
                romacs_regex_engine::emacs_char::string_char(&ours[..n]),
                (code, n),
                "decoder disagreement {code:#x}"
            );
        }
    }
}
