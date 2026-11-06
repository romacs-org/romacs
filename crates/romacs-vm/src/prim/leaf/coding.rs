//! GNU coding.c registry and native UTF-8/raw-text string conversion.
use super::*;

const ROOT: &str = "#:vm2-coding-system-registry";
/// coding.c coding_categories[].id: the coding system set up for each
/// category (nil while its id is -1), and coding_priorities[].
const CATEGORY_SYSTEMS: &str = "#:vm2-coding-category-systems";
const PRIORITIES: &str = "#:vm2-coding-priorities";
const CATEGORIES: [&str; 21] = [
    "iso-7",
    "iso-7-tight",
    "iso-8-1",
    "iso-8-2",
    "iso-7-else",
    "iso-8-else",
    "utf-8-auto",
    "utf-8",
    "utf-8-sig",
    "utf-16-auto",
    "utf-16-be",
    "utf-16-le",
    "utf-16-be-nosig",
    "utf-16-le-nosig",
    "charset",
    "sjis",
    "big5",
    "ccl",
    "emacs-mule",
    "raw-text",
    "undecided",
];
fn num(n: i64) -> Value {
    Value::integer(n).unwrap()
}

impl Leaf<'_> {
    pub(crate) fn coding_convert_string(&mut self, string: Value, coding: Value, nocopy: Value, buffer: Value, encode: bool) -> super::super::Out {
        self.coding_convert(string, coding, nocopy, buffer, encode, true)
    }
    /// code_convert_string; ASCII_FAST is false once detection replaced
    /// the system the caller named, whose ASCII optimization alone applies.
    fn coding_convert(&mut self, string: Value, coding: Value, nocopy: Value, buffer: Value, encode: bool, ascii_fast: bool) -> super::super::Out {
        let source = self.string_view(string)?;
        self.coding_symbol(coding)?;
        let last = self.intern("last-coding-system-used");
        let name = if coding == Value::NIL {
            let name = self.intern("no-conversion");
            self.variable_set(last, name)?;
            if buffer == Value::NIL {
                return if nocopy != Value::NIL { Ok(string) } else { self.propertized_copy(string, &[]) };
            }
            name
        } else { coding };
        let spec = self.coding_required(name)?;
        if buffer != Value::NIL && buffer != Value::TRUE {
            if !matches!(self.kind(buffer), Kind::Buffer | Kind::ByteBuffer) { return Err(Fault::Wrong("bufferp", buffer)); }
            if self.buffer_state(buffer).is_none_or(|state| !state.live) { return Err(Fault::Error("Selecting deleted buffer".into())); }
        }
        let attrs = self.vector_get(spec, num(0))?;
        let eol = self.vector_get(spec, num(2))?;
        let inhibit = self.intern("inhibit-eol-conversion");
        let inhibit = self.variable_value(inhibit)?.unwrap_or(Value::NIL) != Value::NIL;
        let unix = self.intern("unix");
        let fast = ascii_fast && source.bytes().is_ascii() && self.vector_get(attrs, num(5))? != Value::NIL
            && (eol == unix || inhibit || !source.bytes().contains(&if encode { b'\n' } else { b'\r' }));
        if fast && (buffer == Value::NIL || buffer == Value::TRUE) {
            self.variable_set(last, name)?;
            return if nocopy != Value::NIL { Ok(string) } else { Ok(self.make_string(source.bytes().to_vec(), !encode)) };
        }
        let kind = self.vector_get(attrs, num(3))?;
        // Valid UTF-8 with no CR decodes to itself, whatever the line-end
        // mode: buffer text is UTF-8 for these characters. Nothing else needs
        // looking at, which is most of what a visit decodes.
        if !encode && !source.is_multibyte() && kind == self.intern("utf-8") && !source.bytes().contains(&b'\r')
            && self.vector_get(attrs, num(24))? == Value::NIL
            && [6, 7, 9, 10].iter().try_fold(true, |plain, &index| Ok::<_, Fault>(plain && self.vector_get(attrs, num(index))? == Value::NIL))?
            && std::str::from_utf8(source.bytes()).is_ok() {
            // coding_eol: the first LF names the unix subsidiary.
            let unix = !inhibit && self.kind(eol) == Kind::Vector && source.bytes().contains(&b'\n');
            let precise = if unix { self.vector_get(eol, num(0))? } else { name };
            return self.coding_string_result(buffer, last, precise, source.bytes().to_vec(), true, Vec::new());
        }
        let tokens = if encode && kind == self.intern("utf-8") && !source.is_multibyte() {
            coding_encode_unibyte(source.bytes())
        } else { source.chars().map(|code| {
            if !source.is_multibyte() { CodingToken::Byte(code as u8) }
            else if code >= crate::lstring::BYTE8_OFFSET + 128 { CodingToken::Byte((code - crate::lstring::BYTE8_OFFSET) as u8) }
            else if code < 128 { CodingToken::Byte(code as u8) }
            else { CodingToken::Character(code) }
        }).collect::<Vec<_>>() };
        let bom = self.vector_get(attrs, num(24))?;
        let category = self.vector_get(attrs, num(14))?.integer_value().unwrap_or(0) as usize;
        let undecided = kind == self.intern("undecided");
        // decode_coding's detect_coding: an undecided system, or a BOM pair,
        // decodes as the system the text shows.
        if !encode && (undecided || (self.pair(bom).is_some() && matches!(category, CATEGORY_UTF_8_AUTO | CATEGORY_UTF_16_AUTO))) {
            let bytes = coding_detection_bytes(&tokens);
            if let Some(found) = self.coding_detected_system(spec, attrs, &bytes)? {
                return self.coding_convert(string, found, nocopy, buffer, false, false);
            }
        }
        let is = |this: &mut Self, name: &str| kind == this.intern(name);
        let utf8 = is(self, "utf-8");
        let charset = is(self, "charset");
        let utf16 = is(self, "utf-16");
        let iso = is(self, "iso-2022");
        let mule = is(self, "emacs-mule");
        let sjis = is(self, "shift-jis");
        let big5 = is(self, "big5");
        // setup_coding_system (coding.c:5706): an undecided type encodes
        // through encode_coding_raw_text, and decodes so when detection
        // found nothing.
        let raw = is(self, "raw-text") || undecided;
        if !utf8 && !charset && !utf16 && !raw && !iso && !mule && !sjis && !big5 {
            let type_name = self.symbol_name(kind)?;
            return Err(Fault::Error(format!("VM2 string conversion for the {type_name} coding type is not implemented")));
        }
        for index in [6, 7, 9, 10] {
            if self.vector_get(attrs, num(index))? != Value::NIL {
                return Err(Fault::Error("string coding translation tables and conversion hooks are not implemented".into()));
            }
        }
        // A BOM pair decodes as if the BOM were optional (detection found
        // none) and encodes with one.
        let bom = if utf8 || utf16 { bom } else { Value::NIL };
        // UTF-16 line ends are characters, found and converted after decoding.
        let tokens = if utf16 && !encode { self.coding_decode_utf16(attrs, bom, &tokens)? } else { tokens };
        let detected = if !encode && !inhibit && self.kind(eol) == Kind::Vector { coding_eol(&tokens) } else { None };
        let mode = if inhibit { 0 } else if let Some(mode) = detected { mode }
            else if eol == self.intern("dos") { 1 } else if eol == self.intern("mac") { 2 } else { 0 };
        let precise = if let Some(mode) = detected { self.vector_get(eol, num(mode as i64))? } else { name };
        let mut runs = Vec::new();
        let (bytes, multibyte) = if encode {
            let mut bytes = Vec::new();
            if utf8 && bom != Value::NIL { bytes.extend_from_slice(&[239, 187, 191]); }
            if utf16 {
                self.coding_encode_utf16(attrs, bom, &tokens, mode, &mut bytes)?;
                return self.coding_string_result(buffer, last, precise, bytes, false, Vec::new());
            }
            // consume_chars converts line ends before the encoder sees them.
            let tokens = coding_encode_eol(tokens, mode);
            if charset {
                self.coding_encode_charset(attrs, &tokens, &mut bytes)?;
            } else if iso {
                bytes.extend(self.coding_encode_iso_2022(attrs, &tokens, mode == 2)?);
            } else if mule {
                bytes.extend(self.coding_encode_emacs_mule(attrs, &tokens)?);
            } else if sjis || big5 {
                bytes.extend(self.coding_encode_double(attrs, &tokens, sjis)?);
            } else { for token in tokens { match token {
                CodingToken::Byte(byte) => bytes.push(byte),
                CodingToken::Character(code) => coding_emit(code, &mut bytes),
            }}}
            (bytes, false)
        } else {
            let tokens = if utf8 {
                coding_decode_utf8(&tokens, bom != Value::NIL, !inhibit && eol == self.intern("dos"))
                    .into_iter().map(|code| if code < 128 { CodingToken::Byte(code as u8) } else { CodingToken::Character(code) }).collect()
            } else { tokens };
            let tokens = coding_decode_eol(tokens, mode);
            let tokens = if charset || sjis || big5 || iso || mule {
                let (tokens, found) = if charset { self.coding_decode_charset(attrs, &tokens)? }
                    else if iso { self.coding_decode_iso_2022(attrs, &tokens)? }
                    else if mule { self.coding_decode_emacs_mule(&tokens)? }
                    else { self.coding_decode_double(attrs, &tokens, sjis)? };
                runs = found;
                tokens
            } else { tokens };
            let for_unibyte = !utf8 && self.vector_get(attrs, num(12))? != Value::NIL;
            let codes = tokens.into_iter().map(|token| match token {
                    CodingToken::Character(code) if for_unibyte => 0u32.wrapping_sub(code) & 255,
                    CodingToken::Character(code) => code,
                    CodingToken::Byte(byte) if for_unibyte || byte < 128 => u32::from(byte),
                    CodingToken::Byte(byte) => crate::lstring::BYTE8_OFFSET + u32::from(byte),
                });
            let mut bytes = Vec::new();
            for code in codes { if for_unibyte { bytes.push(code as u8); } else { coding_emit(code, &mut bytes); } }
            (bytes, !for_unibyte)
        };
        self.coding_string_result(buffer, last, precise, bytes, multibyte, runs)
    }
    fn coding_string_result(&mut self, buffer: Value, last: Value, precise: Value, bytes: Vec<u8>, multibyte: bool, runs: Vec<(usize, usize, usize)>) -> super::super::Out {
        // A BUFFER destination receives the string (prims/coding.rs).
        let _ = buffer;
        self.variable_set(last, precise)?;
        let result = self.make_string(bytes, multibyte);
        // produce_charset: each run of characters a non-ASCII charset decoded.
        let key = self.intern("charset");
        for (from, to, id) in runs {
            let name = self.vm.charsets.records[self.vm.charsets.ids[id]].attrs[1];
            let properties = self.coding_list(&[key, name]);
            self.add_text_properties(num(from as i64), num(to as i64), properties, result)?;
        }
        Ok(result)
    }
}

