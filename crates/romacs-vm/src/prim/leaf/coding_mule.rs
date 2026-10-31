//! coding.c's charset-switching codecs over the coding owner's tokens
//! (prim/leaf/coding.rs): ISO-2022 (decode_coding_iso_2022,
//! encode_coding_iso_2022), emacs-mule (decode_coding_emacs_mule,
//! encode_coding_emacs_mule), and the SJIS and Big5 encoders. Line ends
//! are converted around them by the owner, as GNU's decode_eol and
//! consume_chars do. Composition sequences signal (DIVERGENCES.md
//! VM2-CODING-COMPOSITION).
use super::*;
use super::coding::CodingToken;

fn num(n: i64) -> Value {
    Value::integer(n).unwrap()
}

/// The output of a decoder: tokens and ADD_CHARSET_DATA's runs (from, to,
/// charset id) over them.
type Decoded = (Vec<CodingToken>, Vec<(usize, usize, usize)>);

// coding.c CODING_ISO_FLAG_*.
const LONG_FORM: i64 = 0x1;
const RESET_AT_EOL: i64 = 0x2;
const RESET_AT_CNTL: i64 = 0x4;
const SEVEN_BITS: i64 = 0x8;
const LOCKING_SHIFT: i64 = 0x10;
const SINGLE_SHIFT: i64 = 0x20;
const DESIGNATION: i64 = 0x40;
const REVISION: i64 = 0x80;
const DIRECTION: i64 = 0x100;
const INIT_AT_BOL: i64 = 0x200;
const DESIGNATE_AT_BOL: i64 = 0x400;
const SAFE: i64 = 0x800;
const COMPOSITION: i64 = 0x2000;
const USE_ROMAN: i64 = 0x8000;
const USE_OLDJIS: i64 = 0x10000;
const LEVEL_4: i64 = 0x20000;
const FULL_SUPPORT: i64 = 0x10_0000;
const ESC: i32 = 0x1B;

fn composition_refusal(what: &str) -> Fault {
    Fault::Error(format!("VM2 {what} composition sequences are not implemented"))
}

/// The bytes of a token stream, as ONE_MORE_BYTE reads them: a byte, or a
/// multibyte source character negated.
struct Source<'a> {
    tokens: &'a [CodingToken],
    at: usize,
}

impl Source<'_> {
    fn next(&mut self) -> Option<i32> {
        let token = *self.tokens.get(self.at)?;
        self.at += 1;
        Some(match token {
            CodingToken::Byte(b) => i32::from(b),
            CodingToken::Character(c) => -(c as i32),
        })
    }
}

/// A decoded character as a token: ASCII stays a byte.
fn char_token(c: u32) -> CodingToken {
    if c < 128 { CodingToken::Byte(c as u8) } else { CodingToken::Character(c) }
}

/// The invocation and designation state of coding_system.spec.iso_2022.
struct IsoState {
    flags: i64,
    initial: [i32; 4],
    designation: [i32; 4],
    invocation: [i32; 2],
    single_shifting: bool,
    /// CODING_ISO_REQUEST: the register each charset id requests, 255 for none.
    safe: Vec<u8>,
}

impl IsoState {
    /// CODING_ISO_INVOKED_CHARSET.
    fn invoked(&self, plane: usize) -> i32 {
        match self.invocation[plane] { r if r < 0 => -1, r => self.designation[r as usize] }
    }
    fn request(&self, id: usize) -> Option<usize> {
        self.safe.get(id).copied().filter(|&r| r != 255).map(usize::from)
    }
}

