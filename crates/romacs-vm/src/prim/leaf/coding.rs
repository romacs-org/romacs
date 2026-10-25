//! GNU coding.c registry and native UTF-8/raw-text string conversion.
use super::*;

const ROOT: &str = "#:vm2-coding-system-registry";
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
        let fast = source.bytes().is_ascii() && self.vector_get(attrs, num(5))? != Value::NIL
            && (eol == unix || inhibit || !source.bytes().contains(&if encode { b'\n' } else { b'\r' }));
        if fast && (buffer == Value::NIL || buffer == Value::TRUE) {
            self.variable_set(last, name)?;
            return if nocopy != Value::NIL { Ok(string) } else { Ok(self.make_string(source.bytes().to_vec(), !encode)) };
        }
        let kind = self.vector_get(attrs, num(3))?;
        let utf8 = kind == self.intern("utf-8");
        if !utf8 && kind != self.intern("raw-text") {
            return Err(Fault::Error("string conversion for this coding type is not implemented (supported: UTF-8 and raw-text)".into()));
        }
        for index in [6, 7, 9, 10] {
            if self.vector_get(attrs, num(index))? != Value::NIL {
                return Err(Fault::Error("string coding translation tables and conversion hooks are not implemented".into()));
            }
        }
        let bom = if utf8 { self.vector_get(attrs, num(24))? } else { Value::NIL };
        if self.pair(bom).is_some() { return Err(Fault::Error("string coding BOM-pair detection is not implemented".into())); }
        let tokens = if encode && utf8 && !source.is_multibyte() {
            coding_encode_unibyte(source.bytes())
        } else { source.chars().map(|code| {
            if !source.is_multibyte() { CodingToken::Byte(code as u8) }
            else if code >= crate::lstring::BYTE8_OFFSET + 128 { CodingToken::Byte((code - crate::lstring::BYTE8_OFFSET) as u8) }
            else if code < 128 { CodingToken::Byte(code as u8) }
            else { CodingToken::Character(code) }
        }).collect::<Vec<_>>() };
        let detected = if !encode && !inhibit && self.kind(eol) == Kind::Vector { coding_eol(&tokens) } else { None };
        let mode = if inhibit { 0 } else if let Some(mode) = detected { mode }
            else if eol == self.intern("dos") { 1 } else if eol == self.intern("mac") { 2 } else { 0 };
        let precise = if let Some(mode) = detected { self.vector_get(eol, num(mode as i64))? } else { name };
        let (bytes, multibyte) = if encode {
            let mut bytes = Vec::new();
            if utf8 && bom != Value::NIL { bytes.extend_from_slice(&[239, 187, 191]); }
            for token in tokens { match token {
                CodingToken::Byte(byte) => bytes.push(byte),
                CodingToken::Character(code) => coding_emit(code, &mut bytes),
            }}
            (coding_encode_eol(bytes, mode), false)
        } else {
            let tokens = if utf8 {
                coding_decode_utf8(&tokens, bom != Value::NIL, !inhibit && eol == self.intern("dos"))
                    .into_iter().map(|code| if code < 128 { CodingToken::Byte(code as u8) } else { CodingToken::Character(code) }).collect()
            } else { tokens };
            let tokens = coding_decode_eol(tokens, mode);
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
        if buffer != Value::NIL && buffer != Value::TRUE {
            return Err(Fault::Error("string coding output to buffers is not implemented".into()));
        }
        self.variable_set(last, precise)?;
        Ok(self.make_string(bytes, multibyte))
    }
}

#[derive(Clone, Copy)]
enum CodingToken { Byte(u8), Character(u32) }

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

fn coding_encode_eol(bytes: Vec<u8>, mode: usize) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    for byte in bytes {
        if byte == b'\n' && mode != 0 { result.push(b'\r'); }
        if byte != b'\n' || mode != 2 { result.push(byte); }
    }
    result
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
    fn coding_items(&self, mut list: Value) -> std::result::Result<Vec<Value>, Fault> {
        let original = list;
        let mut seen = HashSet::new();
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
        let mut seen = HashSet::new();
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
        let mut seen = HashSet::new();
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
        let mut seen = HashSet::new();
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
        let mut seen = HashSet::new();
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
                let mut seen = HashSet::new();
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
        Ok(Value::NIL)
    }
}