impl Leaf<'_> {
    /// coding.c decode_coding_utf_16: 16-bit units in the coding system's
    /// byte order after an optional BOM; a surrogate pair is one character,
    /// a broken one yields its two bytes as characters (and, as in GNU, a
    /// non-surrogate after it leaves the high half pending); an odd final
    /// byte stays a raw byte and a final pending high surrogate is dropped.
    fn coding_decode_utf16(&mut self, attrs: Value, bom: Value, tokens: &[CodingToken]) -> std::result::Result<Vec<CodingToken>, Fault> {
        let big = self.vector_get(attrs, num(25))? == self.intern("big");
        let unit = |a: u8, b: u8| if big { u32::from(a) << 8 | u32::from(b) } else { u32::from(b) << 8 | u32::from(a) };
        let char_token = |c: u32| if c < 128 { CodingToken::Byte(c as u8) } else { CodingToken::Character(c) };
        let mut at = 0;
        if bom == Value::TRUE {
            if let [CodingToken::Byte(a), CodingToken::Byte(b), ..] = tokens {
                if unit(*a, *b) == 0xFEFF { at = 2; }
            }
        }
        let (mut out, mut surrogate) = (Vec::with_capacity(tokens.len() / 2), 0);
        while at < tokens.len() {
            let CodingToken::Byte(c1) = tokens[at] else { out.push(tokens[at]); at += 1; continue; };
            let Some(&second) = tokens.get(at + 1) else { out.push(CodingToken::Byte(c1)); break; };
            at += 2;
            let CodingToken::Byte(c2) = second else { out.extend([CodingToken::Byte(c1), second]); continue; };
            let c = unit(c1, c2);
            if surrogate != 0 {
                if (0xDC00..=0xDFFF).contains(&c) {
                    out.push(CodingToken::Character(0x10000 + (((surrogate - 0xD800) << 10) | (c - 0xDC00))));
                    surrogate = 0;
                } else {
                    let (a, b) = if big { (surrogate >> 8, surrogate & 0xff) } else { (surrogate & 0xff, surrogate >> 8) };
                    out.extend([char_token(a), char_token(b)]);
                    if (0xD800..=0xDBFF).contains(&c) { surrogate = c; } else { out.push(char_token(c)); }
                }
            } else if (0xD800..=0xDBFF).contains(&c) {
                surrogate = c;
            } else {
                out.push(char_token(c));
            }
        }
        Ok(out)
    }
    /// coding.c encode_coding_utf_16: a BOM unless the system has none,
    /// then each character as one unit or a surrogate pair; raw bytes and
    /// other non-Unicode characters become the default character. Line ends
    /// convert as characters.
    fn coding_encode_utf16(&mut self, attrs: Value, bom: Value, tokens: &[CodingToken], mode: usize, bytes: &mut Vec<u8>) -> std::result::Result<(), Fault> {
        let big = self.vector_get(attrs, num(25))? == self.intern("big");
        let default = self.vector_get(attrs, num(11))?.integer_value().unwrap_or(32) as u32;
        let mut emit = |u: u32| {
            let pair = [(u >> 8) as u8, u as u8];
            bytes.extend(if big { pair } else { [pair[1], pair[0]] });
        };
        if bom != Value::NIL { emit(0xFEFF); }
        for &token in tokens {
            let c = match token {
                CodingToken::Byte(b) if b < 128 => u32::from(b),
                CodingToken::Byte(_) => default,
                CodingToken::Character(c) if c > 0x10FFFF => default,
                CodingToken::Character(c) => c,
            };
            if c == u32::from(b'\n') && mode != 0 {
                emit(u32::from(b'\r'));
                if mode == 2 { continue; }
            }
            if c < 0x10000 { emit(c); } else {
                let c = c - 0x10000;
                emit((c >> 10) + 0xD800);
                emit((c & 0x3FF) + 0xDC00);
            }
        }
        Ok(())
    }
    /// coding.c decode_coding_charset: each byte's charset-valids entry
    /// names the charsets (smallest dimension first) that may start a code
    /// there; the first that decodes the code's bytes wins, and a byte none
    /// decodes stays a raw byte.
    /// Also returns ADD_CHARSET_DATA's runs: (from, to, charset id) over
    /// output characters, a run continuing across undecodable bytes.
    fn coding_decode_charset(&mut self, attrs: Value, tokens: &[CodingToken]) -> std::result::Result<(Vec<CodingToken>, Vec<(usize, usize, usize)>), Fault> {
        let valids = self.vector_get(attrs, num(16))?;
        let ascii = self.named_charset_id("ascii")?;
        let (mut runs, mut last_id, mut last_offset) = (Vec::new(), ascii, 0);
        let mut out = Vec::with_capacity(tokens.len());
        let mut at = 0;
        while at < tokens.len() {
            let CodingToken::Byte(byte) = tokens[at] else { out.push(tokens[at]); at += 1; continue; };
            let entry = self.vector_get(valids, num(i64::from(byte)))?;
            let ids = if entry.integer_value().is_some() { vec![entry] } else { self.coding_items(entry)? };
            let mut decoded = None;
            for id in ids {
                let i = self.vm.charsets.ids[id.integer_value().unwrap() as usize];
                let dim = self.vm.charsets.records[i].dimension as usize;
                let Some(code) = tokens.get(at..at + dim).and_then(|bytes| bytes.iter().try_fold(0u32, |code, token| match token {
                    CodingToken::Byte(b) => Some(code << 8 | u32::from(*b)),
                    CodingToken::Character(_) => None,
                })) else { continue };
                if let Some(c) = self.charset_decode(i, code)? { decoded = Some((c, dim, id.integer_value().unwrap() as usize)); break; }
            }
            match decoded {
                Some((c, dim, id)) => {
                    if id != ascii && id != last_id {
                        if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
                        (last_id, last_offset) = (id, out.len());
                    }
                    out.push(CodingToken::Character(c));
                    at += dim;
                }
                None => { out.push(CodingToken::Byte(byte)); at += 1; }
            }
        }
        if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
        Ok((out, runs))
    }
    /// coding.c decode_coding_sjis (SJIS) or decode_coding_big5 over the
    /// EOL-decoded bytes: the charset list is (ROMAN KANA KANJI [KANJI2]) or
    /// (ROMAN BIG5); a byte that starts no valid code is a raw byte. Each
    /// run of one non-ASCII charset is recorded for its `charset' property.
    fn coding_decode_double(&mut self, attrs: Value, tokens: &[CodingToken], sjis: bool) -> std::result::Result<(Vec<CodingToken>, Vec<(usize, usize, usize)>), Fault> {
        let list: Vec<usize> = self.coding_items(self.vector_get(attrs, num(4))?)?.iter().filter_map(|v| v.integer_value()).map(|n| n as usize).collect();
        let ascii = self.named_charset_id("ascii")?;
        let byte = |at: usize| match tokens.get(at) { Some(CodingToken::Byte(b)) => Some(*b), _ => None };
        let (mut runs, mut last_id, mut last_offset) = (Vec::new(), ascii, 0);
        let mut out = Vec::with_capacity(tokens.len());
        let mut at = 0;
        while at < tokens.len() {
            let Some(c) = byte(at) else { out.push(tokens[at]); at += 1; continue };
            let trail = |at: usize| byte(at + 1).filter(|&c1| if sjis { !(c1 < 0x40 || c1 == 0x7F || c1 > 0xFC) } else { !(c1 < 0x40 || (0x7F..0xA1).contains(&c1) || c1 > 0xFE) });
            let decoded: Option<(usize, u32, usize)> = if c < 0x80 {
                Some((list[0], u32::from(c), 1))
            } else if sjis {
                match c {
                    0x80 | 0xA0 => None,
                    0xA1..=0xDF => Some((list[1], u32::from(c & 0x7F), 1)),
                    0x81..=0xEF => trail(at).map(|c1| (list[2], sjis_to_jis(u32::from(c) << 8 | u32::from(c1), false), 2)),
                    0xF0..=0xFC if list.len() > 3 => trail(at).map(|c1| (list[3], sjis_to_jis(u32::from(c) << 8 | u32::from(c1), true), 2)),
                    _ => None,
                }
            } else if (0xA1..=0xFE).contains(&c) {
                trail(at).map(|c1| (list[1], u32::from(c) << 8 | u32::from(c1), 2))
            } else { None };
            let decoded = match decoded {
                Some((id, code, width)) => self.charset_decode(self.vm.charsets.ids[id], code)?.map(|ch| (id, ch, width)),
                None => None,
            };
            match decoded {
                Some((id, ch, width)) => {
                    if id != ascii && id != last_id {
                        if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
                        (last_id, last_offset) = (id, out.len());
                    }
                    out.push(if ch < 128 { CodingToken::Byte(ch as u8) } else { CodingToken::Character(ch) });
                    at += width;
                }
                None => { out.push(CodingToken::Byte(c)); at += 1; }
            }
        }
        if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
        Ok((out, runs))
    }
    /// coding.c encode_coding_charset: ASCII as itself when the coding
    /// system is ASCII compatible, raw bytes as themselves, other characters
    /// by the first charset of the charset list that encodes them, else the
    /// default character.
    fn coding_encode_charset(&mut self, attrs: Value, tokens: &[CodingToken], bytes: &mut Vec<u8>) -> std::result::Result<(), Fault> {
        let ascii_compatible = self.vector_get(attrs, num(5))? != Value::NIL;
        let list = self.vector_get(attrs, num(4))?;
        let charsets: Vec<usize> = self.coding_items(list)?.into_iter()
            .map(|id| self.vm.charsets.ids[id.integer_value().unwrap() as usize]).collect();
        let default = self.vector_get(attrs, num(11))?.integer_value().unwrap_or(32) as u8;
        for &token in tokens {
            let c = match token {
                CodingToken::Byte(byte) if byte >= 128 || ascii_compatible => { bytes.push(byte); continue; }
                CodingToken::Byte(byte) => u32::from(byte),
                CodingToken::Character(c) => c,
            };
            let mut encoded = None;
            for &i in &charsets {
                if let Some(code) = self.charset_encode(i, c)? { encoded = Some((code, self.vm.charsets.records[i].dimension)); break; }
            }
            match encoded {
                Some((code, dim)) => bytes.extend((0..dim).rev().map(|n| (code >> (8 * n)) as u8)),
                None => bytes.push(default),
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) enum CodingToken { Byte(u8), Character(u32) }

fn coding_emit(code: u32, bytes: &mut Vec<u8>) {
    let mut encoded = [0; 5];
    let width = crate::lstring::char_string(code, &mut encoded);
    bytes.extend_from_slice(&encoded[..width]);
}

fn coding_eol(tokens: &[CodingToken]) -> Option<usize> {
    let mut cr = false;
    let mut crlf = false;
    for (at, token) in tokens.iter().enumerate() {
        match token {
            CodingToken::Byte(10) if at == 0 || !matches!(tokens[at - 1], CodingToken::Byte(13)) => return Some(0),
            CodingToken::Byte(10) => crlf = true,
            CodingToken::Byte(13) => cr = true,
            _ => {}
        }
    }
    if crlf { Some(1) } else if cr { Some(2) } else { None }
}

/// consume_chars's line ends: LF becomes CRLF (MODE 1) or CR (MODE 2).
fn coding_encode_eol(tokens: Vec<CodingToken>, mode: usize) -> Vec<CodingToken> {
    if mode == 0 { return tokens; }
    let mut result = Vec::with_capacity(tokens.len());
    for token in tokens {
        if matches!(token, CodingToken::Byte(b'\n')) {
            result.push(CodingToken::Byte(b'\r'));
            if mode == 2 { continue; }
        }
        result.push(token);
    }
    result
}

/// The bytes detection reads: raw bytes as themselves; a character as its
/// internal representation (DIVERGENCES.md VM2-CODING-DETECT-MULTIBYTE).
fn coding_detection_bytes(tokens: &[CodingToken]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(tokens.len());
    for &token in tokens {
        match token {
            CodingToken::Byte(byte) => bytes.push(byte),
            CodingToken::Character(code) => coding_emit(code, &mut bytes),
        }
    }
    bytes
}

fn coding_decode_eol(tokens: Vec<CodingToken>, mode: usize) -> Vec<CodingToken> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut at = 0;
    while at < tokens.len() {
        if matches!(tokens[at], CodingToken::Byte(13)) {
            if mode == 2 { result.push(CodingToken::Byte(10)); at += 1; continue; }
            if mode == 1 && tokens.get(at + 1).is_some_and(|t| matches!(t, CodingToken::Byte(10))) {
                result.push(CodingToken::Byte(10)); at += 2; continue;
            }
        }
        result.push(tokens[at]);
        at += 1;
    }
    result
}

/// Decoding BYTES with the `utf-8' coding system (no BOM, line ends
/// detected unless INHIBIT_EOL) as coding_convert_string does, without a
/// coding-system lookup: the internal bytes of the multibyte result.
pub(super) fn decode_utf8_detecting_eol(bytes: &[u8], inhibit_eol: bool) -> Vec<u8> {
    // Valid UTF-8 whose line ends stay as they are is its own internal form.
    if (inhibit_eol || !bytes.contains(&b'\r')) && std::str::from_utf8(bytes).is_ok() {
        return bytes.to_vec();
    }
    let tokens: Vec<CodingToken> = bytes.iter().map(|&b| CodingToken::Byte(b)).collect();
    let mode = if inhibit_eol { 0 } else { coding_eol(&tokens).unwrap_or(0) };
    let tokens = coding_decode_utf8(&tokens, false, false).into_iter()
        .map(|code| if code < 128 { CodingToken::Byte(code as u8) } else { CodingToken::Character(code) }).collect();
    let mut out = Vec::with_capacity(bytes.len());
    for token in coding_decode_eol(tokens, mode) {
        match token {
            CodingToken::Byte(byte) => coding_emit(u32::from(byte), &mut out),
            CodingToken::Character(code) => coding_emit(code, &mut out),
        }
    }
    out
}

// GNU consume_chars validates internal byte8 forms even in unibyte input.
fn coding_encode_unibyte(mut bytes: &[u8]) -> Vec<CodingToken> {
    let mut tokens = Vec::new();
    while !bytes.is_empty() {
        let (code, width) = crate::lstring::decode_char(bytes).unwrap_or((u32::from(bytes[0]), 1));
        tokens.push(if code >= crate::lstring::BYTE8_OFFSET + 128 {
            CodingToken::Byte((code - crate::lstring::BYTE8_OFFSET) as u8)
        } else if width == 1 { CodingToken::Byte(code as u8) }
        else { CodingToken::Character(code) });
        bytes = &bytes[width..];
    }
    tokens
}

fn coding_decode_utf8(tokens: &[CodingToken], bom: bool, dos: bool) -> Vec<u32> {
    let mut chars = Vec::<i32>::new();
    let mut at = if bom && matches!(tokens.get(..3), Some([CodingToken::Byte(239), CodingToken::Byte(187), CodingToken::Byte(191)])) { 3 } else { 0 };
    let mut after_cr = None;
    loop {
        // GNU saves src_base after the CR lookahead, including on retry/EOF.
        let base = at;
        let Some(token) = after_cr.take().or_else(|| {
            let token = tokens.get(at).copied()?;
            at += 1;
            Some(token)
        }) else { break; };
        let byte = match token {
            CodingToken::Character(code) => { chars.push(code as i32); continue; }
            CodingToken::Byte(byte) => byte,
        };
        if byte < 128 {
            if dos && byte == b'\r' {
                let Some(token) = tokens.get(at).copied() else {
                    coding_flush_utf8(&tokens[base..], &mut chars);
                    break;
                };
                at += 1;
                // ONE_MORE_BYTE represents ordinary multibyte chars negatively;
                // byte_after_cr only reuses its nonnegative byte values.
                if matches!(token, CodingToken::Byte(_)) { after_cr = Some(token); }
            }
            chars.push(i32::from(byte));
            continue;
        }
        let mut bytes = [0; 5];
        bytes[0] = byte;
        let width = if byte & 0xe0 == 0xc0 { 2 } else if byte & 0xf0 == 0xe0 { 3 }
            else if byte & 0xf8 == 0xf0 { 4 } else { 5 };
        let mut invalid = false;
        for byte in &mut bytes[1..width] {
            let Some(token) = tokens.get(at).copied() else {
                coding_flush_utf8(&tokens[base..], &mut chars);
                return coding_charbuf(&chars);
            };
            at += 1;
            if let CodingToken::Byte(next) = token && next & 0xc0 == 0x80 { *byte = next; }
            else { invalid = true; break; }
        }
        if !invalid && let Some(code) = coding_external_utf8(&bytes[..width]) {
            chars.push(code as i32);
        } else {
            at = base;
            if let Some(token) = tokens.get(at) {
                chars.push(match *token {
                    CodingToken::Byte(byte) if byte < 128 => i32::from(byte),
                    CodingToken::Byte(byte) => crate::lstring::BYTE8_OFFSET as i32 + i32::from(byte),
                    CodingToken::Character(code) => crate::lstring::BYTE8_OFFSET as i32 - code as i32,
                });
                at += 1;
            }
        }
    }
    coding_charbuf(&chars)
}

// External UTF-8 admits five-byte forms of byte8 character codes; the
// canonical internal string decoder correctly requires their two-byte forms.
fn coding_external_utf8(bytes: &[u8]) -> Option<u32> {
    let (minimum, mask) = match bytes {
        [0xc0..=0xdf, _] => (0x80, 0x1f),
        [0xe0..=0xef, _, _] => (0x800, 0x0f),
        [0xf0..=0xf7, _, _, _] => (0x10000, 0x07),
        [0xf8..=0xfb, _, _, _, _] => (0x200000, 0x03),
        _ => return None,
    };
    let mut code = u32::from(bytes[0] & mask);
    for &byte in &bytes[1..] {
        if byte & 0xc0 != 0x80 { return None; }
        code = (code << 6) | u32::from(byte & 0x3f);
    }
    (code >= minimum && code <= crate::lstring::MAX_CHAR && !(0xd800..=0xdfff).contains(&code)).then_some(code)
}

#[cfg(test)]
#[test]
fn external_utf8_byte8_forms_preserve_internal_canonical_validation() {
    for code in crate::lstring::MAX_5_BYTE_CHAR + 1..=crate::lstring::MAX_CHAR {
        let bytes = [0xf8, (0x80 | ((code >> 18) & 0x3f)) as u8, (0x80 | ((code >> 12) & 0x3f)) as u8,
            (0x80 | ((code >> 6) & 0x3f)) as u8, (0x80 | (code & 0x3f)) as u8];
        assert_eq!(coding_external_utf8(&bytes), Some(code));
        assert_eq!(crate::lstring::decode_char(&bytes), None);
    }
}

fn coding_flush_utf8(tokens: &[CodingToken], chars: &mut Vec<i32>) {
    for &token in tokens {
        match token {
            CodingToken::Byte(byte) if byte < 128 => chars.push(i32::from(byte)),
            CodingToken::Byte(byte) => chars.push(crate::lstring::BYTE8_OFFSET as i32 + i32::from(byte)),
            CodingToken::Character(code) => {
                let mut bytes = Vec::new();
                coding_emit(code, &mut bytes);
                chars.extend(bytes.into_iter().map(|byte| crate::lstring::BYTE8_OFFSET as i32 + i32::from(byte)));
            }
        }
    }
}

fn coding_charbuf(chars: &[i32]) -> Vec<u32> {
    let mut result = Vec::new();
    let mut at = 0;
    while let Some(&code) = chars.get(at) {
        if code >= 0 { result.push(code as u32); at += 1; }
        // GNU produce_chars treats negative charbuf entries as annotation spans.
        else { at += (-code) as usize; }
    }
    result
}

impl Leaf<'_> {
    pub fn coding_list(&mut self, items: &[Value]) -> Value {
        items
            .iter()
            .rev()
            .fold(Value::NIL, |tail, &v| self.cons(v, tail))
    }
    pub(super) fn coding_items(&self, mut list: Value) -> std::result::Result<Vec<Value>, Fault> {
        let original = list;
        let mut seen = HashSet::default();
        let mut result = Vec::new();
        while let Some((car, cdr)) = self.pair(list) {
            if !seen.insert(list.0) {
                return Err(Fault::Circular(original));
            }
            result.push(car);
            list = cdr;
        }
        if list != Value::NIL {
            return Err(Fault::Wrong("listp", list));
        }
        Ok(result)
    }
    fn coding_symbol(&self, v: Value) -> std::result::Result<(), Fault> {
        if self.kind(v) == Kind::Symbol {
            Ok(())
        } else {
            Err(Fault::Wrong("symbolp", v))
        }
    }
    fn coding_plist_get(&self, mut plist: Value, prop: Value) -> std::result::Result<Value, Fault> {
        let mut seen = HashSet::default();
        while let Some((key, rest)) = self.pair(plist) {
            if !seen.insert(plist.0) {
                return Err(Fault::Circular(plist));
            }
            let Some((value, tail)) = self.pair(rest) else {
                break;
            };
            if key == prop {
                return Ok(value);
            }
            plist = tail;
        }
        Ok(Value::NIL)
    }
    fn coding_plist_put(
        &mut self,
        plist: Value,
        prop: Value,
        value: Value,
    ) -> std::result::Result<Value, Fault> {
        let mut tail = plist;
        let mut last = None;
        let mut seen = HashSet::default();
        while let Some((key, rest)) = self.pair(tail) {
            if !seen.insert(tail.0) {
                return Err(Fault::Circular(plist));
            }
            let Some((_, next)) = self.pair(rest) else {
                return Err(Fault::Wrong("plistp", plist));
            };
            if key == prop {
                self.setcar(rest, value)?;
                return Ok(plist);
            }
            last = Some(rest);
            tail = next;
        }
        if tail != Value::NIL {
            return Err(Fault::Wrong("plistp", plist));
        }
        let new = self.coding_list(&[prop, value]);
        if let Some(last) = last {
            self.setcdr(last, new)?;
            Ok(plist)
        } else {
            Ok(new)
        }
    }
    pub fn coding_define_form(
        &mut self,
        symbol: Value,
        clear: bool,
    ) -> std::result::Result<Value, Fault> {
        let prop = self.intern("coding-system-define-form");
        let plist = self.symbol_plist(symbol)?;
        let value = self.coding_plist_get(plist, prop)?;
        if clear && value != Value::NIL {
            let plist = self.coding_plist_put(plist, prop, Value::NIL)?;
            self.set_symbol_plist(symbol, plist)?;
        }
        Ok(value)
    }
    fn coding_store(&mut self, name: Value, spec: Value) -> std::result::Result<(), Fault> {
        let slot = self.vm.slot(ROOT);
        let table = self.vm.slots[slot].value;
        self.vm
            .declaration_hash_put(table, name, spec)
            .map_err(Fault::Error)?;
        let slot = self.vm.slot("coding-system-list");
        let old = self.vm.slots[slot].value;
        self.vm.slots[slot].value = self.cons(name, old);
        let slot = self.vm.slot("coding-system-alist");
        let old = self.vm.slots[slot].value;
        let text = self.symbol_name(name)?;
        let mut tail = old;
        let mut found = false;
        let mut seen = HashSet::default();
        while let Some((item, rest)) = self.pair(tail) {
            if !seen.insert(tail.0) {
                return Err(Fault::Circular(old));
            }
            if let Some((key, _)) = self.pair(item) {
                if self.string_text(key).ok().as_deref() == Some(&text) {
                    found = true;
                    break;
                }
            }
            tail = rest;
        }
        if !found {
            if tail != Value::NIL {
                return Err(Fault::Wrong("listp", old));
            }
            let text = self.symbol_name_value(name)?;
            let entry = self.cons(text, Value::NIL);
            self.vm.slots[slot].value = self.cons(entry, old);
        }
        Ok(())
    }
    fn coding_subsidiaries(&mut self, name: Value) -> std::result::Result<Value, Fault> {
        let text = self.symbol_name(name)?;
        let values = ["unix", "dos", "mac"].map(|suffix| self.intern(&format!("{text}-{suffix}")));
        Ok(self.vector(&values))
    }
    fn coding_publish(
        &mut self,
        name: Value,
        attrs: Value,
        mut eol: Value,
    ) -> std::result::Result<(), Fault> {
        if eol == Value::NIL {
            eol = self.coding_subsidiaries(name)?;
            for (i, suffix) in ["unix", "dos", "mac"].iter().enumerate() {
                let child = self.vector_get(eol, num(i as i64))?;
                let aliases = self.cons(child, Value::NIL);
                let kind = self.intern(suffix);
                let spec = self.vector(&[attrs, aliases, kind]);
                self.coding_store(child, spec)?;
            }
        }
        let aliases = self.cons(name, Value::NIL);
        let spec = self.vector(&[attrs, aliases, eol]);
        self.coding_store(name, spec)
    }
    pub fn coding_init(&mut self) -> std::result::Result<(), Fault> {
        let slot = self.vm.slot(ROOT);
        if self.vm.slots[slot].value != Value::UNBOUND {
            return Ok(());
        }
        self.vm.specials.insert("inhibit-redisplay".into());
        let table = hash::HashTable::new(hash::Test::Eq, hash::Weakness::None, 65);
        self.vm.slots[slot].value = self
            .vm
            .allocate_object(data::Object::HashTable(Box::new(table)));
        for name in ["coding-system-list", "coding-system-alist"] {
            let slot = self.vm.slot(name);
            self.vm.slots[slot].value = Value::NIL;
            self.vm.specials.insert(name.into());
        }
        // The existing native condition catalog owns coding-system-error.
        // Do not overwrite user changes to its property list here.
        self.vm.ensure_charsets();
        let no_conversion = self.intern("no-conversion");
        let mut categories = Vec::new();
        for category in CATEGORIES {
            let name = format!("coding-category-{category}");
            categories.push(self.intern(&name));
            let slot = self.vm.slot(&name);
            self.vm.slots[slot].value = no_conversion;
        }
        // syms_of_coding: which argument of each operation names its target.
        let target_idx = self.intern("target-idx");
        for (operation, index) in [("insert-file-contents", 0), ("write-region", 2), ("call-process", 0),
                                   ("call-process-region", 2), ("start-process", 2), ("open-network-stream", 3)] {
            let operation = self.intern(operation);
            self.put_symbol_property(operation, target_idx, num(index))?;
        }
        let list = self.coding_list(&categories);
        let slot = self.vm.slot("coding-category-list");
        self.vm.slots[slot].value = list;
        self.vm.specials.insert("coding-category-list".into());
        // Exactly the two syms_of_coding definitions, before mule-conf.el.
        let mut args = [Value::NIL; 16];
        args[0] = no_conversion;
        args[1] = num(61);
        args[2] = self.intern("raw-text");
        args[4] = Value::TRUE;
        args[9] = num(0);
        args[10] = Value::TRUE;
        args[12] = self.intern("unix");
        let mut plist = Vec::new();
        for (key, value) in [
            (":name", args[0]),
            (":mnemonic", args[1]),
            (":coding-type", args[2]),
            (":ascii-compatible-p", args[4]),
            (":default-char", args[9]),
            (":for-unibyte", args[10]),
            (":docstring", Value::NIL),
            (":eol-type", args[12]),
        ] {
            plist.push(self.intern(key));
            plist.push(value);
        }
        plist[13] = self.string("Do no conversion.\n\nWhen you visit a file with this coding, the file is read into a\nunibyte buffer as is, thus each byte of a file is treated as a\ncharacter.");
        args[11] = self.coding_list(&plist);
        self.coding_define(&args[..13])?;
        args[0] = self.intern("undecided");
        args[1] = num(45);
        args[2] = args[0];
        let ascii = self.intern("ascii");
        args[3] = self.cons(ascii, Value::NIL);
        args[10] = Value::NIL;
        args[12] = Value::NIL;
        args[13] = num(0);
        args[14] = num(0);
        plist[1] = args[0];
        plist[3] = args[1];
        plist[5] = args[2];
        plist[8] = self.intern(":charset-list");
        plist[9] = args[3];
        plist[11] = Value::NIL;
        plist[13] = self.string("No conversion on encoding, automatic conversion on decoding.");
        plist[15] = Value::NIL;
        args[11] = self.coding_list(&plist);
        self.coding_define(&args)?;
        Ok(())
    }
    pub fn coding_spec(&mut self, name: Value) -> std::result::Result<Value, Fault> {
        self.coding_init()?;
        let slot = self.vm.slot(ROOT);
        self.vm
            .declaration_hash_get(self.vm.slots[slot].value, name)
            .map(|v| v.unwrap_or(Value::NIL))
            .map_err(Fault::Error)
    }
    pub fn coding_p(&mut self, name: Value) -> std::result::Result<Value, Fault> {
        if name == Value::NIL || self.coding_spec(name)? != Value::NIL {
            return Ok(Value::TRUE);
        }
        if self.kind(name) == Kind::Symbol && self.coding_define_form(name, false)? != Value::NIL {
            return Ok(Value::TRUE);
        }
        Ok(Value::NIL)
    }
    pub fn coding_check(&mut self, name: Value) -> std::result::Result<Value, Fault> {
        if self.coding_p(name)? != Value::NIL {
            Ok(name)
        } else {
            let condition = self.intern("coding-system-error");
            let data = self.cons(name, Value::NIL);
            Err(Fault::Signal(condition, data))
        }
    }
    fn coding_required(&mut self, name: Value) -> std::result::Result<Value, Fault> {
        let spec = self.coding_spec(name)?;
        if spec != Value::NIL {
            return Ok(spec);
        }
        self.coding_check(name)?;
        Err(Fault::Wrong("coding-system-p", name))
    }
    pub fn coding_access(
        &mut self,
        name: Value,
        operation: i64,
    ) -> std::result::Result<Value, Fault> {
        if name == Value::NIL && operation == 0 {
            return Ok(self.intern("no-conversion"));
        }
        let name = if name == Value::NIL {
            self.intern("no-conversion")
        } else {
            name
        };
        if operation == 3 {
            let spec = self.coding_spec(name)?;
            // Unlike GNU's unchecked access to lazy forms, missing specs never crash.
            if spec == Value::NIL {
                return Ok(Value::NIL);
            }
            let eol = self.vector_get(spec, num(2))?;
            if self.kind(eol) == Kind::Vector {
                return Ok(self.vector(&self.vector_values(eol)?));
            }
            return Ok(num(if eol == self.intern("unix") {
                0
            } else if eol == self.intern("dos") {
                1
            } else {
                2
            }));
        }
        let spec = self.coding_required(name)?;
        if operation == 2 {
            return self.vector_get(spec, num(1));
        }
        let attrs = self.vector_get(spec, num(0))?;
        self.vector_get(attrs, num(if operation == 0 { 0 } else { 13 }))
    }
    pub fn coding_alias(&mut self, alias: Value, name: Value) -> std::result::Result<Value, Fault> {
        self.coding_symbol(alias)?;
        let spec = self.coding_required(name)?;
        let mut aliases = self.vector_get(spec, num(1))?;
        let mut seen = HashSet::default();
        loop {
            if !seen.insert(aliases.0) {
                return Err(Fault::Circular(aliases));
            }
            let (_, next) = self.pair(aliases).ok_or(Fault::Wrong("listp", aliases))?;
            if next == Value::NIL {
                break;
            }
            aliases = next;
        }
        let new = self.cons(alias, Value::NIL);
        self.setcdr(aliases, new)?;
        let eol = self.vector_get(spec, num(2))?;
        if self.kind(eol) == Kind::Vector {
            let children = self.coding_subsidiaries(alias)?;
            for i in 0..3 {
                self.coding_alias(
                    self.vector_get(children, num(i))?,
                    self.vector_get(eol, num(i))?,
                )?;
            }
        }
        self.coding_store(alias, spec)?;
        Ok(Value::NIL)
    }
    fn coding_character(&self, value: Value, mnemonic: bool) -> std::result::Result<Value, Fault> {
        if mnemonic && matches!(self.kind(value), Kind::String | Kind::Bytes) {
            return Ok(num(self
                .string_char_at(value, 0)
                .map_or(0, |(c, _)| c as i64)));
        }
        if value
            .integer_value()
            .is_some_and(|n| (0..=0x3fffff).contains(&n))
        {
            Ok(value)
        } else {
            Err(Fault::Wrong("characterp", value))
        }
    }
    fn coding_ccl_valid(&mut self, program: Value) -> std::result::Result<bool, Fault> {
        if self.kind(program) != Kind::Vector {
            return Ok(false);
        }
        let values = self.vector_values(program)?;
        if values.len() <= 2 || values.len() > i32::MAX as usize {
            return Ok(false);
        }
        let mut resolved = Vec::with_capacity(values.len());
        for &value in &values {
            if value
                .integer_value()
                .is_some_and(|n| i32::try_from(n).is_ok())
            {
                resolved.push(value);
                continue;
            }
            let mut found = Value::NIL;
            if let Some((symbol, prop)) = self.pair(value) {
                if self.kind(symbol) != Kind::Symbol || self.kind(prop) != Kind::Symbol {
                    return Ok(false);
                }
                found = self.coding_plist_get(self.symbol_plist(symbol)?, prop)?;
            } else if self.kind(value) == Kind::Symbol {
                for key in [
                    "translation-table-id",
                    "code-conversion-map-id",
                    "ccl-program-idx",
                ] {
                    let prop = self.intern(key);
                    let candidate = self.coding_plist_get(self.symbol_plist(value)?, prop)?;
                    if candidate
                        .integer_value()
                        .is_some_and(|n| (0..=i32::MAX as i64).contains(&n))
                    {
                        found = candidate;
                        break;
                    }
                }
            }
            if !found
                .integer_value()
                .is_some_and(|n| (0..=i32::MAX as i64).contains(&n))
            {
                return Ok(false);
            }
            resolved.push(found);
        }
        Ok(resolved[0].integer_value().unwrap() >= 0
            && resolved[1]
                .integer_value()
                .is_some_and(|n| n >= 0 && n <= values.len() as i64))
    }
    fn coding_copy_sequence(&mut self, value: Value) -> std::result::Result<Value, Fault> {
        match self.kind(value) {
            Kind::Vector => Ok(self.vector(&self.vector_values(value)?)),
            Kind::String | Kind::Bytes => self.propertized_copy(value, &[]),
            Kind::Cons => {
                let values = self.coding_items(value)?;
                Ok(self.coding_list(&values))
            }
            _ if value == Value::NIL => Ok(Value::NIL),
            _ => Err(Fault::Wrong("sequencep", value)),
        }
    }
    fn coding_translation(&self, value: Value) -> std::result::Result<(), Fault> {
        if self.kind(value) == Kind::CharTable || self.pair(value).is_some() {
            Ok(())
        } else {
            self.coding_symbol(value)
        }
    }
    pub fn coding_put(
        &mut self,
        name: Value,
        prop: Value,
        mut value: Value,
    ) -> std::result::Result<Value, Fault> {
        let spec = self.coding_required(name)?;
        let attrs = self.vector_get(spec, num(0))?;
        let key_name = self.symbol_name(prop).unwrap_or_default();
        let key = if prop == self.intern(&key_name) {
            key_name.as_str()
        } else {
            ""
        };
        let index = match key {
            ":mnemonic" => {
                value = self.coding_character(value, true)?;
                Some(2)
            }
            ":default-char" => {
                value = if value == Value::NIL {
                    num(32)
                } else {
                    self.coding_character(value, false)?
                };
                Some(11)
            }
            ":decode-translation-table" => {
                self.coding_translation(value)?;
                Some(6)
            }
            ":encode-translation-table" => {
                self.coding_translation(value)?;
                Some(7)
            }
            ":post-read-conversion" => {
                self.coding_symbol(value)?;
                Some(9)
            }
            ":pre-write-conversion" => {
                self.coding_symbol(value)?;
                Some(10)
            }
            ":ascii-compatible-p" => Some(5),
            _ => None,
        };
        if let Some(index) = index {
            self.vector_set(attrs, num(index), value)?;
        }
        let plist = self.vector_get(attrs, num(13))?;
        let plist = self.coding_plist_put(plist, prop, value)?;
        self.vector_set(attrs, num(13), plist)?;
        Ok(value)
    }
    fn coding_charset(&mut self, name: Value) -> std::result::Result<usize, Fault> {
        self.vm.ensure_charsets();
        self.vm
            .charsets
            .lookup(name)
            .ok_or(Fault::Wrong("charsetp", name))
    }
    fn coding_short(&mut self, args: &[Value], min: usize) -> std::result::Result<(), Fault> {
        if args.len() < min {
            let symbol = self.intern("define-coding-system-internal");
            let condition = self.intern("wrong-number-of-arguments");
            let data = self.cons(symbol, num(args.len() as i64));
            Err(Fault::Signal(condition, data))
        } else {
            Ok(())
        }
    }
    pub fn coding_define(&mut self, args: &[Value]) -> std::result::Result<Value, Fault> {
        self.coding_define_inner(args, false)
    }
    pub fn coding_validate_prefix(&mut self, args: &[Value]) -> std::result::Result<Value, Fault> {
        self.coding_define_inner(args, true)
    }
    fn coding_define_inner(
        &mut self,
        args: &[Value],
        prefix_only: bool,
    ) -> std::result::Result<Value, Fault> {
        self.coding_init()?;
        self.coding_short(args, 13)?;
        let mut attrs = [Value::NIL; 30];
        self.coding_symbol(args[0])?;
        attrs[0] = args[0];
        attrs[2] = self.coding_character(args[1], true)?;
        self.coding_symbol(args[2])?;
        attrs[3] = args[2];
        let kind_name = self.symbol_name(args[2])?;
        let kind = if args[2] == self.intern(&kind_name) {
            kind_name.as_str()
        } else {
            ""
        };
        let special = self.kind(args[3]) == Kind::Symbol;
        let mut ids = Vec::new();
        if special {
            let iso = self.intern("iso-2022");
            let mule = self.intern("emacs-mule");
            if args[3] == iso || args[3] == mule {
                if args[3] != args[2] {
                    return Err(Fault::Error("Invalid charset-list".into()));
                }
                self.vm.ensure_charsets();
                let list = if args[3] == iso {
                    &self.vm.charsets.iso_list
                } else {
                    &self.vm.charsets.mule_list
                };
                ids = list.clone();
                let values: Vec<_> = ids.iter().map(|&i| num(i as i64)).collect();
                attrs[4] = self.coding_list(&values);
            } else {
                attrs[4] = args[3];
            }
        } else if self.pair(args[3]).is_none() {
            // GNU copies sequences but converts charset names only in conses.
            attrs[4] = self.coding_copy_sequence(args[3])?;
        } else {
            for name in self.coding_items(args[3])? {
                let record = self.coding_charset(name)?;
                let r = &self.vm.charsets.records[record];
                if (kind == "iso-2022" && r.iso_final < 0)
                    || (kind == "emacs-mule" && r.mule_id < 0)
                {
                    return Err(Fault::Error(format!(
                        "Can't handle charset `{}`",
                        self.symbol_name(name)?
                    )));
                }
                ids.push(r.attrs[0].integer_value().unwrap() as usize);
            }
            let values: Vec<_> = ids.iter().map(|&i| num(i as i64)).collect();
            attrs[4] = self.coding_list(&values);
        }
        let mut safe = vec![255; ids.iter().max().copied().unwrap_or(0) + 1];
        for &id in &ids {
            safe[id] = 0;
        }
        attrs[15] = self.byte_string(&safe);
        attrs[5] = args[4];
        self.coding_translation(args[5])?;
        attrs[6] = args[5];
        self.coding_translation(args[6])?;
        attrs[7] = args[6];
        self.coding_symbol(args[7])?;
        attrs[9] = args[7];
        self.coding_symbol(args[8])?;
        attrs[10] = args[8];
        attrs[11] = if args[9] == Value::NIL {
            num(32)
        } else {
            self.coding_character(args[9], false)?
        };
        attrs[12] = if args[10] == Value::NIL {
            Value::NIL
        } else {
            Value::TRUE
        };
        if args[11] != Value::NIL && self.pair(args[11]).is_none() {
            return Err(Fault::Wrong("listp", args[11]));
        }
        attrs[13] = args[11];
        let category = match kind {
            "charset" => {
                let mut valid: Vec<Vec<Value>> = vec![Vec::new(); 256];
                for &id in &ids {
                    let r = &self.vm.charsets.records[self.vm.charsets.ids[id]];
                    if r.ascii_compatible {
                        attrs[5] = Value::TRUE;
                    }
                    let dim = r.dimension as usize;
                    for byte in r.space[(dim - 1) * 2]..=r.space[(dim - 1) * 2 + 1] {
                        valid[byte as usize].push(num(id as i64));
                    }
                }
                let mut values = Vec::with_capacity(256);
                for mut entry in valid {
                    entry.sort_by_key(|id| {
                        self.vm.charsets.records
                            [self.vm.charsets.ids[id.integer_value().unwrap() as usize]]
                            .dimension
                    });
                    values.push(if entry.len() == 1 {
                        entry[0]
                    } else {
                        self.coding_list(&entry)
                    });
                }
                attrs[16] = self.vector(&values);
                14
            }
            "raw-text" => {
                attrs[5] = Value::TRUE;
                19
            }
            "emacs-mule" => {
                attrs[5] = Value::TRUE;
                if special && args[3] == self.intern("emacs-mule") {
                    attrs[26] = Value::TRUE;
                }
                18
            }
            "undecided" => {
                self.coding_short(args, 16)?;
                attrs[27..30].copy_from_slice(&args[13..16]);
                20
            }
            "utf-8" | "utf-16" => {
                self.coding_short(args, if kind == "utf-8" { 14 } else { 15 })?;
                if prefix_only {
                    return Ok(Value::NIL);
                }
                let bom = args[13];
                if bom != Value::NIL && bom != Value::TRUE {
                    let (a, b) = self.pair(bom).ok_or(Fault::Wrong("consp", bom))?;
                    for name in [a, b] {
                        self.coding_check(name)?;
                        if name == Value::NIL && self.coding_spec(name)? == Value::NIL {
                            return Err(Fault::Wrong("coding-system-p", name));
                        }
                    }
                }
                attrs[24] = bom;
                if kind == "utf-8" {
                    if bom == Value::NIL {
                        attrs[5] = Value::TRUE;
                    }
                    if self.pair(bom).is_some() {
                        6
                    } else if bom == Value::NIL {
                        7
                    } else {
                        8
                    }
                } else {
                    attrs[5] = Value::NIL;
                    self.coding_symbol(args[14])?;
                    let endian = if args[14] == Value::NIL {
                        self.intern("big")
                    } else {
                        args[14]
                    };
                    let little = self.intern("little");
                    let big = self.intern("big");
                    if endian != little && endian != big {
                        return Err(Fault::Error(format!(
                            "Invalid endian: {}",
                            self.symbol_name(endian)?
                        )));
                    }
                    attrs[25] = endian;
                    if self.pair(bom).is_some() {
                        9
                    } else if bom == Value::NIL {
                        if endian == big { 12 } else { 13 }
                    } else if endian == big {
                        10
                    } else {
                        11
                    }
                }
            }
            "shift-jis" | "big5" => {
                let sjis = kind == "shift-jis";
                if if sjis {
                    ids.len() != 3 && ids.len() != 4
                } else {
                    ids.len() != 2
                } {
                    return Err(Fault::Error(
                        if sjis {
                            "There should be three or four charsets"
                        } else {
                            "There should be just two charsets"
                        }
                        .into(),
                    ));
                }
                for (i, &id) in ids.iter().enumerate() {
                    let r = &self.vm.charsets.records[self.vm.charsets.ids[id]];
                    let dim = if i == 0 || (sjis && i == 1) { 1 } else { 2 };
                    if r.dimension != dim {
                        return Err(Fault::Error(format!(
                            "Dimension of charset {} is not {}",
                            self.symbol_name(r.attrs[1])?,
                            if dim == 1 { "one" } else { "two" }
                        )));
                    }
                    if i == 0 && r.ascii_compatible {
                        attrs[5] = Value::TRUE;
                    }
                }
                if sjis { 15 } else { 16 }
            }
            "iso-2022" => {
                self.coding_short(args, 17)?;
                let initial_copy = self.coding_copy_sequence(args[13])?;
                let mut initial = self.vector_values(initial_copy)?;
                for i in 0..4 {
                    let v = *initial
                        .get(i)
                        .ok_or(Fault::Range(args[13], num(i as i64)))?;
                    initial[i] = if v == Value::NIL {
                        num(-1)
                    } else {
                        let record = self.coding_charset(v)?;
                        let r = &self.vm.charsets.records[record];
                        if i == 0 && r.ascii_compatible {
                            attrs[5] = Value::TRUE;
                        }
                        r.attrs[0]
                    };
                }
                let (a, b) = self.pair(args[14]).ok_or(Fault::Wrong("consp", args[14]))?;
                for value in [a, b] {
                    if value.integer_value().is_none() {
                        return Err(Fault::Wrong("fixnump", value));
                    }
                }
                // GNU copies the outer request list, but rewrites the shared pairs.
                let request_copy = self.coding_copy_sequence(args[15])?;
                let requests = if self.pair(request_copy).is_some() {
                    self.coding_items(request_copy)?
                } else {
                    Vec::new()
                };
                for &entry in &requests {
                    let (name, reg) = self.pair(entry).ok_or(Fault::Wrong("consp", entry))?;
                    let record = self.coding_charset(name)?;
                    if !reg.integer_value().is_some_and(|n| (0..=3).contains(&n)) {
                        return Err(if reg.integer_value().is_none() {
                            Fault::Wrong("integerp", reg)
                        } else {
                            Fault::Range3(reg, num(0), num(3))
                        });
                    }
                    self.setcar(entry, self.vm.charsets.records[record].attrs[0])?;
                }
                let flags = args[16]
                    .integer_value()
                    .filter(|n| *n >= 0)
                    .ok_or(Fault::Wrong("wholenump", args[16]))?
                    & 0x7fffffff;
                let full = special && args[3] == self.intern("iso-2022");
                let flags = flags | if full { 0x100000 } else { 0 };
                if flags & 0x100000 != 0 && !full {
                    ids = self.vm.charsets.iso_list.clone();
                    let values: Vec<_> = ids.iter().map(|&id| num(id as i64)).collect();
                    attrs[4] = self.coding_list(&values);
                    safe = vec![255; ids.iter().max().copied().unwrap_or(0) + 1];
                    for &id in &ids {
                        let explicit = requests.iter().find_map(|&v| {
                            let (key, reg) = self.pair(v)?;
                            (key == num(id as i64)).then_some(reg.integer_value().unwrap())
                        });
                        let r = &self.vm.charsets.records[self.vm.charsets.ids[id]];
                        let reg = explicit.unwrap_or(if r.space[0] == 32 && r.space[1] == 127 {
                            b.integer_value().unwrap()
                        } else {
                            a.integer_value().unwrap()
                        });
                        if reg < 4 {
                            safe[id] = reg as u8;
                        }
                    }
                    attrs[15] = self.byte_string(&safe);
                }
                attrs[20] = self.vector(&initial);
                attrs[21] = args[14];
                attrs[22] = request_copy;
                attrs[23] = num(flags);
                let category = if flags & 8 != 0 {
                    if flags & 0x30 != 0 {
                        4
                    } else if full {
                        0
                    } else {
                        1
                    }
                } else {
                    let id = initial[1].integer_value().unwrap();
                    if flags & 0x10 != 0 || full || id < 0 {
                        5
                    } else if self.vm.charsets.records[self.vm.charsets.ids[id as usize]].dimension
                        == 1
                    {
                        2
                    } else {
                        3
                    }
                };
                if category != 2 && category != 3 {
                    attrs[5] = Value::NIL;
                }
                category
            }
            "ccl" => {
                self.coding_short(args, 16)?;
                // Registry validation does not execute CCL. Symbolic programs need
                // the real CCL registry; do not pretend an arbitrary symbol works.
                for (index, &program) in args[13..15].iter().enumerate() {
                    if !self.coding_ccl_valid(program)? {
                        return Err(Fault::Wrong("cclp", program));
                    }
                    attrs[17 + index] = self.vector(&self.vector_values(program)?);
                }
                let mut valid = [0; 256];
                let mut tail = args[15];
                let mut seen = HashSet::default();
                while let Some((v, next)) = self.pair(tail) {
                    if !seen.insert(tail.0) {
                        return Err(Fault::Circular(args[15]));
                    }
                    tail = next;
                    let (from, to) = if let Some(n) = v.integer_value() {
                        if !(0..=255).contains(&n) {
                            return Err(Fault::Range3(v, num(0), num(255)));
                        }
                        (n, n)
                    } else {
                        let (a, b) = self.pair(v).ok_or(Fault::Wrong("consp", v))?;
                        let a_num = a.integer_value().ok_or(Fault::Wrong("integerp", a))?;
                        if !(0..=255).contains(&a_num) {
                            return Err(Fault::Range3(a, num(0), num(255)));
                        }
                        let b_num = b.integer_value().ok_or(Fault::Wrong("integerp", b))?;
                        if !(a_num..=255).contains(&b_num) {
                            return Err(Fault::Range3(b, num(a_num), num(255)));
                        }
                        (a_num, b_num)
                    };
                    valid[from as usize..=to as usize].fill(1);
                }
                attrs[19] = self.byte_string(&valid);
                17
            }
            _ => {
                return Err(Fault::Error(format!(
                    "Invalid coding system type: {kind_name}"
                )));
            }
        };
        attrs[14] = num(category);
        let cat = self.intern(&format!(
            "coding-category-{}",
            CATEGORIES[category as usize]
        ));
        let prop = self.intern(":category");
        let pair = self.cons(cat, attrs[13]);
        attrs[13] = self.cons(prop, pair);
        let prop = self.intern(":ascii-compatible-p");
        let pair = self.cons(attrs[5], attrs[13]);
        attrs[13] = self.cons(prop, pair);
        let eol = args[12];
        if eol != Value::NIL && !["unix", "dos", "mac"].iter().any(|s| eol == self.intern(s)) {
            return Err(Fault::Error("Invalid eol-type".into()));
        }
        let attrs = self.vector(&attrs);
        self.coding_publish(args[0], attrs, eol)?;
        // Fdefine_coding_system_internal: the first definition in a category,
        // or a redefinition of that same system, sets the category up.
        let systems = self.coding_category_state(CATEGORY_SYSTEMS)?;
        let current = self.vector_get(systems, num(category))?;
        if current == Value::NIL || current == args[0] {
            self.vector_set(systems, num(category), args[0])?;
        }
        Ok(Value::NIL)
    }
    /// Hidden category vectors, created in syms_of_coding's initial state.
    fn coding_category_state(&mut self, name: &str) -> super::super::Out {
        let slot = self.vm.slot(name);
        let value = self.vm.slots[slot].value;
        if value != Value::UNBOUND { return Ok(value); }
        let initial: Vec<Value> = if name == PRIORITIES { (0..CATEGORIES.len() as i64).map(num).collect() }
            else { vec![Value::NIL; CATEGORIES.len()] };
        let vector = self.vector(&initial);
        self.vm.slots[slot].value = vector;
        Ok(vector)
    }
    /// coding.c Fset_coding_system_priority.
    pub fn set_coding_system_priority(&mut self, systems: &[Value]) -> super::super::Out {
        self.coding_init()?;
        let categories = self.coding_category_state(CATEGORY_SYSTEMS)?;
        let priorities = self.coding_category_state(PRIORITIES)?;
        let mut changed = [false; CATEGORIES.len()];
        let mut order = Vec::with_capacity(CATEGORIES.len());
        for &system in systems {
            let spec = self.coding_required(system)?;
            let attrs = self.vector_get(spec, num(0))?;
            let category = self.vector_get(attrs, num(14))?.integer_value().unwrap_or(0) as usize;
            if changed[category] { continue; }
            changed[category] = true;
            order.push(category);
            if self.vector_get(categories, num(category as i64))? != Value::NIL {
                self.vector_set(categories, num(category as i64), system)?;
            }
            let variable = self.intern(&format!("coding-category-{}", CATEGORIES[category]));
            self.variable_set(variable, system)?;
        }
        for i in 0..CATEGORIES.len() {
            let category = self.vector_get(priorities, num(i as i64))?.integer_value().unwrap_or(0) as usize;
            if !changed[category] { order.push(category); }
        }
        let mut names = Vec::with_capacity(order.len());
        for (i, &category) in order.iter().enumerate() {
            self.vector_set(priorities, num(i as i64), num(category as i64))?;
            names.push(self.intern(&format!("coding-category-{}", CATEGORIES[category])));
        }
        let list = self.coding_list(&names);
        let variable = self.intern("coding-category-list");
        self.variable_set(variable, list)?;
        Ok(Value::NIL)
    }
    /// Romacs: the categories' priority order from CATEGORIES, symbols as
    /// `coding-category-list' names them, then the rest in their order,
    /// with `coding-category-list' updated as coding.c updates it.  It
    /// gives back an order `coding-category-list' recorded, which
    /// `set-coding-system-priority' cannot for a category with no coding
    /// system set up (startup's locale replay, rlisp/romacs-loadup-bootstrap.el).
    pub fn set_coding_category_order(&mut self, categories: &[Value]) -> super::super::Out {
        self.coding_init()?;
        let priorities = self.coding_category_state(PRIORITIES)?;
        let mut order: Vec<usize> = Vec::with_capacity(CATEGORIES.len());
        for &category in categories {
            let name = self.symbol_name(category)?;
            let index = name.strip_prefix("coding-category-").and_then(|short| CATEGORIES.iter().position(|c| *c == short))
                .ok_or(Fault::Wrong("coding-category-p", category))?;
            if !order.contains(&index) { order.push(index); }
        }
        for i in 0..CATEGORIES.len() {
            let category = self.vector_get(priorities, num(i as i64))?.integer_value().unwrap_or(0) as usize;
            if !order.contains(&category) { order.push(category); }
        }
        let mut names = Vec::with_capacity(order.len());
        for (i, &category) in order.iter().enumerate() {
            self.vector_set(priorities, num(i as i64), num(category as i64))?;
            names.push(self.intern(&format!("coding-category-{}", CATEGORIES[category])));
        }
        let list = self.coding_list(&names);
        let variable = self.intern("coding-category-list");
        self.variable_set(variable, list)?;
        Ok(Value::NIL)
    }
    /// coding.c Fcoding_system_priority_list: the base names of the systems
    /// set up for each category, in priority order.
    pub fn coding_system_priority_list(&mut self, highestp: Value) -> super::super::Out {
        self.coding_init()?;
        let categories = self.coding_category_state(CATEGORY_SYSTEMS)?;
        let priorities = self.coding_category_state(PRIORITIES)?;
        let mut bases = Vec::new();
        for i in 0..CATEGORIES.len() {
            let category = self.vector_get(priorities, num(i as i64))?;
            let system = self.vector_get(categories, category)?;
            if system == Value::NIL { continue; }
            let spec = self.coding_spec(system)?;
            let attrs = self.vector_get(spec, num(0))?;
            let base = self.vector_get(attrs, num(0))?;
            if highestp != Value::NIL { return Ok(base); }
            bases.push(base);
        }
        Ok(self.coding_list(&bases))
    }
}