impl Leaf<'_> {
    /// setup_coding_system's ISO-2022 state for ATTRS.
    fn iso_state(&mut self, attrs: Value) -> std::result::Result<IsoState, Fault> {
        let flags = self.vector_get(attrs, num(23))?.integer_value().unwrap_or(0);
        let initial_vector = self.vector_get(attrs, num(20))?;
        let mut initial = [-1; 4];
        for (reg, slot) in initial.iter_mut().enumerate() {
            *slot = self.vector_get(initial_vector, num(reg as i64))?.integer_value().unwrap_or(-1) as i32;
        }
        let safe = self.vector_get(attrs, num(15))?;
        let safe = if flags & FULL_SUPPORT == 0 && matches!(self.kind(safe), Kind::String | Kind::Bytes) {
            self.string_view(safe)?.bytes().to_vec()
        } else {
            self.iso_safe_charsets(attrs, flags)?
        };
        Ok(IsoState {
            flags,
            initial,
            designation: initial,
            invocation: [0, if flags & SEVEN_BITS != 0 { -1 } else { 1 }],
            single_shifting: false,
            safe,
        })
    }

    fn charset_record_of(&self, id: usize) -> &crate::charset::Record {
        &self.vm.charsets.records[self.vm.charsets.ids[id]]
    }
    fn charset_chars96(&self, id: usize) -> bool {
        let r = self.charset_record_of(id);
        r.space[1] - r.space[0] + 1 == 96
    }

    /// coding.c decode_coding_iso_2022 over EOL-decoded TOKENS.
    pub(super) fn coding_decode_iso_2022(&mut self, attrs: Value, tokens: &[CodingToken]) -> std::result::Result<Decoded, Fault> {
        let mut iso = self.iso_state(attrs)?;
        let ascii = self.named_charset_id("ascii")? as i32;
        let (jis_roman, jis_1978, jis_0208) = (self.vm.charsets.jis_roman, self.vm.charsets.jis_1978, self.vm.charsets.jis_0208);
        let table = self.vm.charsets.iso.clone();
        let mut src = Source { tokens, at: 0 };
        let mut out = Vec::with_capacity(tokens.len());
        let (mut runs, mut last_id, mut last_offset) = (Vec::new(), ascii, 0);
        let (mut extsegment, mut embedded_utf8) = (0usize, false);
        let raw = |c: i32| if c < 0 { CodingToken::Character((-c) as u32) } else { CodingToken::Byte(c as u8) };
        // DECODE_DESIGNATION; false when the designation is invalid (or
        // ASCII after an invalid one), which is invalid code.
        let designate = |iso: &mut IsoState, reg: usize, dim: i64, chars96: bool, final_char: i32| -> bool {
            let id = (0x30..128).contains(&final_char)
                .then(|| table.iter().find(|(k, _)| *k == (dim, chars96, i64::from(final_char))).map(|(_, id)| *id))
                .flatten()
                .filter(|&id| iso.request(id).is_some());
            let Some(mut id) = id else { iso.designation[reg] = -2; return false };
            let prev = iso.designation[reg];
            if Some(id) == jis_roman && iso.flags & USE_ROMAN != 0 { id = ascii as usize; }
            else if Some(id) == jis_1978 && iso.flags & USE_OLDJIS != 0 { id = jis_0208.unwrap_or(id); }
            iso.designation[reg] = id as i32;
            !(prev == -2 && id == ascii as usize)
        };
        'main: loop {
            let base = src.at;
            let Some(mut c1) = src.next() else { break };
            macro_rules! more { () => { match src.next() { Some(c) => c, None => { src.at = base; break 'main } } } }
            macro_rules! invalid { () => {{
                src.at = base;
                out.push(raw(src.next().unwrap()));
                iso.invocation[0] = 0;
                iso.designation[0] = ascii;
                continue 'main;
            }} }
            if c1 < 0 { invalid!(); }
            if extsegment > 0 {
                out.push(raw(c1));
                extsegment -= 1;
                continue;
            }
            if embedded_utf8 {
                if c1 == ESC {
                    if src.at + 1 >= tokens.len() { src.at = base; break }
                    out.push(CodingToken::Byte(ESC as u8));
                    if matches!(tokens[src.at..src.at + 2], [CodingToken::Byte(b'%'), CodingToken::Byte(b'@')]) {
                        src.at += 2;
                        out.extend([CodingToken::Byte(b'%'), CodingToken::Byte(b'@')]);
                        embedded_utf8 = false;
                    }
                } else {
                    out.push(raw(c1));
                }
                continue;
            }
            let (id0, id1) = (iso.invoked(0), iso.invoked(1));
            let charset: i32 = match c1 {
                0x20 | 0x7F => if id0 < 0 || !self.charset_chars96(id0 as usize) { ascii } else { id0 },
                0x21..=0x7E => if id0 < 0 { ascii } else { id0 },
                0xA0 | 0xFF | 0xA1..=0xFE => {
                    if matches!(c1, 0xA0 | 0xFF) && (id1 < 0 || !self.charset_chars96(id1 as usize) || iso.flags & SEVEN_BITS != 0) { invalid!(); }
                    if id1 < 0 { invalid!(); }
                    id1
                }
                0x0E => {
                    if iso.flags & LOCKING_SHIFT == 0 || iso.designation[1] < 0 { invalid!(); }
                    iso.invocation[0] = 1;
                    continue;
                }
                0x0F => {
                    if iso.flags & LOCKING_SHIFT == 0 { invalid!(); }
                    iso.invocation[0] = 0;
                    continue;
                }
                0x00..=0x1F if !matches!(c1, 0x19 | 0x1B) => ascii,
                0x80..=0x9F if !matches!(c1, 0x8E | 0x8F | 0x9B) => invalid!(),
                _ => {
                    // The escape sequences, with SS2, SS3 and CSI as ESC N, O, [.
                    c1 = match c1 {
                        0x19 if iso.flags & SEVEN_BITS == 0 || iso.flags & SINGLE_SHIFT == 0 => invalid!(),
                        0x8E if iso.flags & SINGLE_SHIFT == 0 => invalid!(),
                        0x8F if iso.flags & SINGLE_SHIFT == 0 => invalid!(),
                        0x19 | 0x8E => i32::from(b'N'),
                        0x8F => i32::from(b'O'),
                        0x9B => i32::from(b'['),
                        _ => more!(),
                    };
                    loop {
                        match u8::try_from(c1).unwrap_or(0) {
                            b'&' => {
                                c1 = more!();
                                if !(0x40..=0x7E).contains(&c1) { invalid!(); }
                                if more!() != ESC { invalid!(); }
                                c1 = more!();
                                continue;
                            }
                            b'$' => {
                                if iso.flags & DESIGNATION == 0 { invalid!(); }
                                c1 = more!();
                                let (reg, chars96) = match c1 {
                                    0x40..=0x42 => (0, false),
                                    0x28..=0x2B => { let r = (c1 - 0x28) as usize; c1 = more!(); (r, false) }
                                    0x2C..=0x2F => { let r = (c1 - 0x2C) as usize; c1 = more!(); (r, true) }
                                    _ => invalid!(),
                                };
                                if !designate(&mut iso, reg, 2, chars96, c1) { invalid!(); }
                                continue 'main;
                            }
                            b'n' | b'o' => {
                                let reg = if c1 == i32::from(b'n') { 2 } else { 3 };
                                if iso.flags & LOCKING_SHIFT == 0 || iso.designation[reg] < 0 { invalid!(); }
                                iso.invocation[0] = reg as i32;
                                continue 'main;
                            }
                            b'N' | b'O' => {
                                let reg = if c1 == i32::from(b'N') { 2 } else { 3 };
                                if iso.flags & SINGLE_SHIFT == 0 || iso.designation[reg] < 0 { invalid!(); }
                                let id = iso.designation[reg];
                                c1 = more!();
                                if c1 < 0x20 || (0x80..0xA0).contains(&c1)
                                    || (iso.flags & SEVEN_BITS == 0 && if iso.flags & LEVEL_4 != 0 { c1 >= 0x80 } else { c1 < 0x80 }) {
                                    invalid!();
                                }
                                break if id < 0 { ascii } else { id };
                            }
                            b'0' | b'2' | b'3' | b'4' => {
                                if iso.flags & COMPOSITION == 0 { invalid!(); }
                                return Err(composition_refusal("ISO-2022"));
                            }
                            b'1' => invalid!(),
                            b'[' => {
                                if iso.flags & DIRECTION == 0 { invalid!(); }
                                // Nested direction is not supported; the
                                // direction itself changes no character.
                                match u8::try_from(more!()).unwrap_or(0) {
                                    b']' => {}
                                    b'0' | b'1' | b'2' => if more!() != i32::from(b']') { invalid!(); },
                                    _ => invalid!(),
                                }
                                continue 'main;
                            }
                            b'%' => {
                                match u8::try_from(more!()).unwrap_or(0) {
                                    b'/' => {
                                        // A CTEXT extended segment, kept as is.
                                        let dim = more!();
                                        if !(0x30..=0x34).contains(&dim) { invalid!(); }
                                        let m = more!();
                                        if m < 128 { invalid!(); }
                                        let l = more!();
                                        if l < 128 { invalid!(); }
                                        out.extend([b'\x1b', b'%', b'/'].map(CodingToken::Byte));
                                        out.extend([dim, m, l].map(|b| CodingToken::Byte(b as u8)));
                                        extsegment = ((m - 128) * 128 + (l - 128)) as usize;
                                    }
                                    b'G' => {
                                        // XFree86's embedded UTF-8, kept as is.
                                        out.extend([b'\x1b', b'%', b'G'].map(CodingToken::Byte));
                                        embedded_utf8 = true;
                                    }
                                    _ => invalid!(),
                                }
                                continue 'main;
                            }
                            _ => {
                                if iso.flags & DESIGNATION == 0 { invalid!(); }
                                let (reg, chars96) = match c1 {
                                    0x28..=0x2B => ((c1 - 0x28) as usize, false),
                                    0x2C..=0x2F => ((c1 - 0x2C) as usize, true),
                                    _ => invalid!(),
                                };
                                c1 = more!();
                                if !designate(&mut iso, reg, 1, chars96, c1) { invalid!(); }
                                continue 'main;
                            }
                        }
                    }
                }
            };
            if charset != ascii && last_id != charset {
                if last_id != ascii { runs.push((last_offset, out.len(), last_id as usize)); }
                (last_id, last_offset) = (charset, out.len());
            }
            let id = charset as usize;
            let dimension = self.charset_record_of(id).dimension;
            let mut code = c1;
            if dimension > 1 {
                let c2 = more!();
                if c2 < 0x20 || (0x80..0xA0).contains(&c2) || (c1 & 0x80) != (c2 & 0x80) { invalid!(); }
                if dimension == 2 {
                    code = (c1 << 8) | c2;
                } else {
                    let c3 = more!();
                    if c3 < 0x20 || (0x80..0xA0).contains(&c3) || (c1 & 0x80) != (c3 & 0x80) { invalid!(); }
                    // GNU repeats C2 where C3 belongs.
                    code = (c1 << 16) | (c2 << 8) | c2;
                }
            }
            let i = self.vm.charsets.ids[id];
            match self.charset_decode(i, (code & 0x7F7F7F) as u32)? {
                Some(c) => out.push(char_token(c)),
                None => out.extend(tokens[base..src.at].iter().copied()),
            }
        }
        if last_id != ascii { runs.push((last_offset, out.len(), last_id as usize)); }
        // decode_coding: an unfinished sequence's bytes as raw bytes.
        out.extend(tokens[src.at..].iter().copied());
        Ok((out, runs))
    }

    /// coding.c encode_coding_iso_2022 over TOKENS whose line ends
    /// consume_chars already converted (MAC: CR ends a line).
    pub(super) fn coding_encode_iso_2022(&mut self, attrs: Value, tokens: &[CodingToken], mac: bool) -> std::result::Result<Vec<u8>, Fault> {
        let mut iso = self.iso_state(attrs)?;
        let flags = iso.flags;
        let ascii = self.named_charset_id("ascii")?;
        let list: Vec<usize> = if flags & FULL_SUPPORT != 0 {
            self.vm.charsets.iso_list.clone()
        } else {
            self.coding_items(self.vector_get(attrs, num(4))?)?.iter().filter_map(|v| v.integer_value()).map(|n| n as usize).collect()
        };
        let ascii_compatible = self.vector_get(attrs, num(5))? != Value::NIL && flags & (DESIGNATION | LOCKING_SHIFT) == 0;
        let default = self.vector_get(attrs, num(11))?.integer_value().unwrap_or(32) as u32;
        let mut out = Vec::with_capacity(tokens.len());
        let mut bol_designation = flags & DESIGNATE_AT_BOL != 0;
        for (at, &token) in tokens.iter().enumerate() {
            if bol_designation {
                self.iso_designate_at_bol(&mut iso, &list, &tokens[at..], &mut out)?;
                bol_designation = false;
            }
            let c = match token {
                CodingToken::Byte(b) if b >= 128 => { out.push(b); continue }
                CodingToken::Byte(b) => u32::from(b),
                CodingToken::Character(c) => c,
            };
            if c < 0x20 || c == 0x7F {
                if c == u32::from(b'\n') || (c == u32::from(b'\r') && mac) {
                    if flags & RESET_AT_EOL != 0 { self.iso_reset(&mut iso, &mut out); }
                    if flags & INIT_AT_BOL != 0 { iso.designation = iso.initial; }
                    bol_designation = flags & DESIGNATE_AT_BOL != 0;
                } else if flags & RESET_AT_CNTL != 0 {
                    self.iso_reset(&mut iso, &mut out);
                }
                out.push(c as u8);
            } else if c < 128 {
                if ascii_compatible { out.push(c as u8); } else { self.iso_encode_char(&mut iso, ascii, c, &mut out)?; }
            } else {
                let (id, c) = match self.charset_in_list(&list, c)? {
                    Some(id) => (id, c),
                    None if flags & SAFE != 0 => (ascii, u32::from(b'?')),
                    None => (self.charset_in_list(&list, default)?.unwrap_or(ascii), default),
                };
                self.iso_encode_char(&mut iso, id, c, &mut out)?;
            }
        }
        if flags & RESET_AT_EOL != 0 { self.iso_reset(&mut iso, &mut out); }
        Ok(out)
    }

    /// charset.c char_charset over a non-nil charset LIST: the first that
    /// encodes C.
    fn charset_in_list(&mut self, list: &[usize], c: u32) -> std::result::Result<Option<usize>, Fault> {
        for &id in list {
            if self.charset_encode(self.vm.charsets.ids[id], c)?.is_some() { return Ok(Some(id)); }
        }
        Ok(None)
    }

    /// ENCODE_ISO_CHARACTER: C of charset ID, designated and invoked first
    /// when it must be.
    fn iso_encode_char(&mut self, iso: &mut IsoState, id: usize, c: u32, out: &mut Vec<u8>) -> std::result::Result<(), Fault> {
        let code = self.charset_encode(self.vm.charsets.ids[id], c)?.unwrap_or(0);
        let mut id = id;
        let ascii = self.named_charset_id("ascii")?;
        let dimension = self.charset_record_of(id).dimension;
        if dimension == 1 && iso.flags & USE_ROMAN != 0 && id == ascii {
            id = self.vm.charsets.jis_roman.unwrap_or(id);
        } else if dimension != 1 && iso.flags & USE_OLDJIS != 0 && Some(id) == self.vm.charsets.jis_0208 {
            id = self.vm.charsets.jis_1978.unwrap_or(id);
        }
        let bytes: Vec<u8> = if dimension == 1 { vec![code as u8] } else { vec![(code >> 8) as u8, code as u8] };
        // Designation and invocation take at most two rounds.
        for _ in 0..3 {
            if iso.single_shifting {
                let seven = iso.flags & SEVEN_BITS != 0;
                out.extend(bytes.iter().map(|&b| if seven { b & 0x7F } else { b | 0x80 }));
                iso.single_shifting = false;
                return Ok(());
            } else if id as i32 == iso.invoked(0) {
                out.extend(bytes.iter().map(|&b| b & 0x7F));
                return Ok(());
            } else if id as i32 == iso.invoked(1) {
                out.extend(bytes.iter().map(|&b| b | 0x80));
                return Ok(());
            }
            self.iso_invoke_designate(iso, id, out);
        }
        Err(Fault::Error("VM2 ISO-2022 encoding could not invoke the character's charset".into()))
    }

    /// encode_invocation_designation.
    fn iso_invoke_designate(&mut self, iso: &mut IsoState, id: usize, out: &mut Vec<u8>) {
        let reg = match iso.designation.iter().position(|&d| d == id as i32) {
            Some(reg) => reg,
            None => {
                let reg = iso.request(id).unwrap_or(0);
                self.iso_designate(iso, id, reg, out);
                reg
            }
        };
        if iso.invocation[0] != reg as i32 && iso.invocation[1] != reg as i32 {
            let seven = iso.flags & SEVEN_BITS != 0;
            match reg {
                0 => { out.push(0x0F); iso.invocation[0] = 0; }
                1 => { out.push(0x0E); iso.invocation[0] = 1; }
                2 | 3 if iso.flags & SINGLE_SHIFT != 0 => {
                    let (escape, byte) = if reg == 2 { (b'N', 0x8E) } else { (b'O', 0x8F) };
                    if seven { out.extend([0x1B, escape]); } else { out.push(byte); }
                    iso.single_shifting = true;
                }
                // GNU's locking-shift-3 also writes ESC n.
                _ => { out.extend([0x1B, b'n']); iso.invocation[0] = reg as i32; }
            }
        }
    }

    /// ENCODE_DESIGNATION of charset ID to register REG.
    fn iso_designate(&mut self, iso: &mut IsoState, id: usize, reg: usize, out: &mut Vec<u8>) {
        let chars96 = self.charset_chars96(id);
        let record = self.charset_record_of(id);
        let (final_char, revision, dimension) = (record.iso_final as u8, record.iso_revision, record.dimension);
        if iso.flags & REVISION != 0 && revision >= 0 {
            out.extend([0x1B, b'&', b'@' + revision as u8]);
        }
        out.push(0x1B);
        let (i94, i96) = (b"()*+"[reg], b",-./"[reg]);
        if dimension == 1 {
            out.push(if chars96 { i96 } else { i94 });
        } else {
            out.push(b'$');
            if chars96 {
                out.push(i96);
            } else if iso.flags & LONG_FORM != 0 || reg != 0 || !(b'@'..=b'B').contains(&final_char) {
                out.push(i94);
            }
        }
        out.push(final_char);
        iso.designation[reg] = id as i32;
    }

    /// ENCODE_RESET_PLANE_AND_REGISTER.
    fn iso_reset(&mut self, iso: &mut IsoState, out: &mut Vec<u8>) {
        if iso.invocation[0] != 0 { out.push(0x0F); iso.invocation[0] = 0; }
        for reg in 0..4 {
            let initial = iso.initial[reg];
            if initial >= 0 && iso.designation[reg] != initial {
                self.iso_designate(iso, initial as usize, reg, out);
            }
        }
    }

    /// encode_designation_at_bol: the designations the line starting at
    /// TOKENS requests, before its first character.
    fn iso_designate_at_bol(&mut self, iso: &mut IsoState, list: &[usize], tokens: &[CodingToken], out: &mut Vec<u8>) -> std::result::Result<(), Fault> {
        let mut r = [None; 4];
        let mut found = 0;
        for &token in tokens {
            if found >= 4 { break }
            let c = match token { CodingToken::Byte(b) if b >= 128 => crate::lstring::BYTE8_OFFSET + u32::from(b), CodingToken::Byte(b) => u32::from(b), CodingToken::Character(c) => c };
            if c == u32::from(b'\n') { break }
            let Some(id) = self.charset_in_list(list, c)? else { continue };
            if let Some(reg) = iso.request(id) && r[reg].is_none() {
                r[reg] = Some(id);
                found += 1;
            }
        }
        for (reg, id) in r.into_iter().enumerate() {
            if let Some(id) = id && iso.designation[reg] != id as i32 { self.iso_designate(iso, id, reg, out); }
        }
        Ok(())
    }

    /// coding.c decode_coding_emacs_mule over EOL-decoded TOKENS.
    pub(super) fn coding_decode_emacs_mule(&mut self, tokens: &[CodingToken]) -> std::result::Result<Decoded, Fault> {
        let ascii = self.named_charset_id("ascii")?;
        let eight_bit = self.named_charset_id("eight-bit")?;
        let bytes = self.emacs_mule_bytes();
        let mut charsets = [None; 256];
        for &(mule_id, id, _) in &self.vm.charsets.mule {
            if (0..256).contains(&mule_id) { charsets[mule_id as usize] = Some(id); }
        }
        let mut src = Source { tokens, at: 0 };
        let mut out = Vec::with_capacity(tokens.len());
        let (mut runs, mut last_id, mut last_offset) = (Vec::new(), ascii, 0);
        'main: loop {
            let base = src.at;
            let Some(c) = src.next() else { break };
            if c < 0 {
                out.push(CodingToken::Character((-c) as u32));
                continue;
            }
            if c == 0x80 {
                // DECODE_EMACS_MULE_COMPOSITION_START: a valid header starts
                // a composition; otherwise 0x80 is invalid code.
                match src.next() {
                    None => { src.at = base; break }
                    Some(h) if (0xF2..=0xF5).contains(&h) || (0xA0..0xC0).contains(&h) || h == 0xFF => {
                        return Err(composition_refusal("emacs-mule"));
                    }
                    Some(_) => { src.at = base + 1; out.push(CodingToken::Byte(0x80)); continue }
                }
            }
            // emacs_mule_char: -1 invalid code, None the source ran out.
            let decoded: Option<std::result::Result<(usize, u32), ()>> = if c < 0x80 {
                Some(Ok((ascii, c as u32)))
            } else {
                let mut char_of = || -> Option<std::result::Result<(usize, u32), ()>> {
                    if c >= 0xA0 { return Some(Err(())) }
                    let (id, code) = match bytes[c as usize] {
                        2 => {
                            let Some(id) = charsets[c as usize] else { return Some(Err(())) };
                            let b = src.next()?;
                            if b < 0xA0 { return Some(Err(())) }
                            (id, (b & 0x7F) as u32)
                        }
                        3 if c == 0x9A || c == 0x9B => {
                            let b = src.next()?;
                            let Some(id) = (b >= 0xA0).then(|| charsets[b as usize]).flatten() else { return Some(Err(())) };
                            let b = src.next()?;
                            if b < 0xA0 { return Some(Err(())) }
                            (id, (b & 0x7F) as u32)
                        }
                        3 => {
                            let Some(id) = charsets[c as usize] else { return Some(Err(())) };
                            let b1 = src.next()?;
                            if b1 < 0xA0 { return Some(Err(())) }
                            let b2 = src.next()?;
                            if b2 < 0xA0 { return Some(Err(())) }
                            (id, (((b1 & 0x7F) << 8) | (b2 & 0x7F)) as u32)
                        }
                        4 => {
                            let b = src.next()?;
                            let Some(id) = (b >= 0).then(|| charsets[b as usize]).flatten() else { return Some(Err(())) };
                            let b1 = src.next()?;
                            if b1 < 0xA0 { return Some(Err(())) }
                            let b2 = src.next()?;
                            if b2 < 0xA0 { return Some(Err(())) }
                            (id, (((b1 & 0x7F) << 8) | (b2 & 0x7F)) as u32)
                        }
                        _ => (if c < 0x80 { ascii } else { eight_bit }, c as u32),
                    };
                    Some(Ok((id, code)))
                };
                char_of()
            };
            let (id, ch) = match decoded {
                None => { src.at = base; break 'main }
                Some(Ok((id, code))) => match self.charset_decode(self.vm.charsets.ids[id], code)? {
                    Some(ch) => (id, ch),
                    None => { src.at = base + 1; out.push(CodingToken::Byte(c as u8)); continue }
                },
                Some(Err(())) => { src.at = base + 1; out.push(CodingToken::Byte(c as u8)); continue }
            };
            if last_id != id {
                if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
                (last_id, last_offset) = (id, out.len());
            }
            out.push(char_token(ch));
        }
        if last_id != ascii { runs.push((last_offset, out.len(), last_id)); }
        out.extend(tokens[src.at..].iter().copied());
        Ok((out, runs))
    }

    /// coding.c encode_coding_emacs_mule over line-converted TOKENS.
    pub(super) fn coding_encode_emacs_mule(&mut self, attrs: Value, tokens: &[CodingToken]) -> std::result::Result<Vec<u8>, Fault> {
        let list = self.vm.charsets.mule_list.clone();
        let default = self.vector_get(attrs, num(11))?.integer_value().unwrap_or(32) as u32;
        let mut out = Vec::with_capacity(tokens.len());
        for &token in tokens {
            let c = match token {
                CodingToken::Byte(b) => { out.push(b); continue }
                CodingToken::Character(c) => c,
            };
            let (id, code) = match self.charset_in_list(&list, c)? {
                Some(id) => (id, c),
                None if default < 128 => { out.push(default as u8); continue }
                None => match self.charset_in_list(&list, default)? {
                    Some(id) => (id, default),
                    None => { out.push(b'?'); continue }
                },
            };
            let code = self.charset_encode(self.vm.charsets.ids[id], code)?.unwrap_or(0);
            let record = self.charset_record_of(id);
            let (mule_id, dimension) = (record.mule_id, record.dimension);
            // EMACS_MULE_LEADING_CODES.
            match mule_id {
                ..0xA0 => out.push(mule_id as u8),
                ..0xE0 => out.extend([0x9A, mule_id as u8]),
                ..0xF0 => out.extend([0x9B, mule_id as u8]),
                ..0xF5 => out.extend([0x9C, mule_id as u8]),
                _ => out.extend([0x9D, mule_id as u8]),
            }
            if dimension == 1 { out.push((code | 0x80) as u8); }
            else { let code = code | 0x8080; out.extend([(code >> 8) as u8, code as u8]); }
        }
        Ok(out)
    }

    /// coding.c encode_coding_sjis (SJIS) or encode_coding_big5 over
    /// line-converted TOKENS; the charset list is (ROMAN KANA KANJI
    /// [KANJI2]) or (ROMAN BIG5).
    pub(super) fn coding_encode_double(&mut self, attrs: Value, tokens: &[CodingToken], sjis: bool) -> std::result::Result<Vec<u8>, Fault> {
        let list: Vec<usize> = self.coding_items(self.vector_get(attrs, num(4))?)?.iter().filter_map(|v| v.integer_value()).map(|n| n as usize).collect();
        let ascii_compatible = self.vector_get(attrs, num(5))? != Value::NIL;
        let default = self.vector_get(attrs, num(11))?.integer_value().unwrap_or(32) as u32;
        let mut out = Vec::with_capacity(tokens.len());
        for &token in tokens {
            let c = match token {
                CodingToken::Byte(b) if b < 128 && ascii_compatible => { out.push(b); continue }
                CodingToken::Byte(b) if b >= 128 => { out.push(b); continue }
                CodingToken::Byte(b) => u32::from(b),
                CodingToken::Character(c) => c,
            };
            let (id, c) = match self.charset_in_list(&list, c)? {
                Some(id) => (id, c),
                None => match self.charset_in_list(&list, default)? {
                    Some(id) => (id, default),
                    None => { out.push(b'?'); continue }
                },
            };
            let code = self.charset_encode(self.vm.charsets.ids[id], c)?.unwrap_or(0);
            if sjis && Some(&id) == list.get(2) {
                out.extend(jis_to_sjis(code, false));
            } else if sjis && Some(&id) == list.get(1) {
                out.push((code | 0x80) as u8);
            } else if sjis && Some(&id) == list.get(3) {
                let c1 = code >> 8;
                if c1 == 0x21 || (0x23..=0x25).contains(&c1) || c1 == 0x28 || (0x2C..=0x2F).contains(&c1) || c1 >= 0x6E {
                    out.extend(jis_to_sjis(code, true));
                } else {
                    out.push((code & 0x7F) as u8);
                }
            } else if !sjis && Some(&id) == list.get(1) {
                out.extend([(code >> 8) as u8, code as u8]);
            } else {
                out.push((code & 0x7F) as u8);
            }
        }
        Ok(out)
    }
}

/// coding.h JIS_TO_SJIS, or JIS_TO_SJIS2 for JIS X 0213 plane 2.
fn jis_to_sjis(code: u32, plane2: bool) -> [u8; 2] {
    let (j1, j2) = (code >> 8, code & 0xFF);
    let (s1, s2) = if j1 & 1 != 0 {
        let s1 = if !plane2 { j1 / 2 + if j1 < 0x5F { 0x71 } else { 0xB1 } }
            else if j1 <= 0x25 { 0xF0 + (j1 - 0x21) / 2 }
            else if j1 <= 0x2F { 0xF3 + (j1 - 0x2D) / 2 }
            else { 0xF5 + (j1 - 0x6F) / 2 };
        (s1, j2 + if j2 >= 0x60 { 0x20 } else { 0x1F })
    } else {
        let s1 = if !plane2 { j1 / 2 + if j1 < 0x5F { 0x70 } else { 0xB0 } }
            else { match j1 { 0x28 => 0xF0, 0x24 => 0xF1, 0x2C => 0xF2, 0x2E => 0xF3, _ => 0xF4 + (j1.wrapping_sub(0x6E)) / 2 } };
        (s1, j2 + 0x7E)
    };
    [s1 as u8, s2 as u8]
}