/// coding.c EOL_SEEN_* bits.
const EOL_SEEN_LF: u8 = 1;
const EOL_SEEN_CR: u8 = 2;
const EOL_SEEN_CRLF: u8 = 4;

/// coding.c detect_eol over BYTES: the first line end's type, a mixture
/// of CR and CRLF counting as CRLF and any other mixture as LF, looking at
/// most at MAX_EOL_CHECK_COUNT line ends. UTF16 reads 16-bit units, whose
/// high byte GNU always takes first (its byte-order test never holds).
fn detect_eol(bytes: &[u8], utf16: bool) -> u8 {
    let (mut seen, mut total, mut at) = (0u8, 0, 0);
    let step = if utf16 { 2 } else { 1 };
    while at + step <= bytes.len() {
        let (high, c) = if utf16 { (bytes[at], bytes[at + 1]) } else { (0, bytes[at]) };
        at += step;
        if high != 0 || (c != b'\n' && c != b'\r') { continue }
        let this = if c == b'\n' { EOL_SEEN_LF }
            else if utf16 && (at + 1 >= bytes.len() || bytes[at] != 0 || bytes[at + 1] != b'\n') { EOL_SEEN_CR }
            else if !utf16 && bytes.get(at) != Some(&b'\n') { EOL_SEEN_CR }
            else { at += step; EOL_SEEN_CRLF };
        if seen == 0 {
            seen = this;
        } else if seen != this {
            if matches!((seen, this), (EOL_SEEN_CR, EOL_SEEN_CRLF) | (EOL_SEEN_CRLF, EOL_SEEN_CR)) { seen = EOL_SEEN_CRLF; }
            else { seen = EOL_SEEN_LF; break }
        }
        total += 1;
        if total == 3 { break }
    }
    seen
}

/// adjust_coding_eol_type's choice for the EOL bits seen: the index of the
/// unix, dos or mac subsidiary, LF winning over CRLF over CR.
fn eol_index(seen: u8) -> Option<usize> {
    if seen & EOL_SEEN_LF != 0 { Some(0) } else if seen & EOL_SEEN_CRLF != 0 { Some(1) }
    else if seen & EOL_SEEN_CR != 0 { Some(2) } else { None }
}
const CATEGORY_ISO_7: usize = 0;
const CATEGORY_ISO_7_TIGHT: usize = 1;
const CATEGORY_ISO_8_1: usize = 2;
const CATEGORY_ISO_8_2: usize = 3;
const CATEGORY_ISO_7_ELSE: usize = 4;
const CATEGORY_ISO_8_ELSE: usize = 5;
const CATEGORY_UTF_8_AUTO: usize = 6;
const CATEGORY_UTF_8_NOSIG: usize = 7;
const CATEGORY_UTF_8_SIG: usize = 8;
const CATEGORY_UTF_16_AUTO: usize = 9;
const CATEGORY_UTF_16_BE: usize = 10;
const CATEGORY_UTF_16_LE: usize = 11;
const CATEGORY_UTF_16_BE_NOSIG: usize = 12;
const CATEGORY_UTF_16_LE_NOSIG: usize = 13;
const CATEGORY_CHARSET: usize = 14;
const CATEGORY_SJIS: usize = 15;
const CATEGORY_BIG5: usize = 16;
const CATEGORY_EMACS_MULE: usize = 18;
const CATEGORY_RAW_TEXT: usize = 19;
const MASK_UTF_8: u32 = 1 << CATEGORY_UTF_8_AUTO | 1 << CATEGORY_UTF_8_NOSIG | 1 << CATEGORY_UTF_8_SIG;
const MASK_UTF_16: u32 = 0b11111 << 9;
const MASK_ISO_7BIT: u32 = 1 << CATEGORY_ISO_7 | 1 << CATEGORY_ISO_7_TIGHT;
const MASK_ISO_8BIT: u32 = 1 << CATEGORY_ISO_8_1 | 1 << CATEGORY_ISO_8_2;
const MASK_ISO_7_ELSE: u32 = 1 << CATEGORY_ISO_7_ELSE;
const MASK_ISO_8_ELSE: u32 = 1 << CATEGORY_ISO_8_ELSE;
const MASK_ISO: u32 = MASK_ISO_7BIT | MASK_ISO_8BIT | MASK_ISO_7_ELSE | MASK_ISO_8_ELSE;
const MASK_ISO_ESCAPE: u32 = MASK_ISO_7BIT | MASK_ISO_7_ELSE | MASK_ISO_8_ELSE;
/// coding.c CATEGORY_MASK_ANY: every detecting category, raw-text and
/// undecided excluded.
const MASK_ANY: u32 = (1 << CATEGORY_RAW_TEXT) - 1;
/// coding.c CODING_ISO_FLAG_SINGLE_SHIFT, _LATIN_EXTRA, _FULL_SUPPORT.
const ISO_FLAG_SINGLE_SHIFT: i64 = 0x20;
const ISO_FLAG_LATIN_EXTRA: i64 = 0x1000;
const ISO_FLAG_FULL_SUPPORT: i64 = 0x10_0000;
const MAX_COMPOSITION_COMPONENTS: i64 = 16;

/// What detect_coding_iso_2022 reads of the ISO categories' coding
/// systems and the charset registry.
struct IsoContext {
    /// Per ISO category (iso-7 .. iso-8-else): its system's safe-charsets
    /// (coding_attr_safe_charsets, 255 for unsafe) and ISO flags; None
    /// when the category has no system.
    systems: [Option<(Vec<u8>, i64)>; 6],
    /// iso_charset_table: (DIMENSION, CHARS-96, FINAL) to charset id.
    iso: Vec<((i64, bool, i64), usize)>,
    /// Vlatin_extra_code_table's non-nil entries, when it is a vector.
    latin_extra: Option<Vec<bool>>,
    inhibit_ied: bool,
}

impl IsoContext {
    fn flags(&self, category: usize) -> i64 { self.systems[category].as_ref().map_or(0, |(_, f)| *f) }
    /// SAFE_CHARSET_P.
    fn safe(&self, category: usize, id: usize) -> bool {
        self.systems[category].as_ref().is_some_and(|(safe, _)| safe.get(id).is_some_and(|&r| r != 255))
    }
    fn charset(&self, dimension: i64, chars96: bool, final_char: u8) -> Option<usize> {
        self.iso.iter().find(|(k, _)| *k == (dimension, chars96, i64::from(final_char))).map(|(_, id)| *id)
    }
}

/// coding.c detect_coding_iso_2022 over BYTES from HEAD_ASCII.
fn detect_iso_2022(bytes: &[u8], head_ascii: usize, cx: &IsoContext, info: &mut DetectInfo) -> bool {
    info.checked |= MASK_ISO;
    let (mut at, mut single_shifting, mut rejected, mut found) = (head_ascii, false, 0u32, 0u32);
    let mut composition_count: i64 = -1;
    let next = |at: &mut usize| { let c = bytes.get(*at).copied(); *at += usize::from(c.is_some()); c };
    let complete = 'scan: loop {
        if rejected == MASK_ISO { break false }
        let Some(c) = next(&mut at) else { break true };
        let mut extra_latin = false;
        match c {
            0x1B => {
                if cx.inhibit_ied { continue }
                single_shifting = false;
                let Some(c) = next(&mut at) else { break true };
                if c == b'N' || c == b'O' {
                    single_shifting = true;
                    rejected |= MASK_ISO_7BIT | MASK_ISO_8BIT;
                } else if c == b'1' {
                    if (0..=MAX_COMPOSITION_COMPONENTS).contains(&composition_count) {
                        composition_count = -1;
                        found |= MASK_ISO;
                    }
                } else if (b'0'..=b'4').contains(&c) {
                    composition_count = 0;
                } else {
                    let invalid = |c: u8, rejected: &mut u32| if c >= 0x80 { *rejected |= MASK_ISO_7BIT | MASK_ISO_7_ELSE };
                    let id = if (b'('..=b'/').contains(&c) {
                        let Some(c1) = next(&mut at) else { break true };
                        match (c1 >= b' ' && c1 < 0x80).then(|| cx.charset(1, c >= b',', c1)).flatten() {
                            Some(id) => id,
                            None => { invalid(c1, &mut rejected); continue }
                        }
                    } else if c == b'$' {
                        let Some(c) = next(&mut at) else { break true };
                        if (b'@'..=b'B').contains(&c) {
                            match cx.charset(2, false, c) { Some(id) => id, None => continue }
                        } else if (b'('..=b'/').contains(&c) {
                            let Some(c1) = next(&mut at) else { break true };
                            match (c1 >= b' ' && c1 < 0x80).then(|| cx.charset(2, c >= b',', c1)).flatten() {
                                Some(id) => id,
                                None => { invalid(c1, &mut rejected); continue }
                            }
                        } else { invalid(c, &mut rejected); continue }
                    } else { invalid(c, &mut rejected); continue };
                    // A valid designation sequence.
                    rejected |= MASK_ISO_8BIT;
                    for category in [CATEGORY_ISO_7, CATEGORY_ISO_7_TIGHT, CATEGORY_ISO_7_ELSE, CATEGORY_ISO_8_ELSE] {
                        if cx.safe(category, id) { found |= 1 << category } else { rejected |= 1 << category }
                    }
                }
            }
            0x0E | 0x0F => {
                if cx.inhibit_ied { continue }
                single_shifting = false;
                rejected |= MASK_ISO_7BIT | MASK_ISO_8BIT;
            }
            0x9B => {
                single_shifting = false;
                rejected |= MASK_ISO_7BIT | MASK_ISO_7_ELSE;
                found |= MASK_ISO_8_ELSE;
                extra_latin = true;
            }
            0x8E | 0x8F => {
                if cx.inhibit_ied { continue }
                single_shifting = false;
                rejected |= MASK_ISO_7BIT | MASK_ISO_7_ELSE;
                for category in [CATEGORY_ISO_8_1, CATEGORY_ISO_8_2] {
                    if cx.flags(category) & ISO_FLAG_SINGLE_SHIFT != 0 { found |= 1 << category; single_shifting = true; }
                }
                extra_latin = !single_shifting;
            }
            c if c < 0x80 => {
                if composition_count >= 0 { composition_count += 1 }
                single_shifting = false;
            }
            c => {
                rejected |= MASK_ISO_7BIT | MASK_ISO_7_ELSE;
                if c >= 0xA0 {
                    found |= 1 << CATEGORY_ISO_8_1;
                    // An even run of 0xA0..0xFF codes may be ISO-8-2.
                    if !single_shifting && rejected & 1 << CATEGORY_ISO_8_2 == 0 {
                        let mut len = 1i64;
                        while let Some(&c) = bytes.get(at) {
                            if c < 0xA0 { break }
                            at += 1;
                            len += 1;
                        }
                        if len & 1 == 1 && at < bytes.len() {
                            rejected |= 1 << CATEGORY_ISO_8_2;
                            if composition_count >= 0 { composition_count += len }
                        } else {
                            found |= 1 << CATEGORY_ISO_8_2;
                            if composition_count >= 0 { composition_count += len / 2 }
                        }
                    }
                } else {
                    extra_latin = true;
                }
            }
        }
        if extra_latin {
            // check_extra_latin
            if !cx.latin_extra.as_ref().is_some_and(|table| table[usize::from(c)]) {
                rejected = MASK_ISO;
                continue 'scan;
            }
            if cx.flags(CATEGORY_ISO_8_1) & ISO_FLAG_LATIN_EXTRA != 0 { found |= 1 << CATEGORY_ISO_8_1 }
            else { rejected |= 1 << CATEGORY_ISO_8_1 }
            rejected |= 1 << CATEGORY_ISO_8_2;
        }
    };
    if !complete {
        info.rejected |= MASK_ISO;
        return false;
    }
    info.rejected |= rejected;
    info.found |= found & !rejected;
    true
}

/// coding.c detect_coding_utf_16 (the decoder's detection, without
/// CODING_MODE_LAST_BLOCK): a BOM finds a signature category; otherwise
/// the even and odd byte dispersion rejects the no-signature ones.
fn detect_utf_16(bytes: &[u8], info: &mut DetectInfo) -> bool {
    info.checked |= MASK_UTF_16;
    let mut pairs = bytes.chunks_exact(2).map(|p| (p[0], p[1]));
    let Some((c1, c2)) = pairs.next() else { return true };
    let (le, be) = (1 << CATEGORY_UTF_16_LE, 1 << CATEGORY_UTF_16_BE);
    let (be_nosig, le_nosig) = (1 << CATEGORY_UTF_16_BE_NOSIG, 1 << CATEGORY_UTF_16_LE_NOSIG);
    if (c1, c2) == (0xFF, 0xFE) {
        info.found |= le | 1 << CATEGORY_UTF_16_AUTO;
        info.rejected |= be | be_nosig | le_nosig;
        return true;
    }
    if (c1, c2) == (0xFE, 0xFF) {
        info.found |= be | 1 << CATEGORY_UTF_16_AUTO;
        info.rejected |= le | be_nosig | le_nosig;
        return true;
    }
    let (mut e, mut o) = ([false; 256], [false; 256]);
    let (mut e_num, mut o_num) = (1, 1);
    e[usize::from(c1)] = true;
    o[usize::from(c2)] = true;
    info.rejected |= 1 << CATEGORY_UTF_16_AUTO | be | le;
    while info.rejected & MASK_UTF_16 != MASK_UTF_16 {
        let Some((c1, c2)) = pairs.next() else { return true };
        if !e[usize::from(c1)] {
            e[usize::from(c1)] = true;
            e_num += 1;
            if e_num >= 128 { info.rejected |= be_nosig }
        }
        if !o[usize::from(c2)] {
            o[usize::from(c2)] = true;
            o_num += 1;
            if o_num >= 128 { info.rejected |= le_nosig }
        }
    }
    false
}

/// coding.h SJIS_TO_JIS, or SJIS_TO_JIS2 for JIS X 0213 plane 2.
fn sjis_to_jis(code: u32, plane2: bool) -> u32 {
    let (s1, s2) = (code >> 8, code & 0xFF);
    let j1 = if !plane2 {
        if s2 >= 0x9F { s1 * 2 - if s1 >= 0xE0 { 0x160 } else { 0xE0 } } else { s1 * 2 - if s1 >= 0xE0 { 0x161 } else { 0xE1 } }
    } else if s2 >= 0x9F {
        match s1 { 0xF0 => 0x28, 0xF1 => 0x24, 0xF2 => 0x2C, 0xF3 => 0x2E, _ => 0x6E + (s1 - 0xF4) * 2 }
    } else if s1 <= 0xF2 { 0x21 + (s1 - 0xF0) * 2 } else if s1 <= 0xF4 { 0x2D + (s1 - 0xF3) * 2 } else { 0x6F + (s1 - 0xF5) * 2 };
    let j2 = if s2 >= 0x9F { s2 - 0x7E } else { s2 - if s2 >= 0x7F { 0x20 } else { 0x1F } };
    j1 << 8 | j2
}

/// coding.c detect_coding_sjis; MAX_FIRST is 0xEF for a system of at most
/// three charsets, else 0xFC.
fn detect_sjis(bytes: &[u8], head_ascii: usize, max_first: u8, info: &mut DetectInfo) -> bool {
    info.checked |= 1 << CATEGORY_SJIS;
    let mut found = 0;
    let mut at = head_ascii;
    while let Some(&c) = bytes.get(at) {
        at += 1;
        if c < 0x80 { continue }
        if (0x81..=0x9F).contains(&c) || (0xE0..=max_first).contains(&c) {
            let Some(&c) = bytes.get(at) else { break };
            at += 1;
            if c < 0x40 || c == 0x7F || c > 0xFC { info.rejected |= 1 << CATEGORY_SJIS; return false }
            found = 1 << CATEGORY_SJIS;
        } else if (0xA0..0xE0).contains(&c) {
            found = 1 << CATEGORY_SJIS;
        } else {
            info.rejected |= 1 << CATEGORY_SJIS;
            return false;
        }
    }
    info.found |= found;
    true
}

/// coding.c detect_coding_big5. As GNU's, a bad second byte returns
/// without rejecting the category.
fn detect_big5(bytes: &[u8], head_ascii: usize, info: &mut DetectInfo) -> bool {
    info.checked |= 1 << CATEGORY_BIG5;
    let mut found = 0;
    let mut at = head_ascii;
    while let Some(&c) = bytes.get(at) {
        at += 1;
        if c < 0x80 { continue }
        if c >= 0xA1 {
            let Some(&c) = bytes.get(at) else { break };
            at += 1;
            if c < 0x40 || (0x7F..=0xA0).contains(&c) { return false }
            found = 1 << CATEGORY_BIG5;
        } else {
            info.rejected |= 1 << CATEGORY_BIG5;
            return false;
        }
    }
    info.found |= found;
    true
}

/// coding.c detect_coding_emacs_mule; MULE_BYTES is emacs_mule_bytes.
fn detect_emacs_mule(bytes: &[u8], head_ascii: usize, mule_bytes: &[u8; 256], info: &mut DetectInfo) -> bool {
    info.checked |= 1 << CATEGORY_EMACS_MULE;
    let mut found = 0;
    let mut at = head_ascii;
    let reject = |info: &mut DetectInfo| { info.rejected |= 1 << CATEGORY_EMACS_MULE; false };
    'scan: while let Some(&c) = bytes.get(at) {
        at += 1;
        let mut c = c;
        if c == 0x80 {
            // A composite character: skip its 0xA0.. bytes, needing more than 4.
            loop {
                let start = at;
                loop {
                    let Some(&b) = bytes.get(at) else { break 'scan };
                    at += 1;
                    c = b;
                    if c < 0xA0 { break }
                }
                if at - start <= 4 { return reject(info) }
                found = 1 << CATEGORY_EMACS_MULE;
                if c != 0x80 { break }
            }
        }
        if c < 0x80 {
            if c < 0x20 && matches!(c, 0x1B | 0x0E | 0x0F) { return reject(info) }
        } else {
            let mut more = i32::from(mule_bytes[usize::from(c)]) - 1;
            while more > 0 {
                let Some(&b) = bytes.get(at) else { break 'scan };
                at += 1;
                if b < 0xA0 { at -= 1; break }
                more -= 1;
            }
            if more != 0 { return reject(info) }
            found = 1 << CATEGORY_EMACS_MULE;
        }
    }
    info.found |= found;
    true
}

/// coding.c coding_detection_info.
#[derive(Default)]
struct DetectInfo { checked: u32, found: u32, rejected: u32 }

/// What detect_scan saw: the detectors' masks, whether a null byte or an
/// 8-bit byte occurred, the ASCII head, the EOL bits of the skip, and the
/// detecting categories in priority order with their systems.
struct DetectScan { info: DetectInfo, null_byte: bool, eight_bit: bool, head_ascii: usize, eol_seen: u8, order: Vec<(usize, Value)> }

/// coding.c detect_coding_utf_8 over BYTES, of which the first HEAD_ASCII
/// are ASCII. GNU detects before CODING_MODE_LAST_BLOCK is set, so a
/// sequence cut short by the end of the text does not reject.
fn detect_utf_8(bytes: &[u8], head_ascii: usize, info: &mut DetectInfo) -> bool {
    info.checked |= MASK_UTF_8;
    let mut at = head_ascii;
    let mut nchars = head_ascii;
    let mut bom = false;
    if at == 0 && bytes.len() > 3 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        bom = true;
        at = 3;
        nchars += 1;
    }
    'chars: loop {
        let Some(&c) = bytes.get(at) else { break };
        at += 1;
        if c < 0x80 {
            nchars += 1;
            if c == b'\r' && bytes.get(at) == Some(&b'\n') { at += 1; nchars += 1; }
            continue;
        }
        // Each continuation octet, then the lead forms of 2..5 octets; GNU's
        // 5-octet test also requires c < MAX_MULTIBYTE_LEADING_CODE (0xF8),
        // so no 5-octet form is accepted.
        for leading in [0xE0u8 & c == 0xC0, 0xF0 & c == 0xE0, 0xF8 & c == 0xF0, 0xFC & c == 0xF8 && c < 0xF8] {
            let Some(&next) = bytes.get(at) else { break 'chars };
            at += 1;
            if next & 0xC0 != 0x80 {
                info.rejected |= MASK_UTF_8;
                return false;
            }
            if leading { nchars += 1; continue 'chars; }
        }
        info.rejected |= MASK_UTF_8;
        return false;
    }
    if bom {
        info.found |= MASK_UTF_8;
    } else {
        info.rejected |= 1 << CATEGORY_UTF_8_SIG;
        if nchars < bytes.len() { info.found |= 1 << CATEGORY_UTF_8_AUTO | 1 << CATEGORY_UTF_8_NOSIG; }
    }
    true
}

impl Leaf<'_> {
    /// decode_coding_gap's ASCII fast path (coding.c:7928-7970): for
    /// all-ASCII BYTES and an ASCII-compatible coding with undecided EOL,
    /// check_ascii's EOL bits pick the subsidiary (an index into its EOL
    /// vector), which the string decoder does not do.
    pub(crate) fn coding_ascii_eol(&mut self, bytes: &[u8], attrs: Value) -> std::result::Result<Option<usize>, Fault> {
        if !bytes.is_ascii() || self.vector_get(attrs, num(5))? == Value::NIL
            || self.flag_variable("disable-ascii-optimization")? || self.flag_variable("inhibit-eol-conversion")? {
            return Ok(None);
        }
        let mut seen = 0;
        let mut at = 0;
        while at < bytes.len() {
            match bytes[at] {
                b'\r' if bytes.get(at + 1) == Some(&b'\n') => { seen |= EOL_SEEN_CRLF; at += 1; }
                b'\r' => seen |= EOL_SEEN_CR,
                b'\n' => seen |= EOL_SEEN_LF,
                _ => {}
            }
            at += 1;
        }
        Ok(eol_index(seen))
    }

    fn flag_variable(&mut self, name: &str) -> std::result::Result<bool, Fault> {
        let symbol = self.intern(name);
        Ok(self.variable_value(symbol)?.is_some_and(|v| v != Value::NIL))
    }

    /// coding.c detect_coding for a coding system with attribute vector ATTRS
    /// over BYTES: the system found, if any, and the EOL subsidiary index
    /// (adjust_coding_eol_type) its scan saw. Only the undecided type and
    /// the utf-8-auto category detect. Every category's detector is GNU's
    /// except CCL's, which a defined CCL-category system refuses.
    pub(crate) fn coding_detect(&mut self, bytes: &[u8], attrs: Value) -> std::result::Result<(Option<Value>, Option<usize>), Fault> {
        let refuse = |what: &str| Fault::Error(format!("VM2 coding detection of {what} is unsupported"));
        let kind = self.vector_get(attrs, num(3))?;
        let category = self.vector_get(attrs, num(14))?.integer_value().unwrap_or(0) as usize;
        if category == CATEGORY_UTF_8_AUTO && kind != self.intern("undecided") {
            let systems = self.vector_get(attrs, num(24))?;
            let Some((sig, nosig)) = self.pair(systems) else { return Ok((None, None)) };
            let head_ascii = bytes.iter().take_while(|&&b| b < 0x80).count();
            if head_ascii == bytes.len() { return Ok((Some(nosig), None)) }
            let mut info = DetectInfo::default();
            if !detect_utf_8(bytes, head_ascii, &mut info) { return Ok((None, None)) }
            return Ok((Some(if info.found & 1 << CATEGORY_UTF_8_SIG != 0 { sig } else { nosig }), None));
        }
        if category == CATEGORY_UTF_16_AUTO && kind != self.intern("undecided") {
            let systems = self.vector_get(attrs, num(24))?;
            let Some((le, be)) = self.pair(systems) else { return Ok((None, None)) };
            let mut info = DetectInfo::default();
            if !detect_utf_16(bytes, &mut info) { return Ok((None, None)) }
            return Ok((if info.found & 1 << CATEGORY_UTF_16_LE != 0 { Some(le) }
                else if info.found & 1 << CATEGORY_UTF_16_BE != 0 { Some(be) } else { None }, None));
        }
        if kind != self.intern("undecided") {
            return Ok((None, None));
        }
        let (scan, chosen) = self.detect_scan(bytes, attrs, true)?;
        let DetectScan { info, null_byte, eight_bit, head_ascii, eol_seen, order } = scan;
        if !null_byte && !eight_bit && head_ascii >= bytes.len() && info.found == 0 {
            return Ok((None, eol_index(eol_seen)));
        }
        let found = if let Some((category, system)) = chosen {
            if category == CATEGORY_UTF_8_AUTO || category == CATEGORY_UTF_16_AUTO {
                let attrs = self.coding_required(system)?;
                let attrs = self.vector_get(attrs, num(0))?;
                let systems = self.vector_get(attrs, num(24))?;
                match self.pair(systems) {
                    Some((sig, nosig)) if category == CATEGORY_UTF_8_AUTO => if info.found & 1 << CATEGORY_UTF_8_SIG != 0 { sig } else { nosig },
                    // coding.c: little-endian is the car, big-endian the cdr.
                    Some((le, be)) => if info.found & 1 << CATEGORY_UTF_16_LE != 0 { le }
                        else if info.found & 1 << CATEGORY_UTF_16_BE != 0 { be }
                        else { return Ok((None, eol_index(eol_seen))) },
                    None => system,
                }
            } else {
                system
            }
        } else if null_byte {
            self.intern("no-conversion")
        } else if info.rejected & MASK_ANY == MASK_ANY {
            self.intern("raw-text")
        } else if info.rejected != 0 {
            match order.iter().find(|(category, _)| info.rejected & 1 << category == 0) {
                Some(&(_, system)) => system,
                None => return Ok((None, eol_index(eol_seen))),
            }
        } else {
            return Ok((None, eol_index(eol_seen)));
        };
        Ok((Some(found), eol_index(eol_seen)))
    }

    /// The scan detect_coding and detect_coding_system share for the
    /// undecided system ATTRS over BYTES: the skip over ASCII (ISO-2022's
    /// escapes detected there), then each category's detector in priority
    /// order. With HIGHEST the first complete category found ends it, and
    /// is returned.
    fn detect_scan(&mut self, bytes: &[u8], attrs: Value, highest: bool) -> std::result::Result<(DetectScan, Option<(usize, Value)>), Fault> {
        let refuse = |what: &str| Fault::Error(format!("VM2 coding detection of {what} is unsupported"));
        // inhibit_flag (encode_inhibit_flag (ATTR), VARIABLE).
        let inhibit = |this: &mut Self, index: i64, variable: &str| -> std::result::Result<bool, Fault> {
            let flag = this.vector_get(attrs, num(index))?;
            let encoded = if flag == Value::NIL { -1 } else if flag == Value::TRUE { 1 } else { 0 };
            Ok(0 < encoded + i32::from(this.flag_variable(variable)?))
        };
        let inhibit_nbd = inhibit(self, 27, "inhibit-null-byte-detection")?;
        let inhibit_ied = inhibit(self, 28, "inhibit-iso-escape-detection")?;
        let prefer_utf_8 = self.vector_get(attrs, num(29))? != Value::NIL;
        let scan_eol = !self.flag_variable("disable-ascii-optimization")? && !self.flag_variable("inhibit-eol-conversion")?;
        let systems = self.coding_category_state(CATEGORY_SYSTEMS)?;
        let priorities = self.coding_category_state(PRIORITIES)?;
        let mut order = Vec::with_capacity(CATEGORY_RAW_TEXT);
        for i in 0..CATEGORY_RAW_TEXT {
            let category = self.vector_get(priorities, num(i as i64))?.integer_value().unwrap_or(0) as usize;
            order.push((category, self.vector_get(systems, num(category as i64))?));
        }
        // detect_coding_iso_2022 reads the global variable itself; the
        // system's attribute only gates the initial escape scan.
        let iso_inhibit = self.flag_variable("inhibit-iso-escape-detection")?;
        let iso = self.iso_context(systems, iso_inhibit)?;
        let mut info = DetectInfo::default();
        let (mut head_ascii, mut null_byte, mut eight_bit, mut eol_seen) = (0, false, false, 0u8);
        let mut at = 0;
        while at < bytes.len() {
            let c = bytes[at];
            if c & 0x80 != 0 {
                eight_bit = true;
                if null_byte { break }
            } else if c < 0x20 {
                if matches!(c, 0x1B | 0x0E | 0x0F) && !inhibit_ied && info.checked == 0 {
                    if detect_iso_2022(bytes, head_ascii, &iso, &mut info) {
                        // The whole text scanned: with no 8-bit code it is all ISO-2022.
                        if info.rejected & MASK_ISO_7_ELSE == 0 { head_ascii = bytes.len(); }
                        info.rejected |= !MASK_ISO_ESCAPE;
                        break;
                    }
                } else if c == 0 && !inhibit_nbd {
                    null_byte = true;
                    if eight_bit { break }
                } else if scan_eol {
                    if c == b'\r' {
                        if bytes.get(at + 1) == Some(&b'\n') {
                            eol_seen |= EOL_SEEN_CRLF;
                            at += 1;
                            if !eight_bit { head_ascii += 1; }
                        } else {
                            eol_seen |= EOL_SEEN_CR;
                        }
                    } else if c == b'\n' {
                        eol_seen |= EOL_SEEN_LF;
                    }
                }
                if !eight_bit { head_ascii += 1; }
            } else if !eight_bit {
                head_ascii += 1;
            }
            at += 1;
        }
        let mut chosen = None;
        if !null_byte && !eight_bit && head_ascii >= bytes.len() && info.found == 0 {
            return Ok((DetectScan { info, null_byte, eight_bit, head_ascii, eol_seen, order }, None));
        }
        if head_ascii >= bytes.len() {
            // All bytes 7-bit: only an ISO-2022 category can have been found.
            chosen = order.iter().copied().find(|&(category, _)| info.found & 1 << category != 0);
        } else {
            if null_byte {
                info.checked |= !MASK_UTF_16;
                info.rejected |= !MASK_UTF_16;
            } else if prefer_utf_8 && detect_utf_8(bytes, head_ascii, &mut info) {
                info.checked |= !MASK_UTF_8;
                info.rejected |= !MASK_UTF_8;
            }
            for &(category, system) in &order {
                let bit = 1u32 << category;
                if system == Value::NIL {
                    info.rejected |= bit;
                    continue;
                } else if category >= CATEGORY_RAW_TEXT {
                    continue;
                } else if info.checked & bit != 0 {
                    if highest && info.found & bit != 0 { chosen = Some((category, system)); break }
                    continue;
                }
                let complete = match category {
                    CATEGORY_ISO_7..=CATEGORY_ISO_8_ELSE => detect_iso_2022(bytes, head_ascii, &iso, &mut info),
                    CATEGORY_UTF_8_AUTO..=CATEGORY_UTF_8_SIG => detect_utf_8(bytes, head_ascii, &mut info),
                    CATEGORY_UTF_16_AUTO..=CATEGORY_UTF_16_LE_NOSIG => detect_utf_16(bytes, &mut info),
                    CATEGORY_CHARSET => self.detect_charset(system, bytes, head_ascii, &mut info)?,
                    CATEGORY_SJIS => {
                        let spec = self.coding_required(system)?;
                        let attrs = self.vector_get(spec, num(0))?;
                        let charsets = self.coding_items(self.vector_get(attrs, num(4))?)?.len();
                        detect_sjis(bytes, head_ascii, if charsets <= 3 { 0xEF } else { 0xFC }, &mut info)
                    }
                    CATEGORY_BIG5 => detect_big5(bytes, head_ascii, &mut info),
                    CATEGORY_EMACS_MULE => detect_emacs_mule(bytes, head_ascii, &self.emacs_mule_bytes(), &mut info),
                    _ => return Err(refuse(&format!("the {} category", CATEGORIES[category]))),
                };
                if highest && complete && info.found & bit != 0 { chosen = Some((category, system)); break }
            }
        }
        Ok((DetectScan { info, null_byte, eight_bit, head_ascii, eol_seen, order }, chosen))
    }


    /// coding.c Fdetect_coding_string.
    pub(crate) fn coding_detect_string(&mut self, string: Value, highest: Value) -> super::super::Out {
        let source = self.string_view(string)?;
        let bytes = if source.is_multibyte() {
            let mut bytes = Vec::with_capacity(source.bytes().len());
            for code in source.chars() {
                if code >= crate::lstring::BYTE8_OFFSET + 128 { bytes.push((code - crate::lstring::BYTE8_OFFSET) as u8); }
                else { coding_emit(code, &mut bytes); }
            }
            bytes
        } else { source.bytes().to_vec() };
        self.coding_detect_system(&bytes, highest != Value::NIL)
    }

    /// coding.c detect_coding_system for `undecided' over BYTES: the
    /// possible coding systems in priority order, each given the EOL type
    /// the text shows, or with HIGHEST the first of them (nil if none).
    pub(crate) fn coding_detect_system(&mut self, bytes: &[u8], highest: bool) -> super::super::Out {
        let undecided = self.intern("undecided");
        let spec = self.coding_required(undecided)?;
        let attrs = self.vector_get(spec, num(0))?;
        let (scan, chosen) = self.detect_scan(bytes, attrs, highest)?;
        let DetectScan { mut info, null_byte, order, .. } = scan;
        let systems = self.coding_category_state(CATEGORY_SYSTEMS)?;
        let mut val: Vec<Value> = Vec::new();
        if info.rejected & MASK_ANY == MASK_ANY || null_byte {
            info.found = 1 << CATEGORY_RAW_TEXT;
            val.push(self.intern("no-conversion"));
        } else if info.rejected == 0 && info.found == 0 {
            info.found = MASK_ANY;
            val.push(self.vector_get(systems, num(20))?);
        } else if highest {
            if info.found != 0 {
                // GNU keeps the loop's category, utf-16-auto's mask the
                // byte order it found, and its system.
                let (mut category, system) = chosen.or_else(|| order.last().copied()).unwrap_or((CATEGORY_RAW_TEXT, Value::NIL));
                if category == CATEGORY_UTF_16_AUTO {
                    category = if info.found & 1 << CATEGORY_UTF_16_LE != 0 { CATEGORY_UTF_16_LE } else { CATEGORY_UTF_16_BE };
                }
                info.found = 1 << category;
                val.push(system);
            } else if let Some(&(category, system)) = order.iter().find(|(category, _)| info.rejected & 1 << category == 0) {
                info.found = 1 << category;
                val.push(system);
            }
        } else {
            let mask = info.rejected | info.found;
            let mut extra = 0;
            for &(category, system) in order.iter().rev() {
                if mask & 1 << category == 0 {
                    extra |= 1 << category;
                    if system != Value::NIL { val = vec![system]; }
                }
            }
            let found: Vec<Value> = order.iter().filter(|(category, _)| info.found & 1 << category != 0).map(|&(_, system)| system).collect();
            val.splice(0..0, found);
            info.found |= extra;
        }
        // Then the EOL format of each system that leaves it open.
        let normal = (info.found & !MASK_UTF_16 != 0).then(|| if null_byte { EOL_SEEN_LF } else { detect_eol(bytes, false) });
        let be = (info.found & (1 << CATEGORY_UTF_16_BE | 1 << CATEGORY_UTF_16_BE_NOSIG) != 0).then(|| detect_eol(bytes, true));
        let le = (info.found & (1 << CATEGORY_UTF_16_LE | 1 << CATEGORY_UTF_16_LE_NOSIG) != 0).then(|| detect_eol(bytes, true));
        for name in val.iter_mut() {
            let spec = self.coding_required(*name)?;
            let attrs = self.vector_get(spec, num(0))?;
            let eol = self.vector_get(spec, num(2))?;
            if self.kind(eol) != Kind::Vector { continue }
            let category = self.vector_get(attrs, num(14))?.integer_value().unwrap_or(0) as usize;
            let seen = match category {
                CATEGORY_UTF_16_BE | CATEGORY_UTF_16_BE_NOSIG => be,
                CATEGORY_UTF_16_LE | CATEGORY_UTF_16_LE_NOSIG => le,
                _ => normal,
            };
            let index = match seen { Some(EOL_SEEN_LF) => 0, Some(EOL_SEEN_CRLF) => 1, Some(EOL_SEEN_CR) => 2, _ => continue };
            *name = self.vector_get(eol, num(index))?;
        }
        Ok(if highest { val.first().copied().unwrap_or(Value::NIL) } else { self.coding_list(&val) })
    }

    /// detect_coding's choice for decoding BYTES with the system NAME
    /// (spec SPEC, attributes ATTRS): the system found, given NAME's own
    /// EOL type when NAME specifies one; None when NAME decodes itself.
    pub(crate) fn coding_detected_system(&mut self, spec: Value, attrs: Value, bytes: &[u8]) -> std::result::Result<Option<Value>, Fault> {
        let (found, eol_seen) = self.coding_detect(bytes, attrs)?;
        let eols = self.vector_get(spec, num(2))?;
        let Some(mut found) = found else {
            // Nothing detected (ASCII): an undecided system still takes
            // the EOL the scan saw (adjust_coding_eol_type), so ASCII text
            // decodes as undecided-unix, as GNU reports it.
            if self.vector_get(attrs, num(3))? == self.intern("undecided") && self.kind(eols) == Kind::Vector
                && let Some(index) = eol_seen {
                return Ok(Some(self.vector_get(eols, num(index as i64))?));
            }
            return Ok(None);
        };
        if self.kind(eols) != Kind::Vector {
            let found_spec = self.coding_required(found)?;
            let found_eols = self.vector_get(found_spec, num(2))?;
            if self.kind(found_eols) == Kind::Vector {
                let index = ["unix", "dos", "mac"].iter().position(|e| eols == self.intern(e)).unwrap_or(0);
                found = self.vector_get(found_eols, num(index as i64))?;
            }
        }
        Ok(Some(found))
    }

    /// The ISO categories' systems, as detect_coding_iso_2022 sets them
    /// up (setup_iso_safe_charsets for a full-support system), and the
    /// registry's ISO charset table.
    fn iso_context(&mut self, systems: Value, inhibit_ied: bool) -> std::result::Result<IsoContext, Fault> {
        let mut found: [Option<(Vec<u8>, i64)>; 6] = Default::default();
        for (category, slot) in found.iter_mut().enumerate() {
            let system = self.vector_get(systems, num(category as i64))?;
            if system == Value::NIL { continue }
            let spec = self.coding_required(system)?;
            let attrs = self.vector_get(spec, num(0))?;
            let flags = self.vector_get(attrs, num(23))?.integer_value().unwrap_or(0);
            let safe = self.vector_get(attrs, num(15))?;
            let safe = if flags & ISO_FLAG_FULL_SUPPORT == 0 && matches!(self.kind(safe), Kind::String | Kind::Bytes) {
                self.string_view(safe)?.bytes().to_vec()
            } else {
                self.iso_safe_charsets(attrs, flags)?
            };
            *slot = Some((safe, flags));
        }
        let latin = self.intern("latin-extra-code-table");
        let latin_extra = match self.variable_value(latin)?.filter(|&table| self.kind(table) == Kind::Vector) {
            Some(table) => Some((0..256).map(|i| self.vector_get(table, num(i)).map(|v| v != Value::NIL)).collect::<std::result::Result<Vec<_>, _>>()?),
            None => None,
        };
        Ok(IsoContext { systems: found, iso: self.vm.charsets.iso.clone(), latin_extra, inhibit_ied })
    }

    /// coding.c setup_iso_safe_charsets: the register each charset of the
    /// system's list (every ISO charset for a full-support system) is
    /// designated to, 255 where none.
    pub(super) fn iso_safe_charsets(&mut self, attrs: Value, flags: i64) -> std::result::Result<Vec<u8>, Fault> {
        let ids: Vec<usize> = if flags & ISO_FLAG_FULL_SUPPORT != 0 {
            self.vm.charsets.iso.iter().map(|(_, id)| *id).collect()
        } else {
            self.coding_items(self.vector_get(attrs, num(4))?)?.iter().filter_map(|v| v.integer_value()).map(|n| n as usize).collect()
        };
        let mut safe = vec![255u8; ids.iter().max().map_or(0, |m| m + 1)];
        let request = self.vector_get(attrs, num(22))?;
        let usage = self.vector_get(attrs, num(21))?;
        let (reg94, reg96) = self.pair(usage).map_or((4, 4), |(a, b)| (a.integer_value().unwrap_or(4), b.integer_value().unwrap_or(4)));
        for id in ids {
            let mut reg = None;
            let mut tail = request;
            while let Some((entry, rest)) = self.pair(tail) {
                if let Some((key, value)) = self.pair(entry) && key.integer_value() == Some(id as i64) { reg = value.integer_value(); break }
                tail = rest;
            }
            let chars96 = { let r = &self.vm.charsets.records[self.vm.charsets.ids[id]]; r.space[1] - r.space[0] + 1 == 96 };
            let reg = reg.or_else(|| { let r = if chars96 { reg96 } else { reg94 }; (r < 4).then_some(r) });
            if let Some(reg) = reg { safe[id] = reg as u8; }
        }
        Ok(safe)
    }

    /// coding.c emacs_mule_bytes: 1 by default, the private leading codes,
    /// and each charset with an emacs-mule id (charset.c:1171).
    pub(super) fn emacs_mule_bytes(&self) -> [u8; 256] {
        let mut bytes = [1u8; 256];
        bytes[0x9A] = 3; bytes[0x9B] = 3; bytes[0x9C] = 4; bytes[0x9D] = 4;
        for &i in &self.vm.charsets.ids {
            let record = &self.vm.charsets.records[i];
            if (0..256).contains(&record.mule_id) {
                bytes[record.mule_id as usize] = (record.dimension + if record.mule_id < 0xA0 { 1 } else { 2 }) as u8;
            }
        }
        bytes
    }

    /// coding.c detect_coding_charset over BYTES for SYSTEM, the system set
    /// up for the charset category: every code must start at a byte its
    /// charset-valids entry accepts and fit the code space of one of the
    /// entry's charsets; an 8-bit byte makes it found. For an `iso-8859-' or
    /// `iso-latin-' system a C1 byte also needs `latin-extra-code-table'.
    fn detect_charset(&mut self, system: Value, bytes: &[u8], head_ascii: usize, info: &mut DetectInfo) -> std::result::Result<bool, Fault> {
        info.checked |= 1 << CATEGORY_CHARSET;
        let spec = self.coding_required(system)?;
        let attrs = self.vector_get(spec, num(0))?;
        let valids = self.vector_get(attrs, num(16))?;
        let name = self.symbol_name(system)?;
        let check_latin_extra = name.starts_with("iso-8859-") || name.starts_with("iso-latin-");
        let symbol = self.intern("latin-extra-code-table");
        let table = self.variable_value(symbol)?.filter(|&table| self.kind(table) == Kind::Vector);
        let mut at = if self.vector_get(attrs, num(5))? != Value::NIL { head_ascii } else { 0 };
        let mut found = false;
        let accepted = 'codes: loop {
            let Some(&c) = bytes.get(at) else { break true };
            at += 1;
            let entry = self.vector_get(valids, num(i64::from(c)))?;
            if entry == Value::NIL { break false }
            if c >= 0x80 {
                if c < 0xA0 && check_latin_extra {
                    let extra = match table { Some(table) => self.vector_get(table, num(i64::from(c)))?, None => Value::NIL };
                    if extra == Value::NIL { break false }
                }
                found = true;
            }
            // A charset id, or a list of them, smallest dimension first.
            // IDX carries across the list: a later charset continues after
            // the bytes an earlier one consumed, as GNU's loop does.
            let ids = if entry.integer_value().is_some() { vec![entry] } else { self.coding_items(entry)? };
            let mut idx = 1;
            for id in ids {
                let record = &self.vm.charsets.records[self.vm.charsets.ids[id.integer_value().unwrap() as usize]];
                let (dim, space) = (record.dimension as usize, record.space);
                while idx < dim {
                    // too_short
                    let Some(&c) = bytes.get(at) else { break 'codes false };
                    at += 1;
                    let byte = (dim - 1 - idx) * 2;
                    if !(space[byte]..=space[byte + 1]).contains(&u32::from(c)) { break }
                    idx += 1;
                }
                if idx == dim { continue 'codes }
            }
            break false
        };
        let bit = 1 << CATEGORY_CHARSET;
        if !accepted { info.rejected |= bit; } else { info.found |= if found { bit } else { 0 }; }
        Ok(accepted)
    }
}
