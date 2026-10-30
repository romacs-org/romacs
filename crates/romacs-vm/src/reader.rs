//! One integer-character grammar for host sources and live Lisp streams.
use crate::coordinates::ByteOffset;
use crate::{Form, Result, Value, Vm, data::Object};
use std::collections::BTreeMap;

trait CharacterCode {
    fn is_ascii_whitespace(self) -> bool;
    fn is_ascii_alphanumeric(self) -> bool;
    fn is_ascii_digit(self) -> bool;
    fn to_digit(self, radix: u32) -> Option<u32>;
}
impl CharacterCode for u32 {
    fn is_ascii_whitespace(self) -> bool {
        matches!(self, 9..=13 | 32)
    }
    fn is_ascii_alphanumeric(self) -> bool {
        matches!(self,48..=57|65..=90|97..=122)
    }
    fn is_ascii_digit(self) -> bool {
        matches!(self, 48..=57)
    }
    fn to_digit(self, radix: u32) -> Option<u32> {
        char::from_u32(self).and_then(|c| c.to_digit(radix))
    }
}

enum Input<'a> {
    /// GNU internal multibyte text: UTF-8 plus the 4- and 5-byte forms of
    /// characters above U+10FFFF and the 2-byte raw-byte forms.
    Host {
        text: &'a [u8],
        byte: usize,
    },
    String {
        root: usize,
        byte: usize,
        character: usize,
        end: usize,
    },
    Buffer {
        root: usize,
    },
    Marker {
        root: usize,
    },
    Function {
        root: usize,
    },
}
/// Bytes CODE occupies in GNU internal text.
fn host_width(code: u32) -> usize {
    crate::lstring::char_string(code, &mut [0; 5])
}

struct Reader<'a, 'v> {
    input: Input<'a>,
    at: usize,
    lookahead: Option<u32>,
    error: Option<String>,
    eof: bool,
    labels: BTreeMap<u64, usize>,
    vm: &'v mut Vm,
    signal: Option<(Value, Value)>,
    #[cfg(test)]
    collect_subforms: bool,
}
impl Reader<'_, '_> {
    fn get(&mut self) -> Result<Option<u32>> {
        match &mut self.input {
            Input::Host { text, byte } => {
                if *byte >= text.len() {
                    return Ok(None);
                }
                let (c, width) = crate::lstring::decode_char(&text[*byte..]).ok_or("invalid source character")?;
                *byte += width;
                Ok(Some(c))
            }
            Input::String {
                root,
                byte,
                character,
                end,
            } => {
                if *character >= *end {
                    return Ok(None);
                }
                let view = self
                    .vm
                    .str_view(self.vm.external_roots[*root])
                    .ok_or("string stream no longer exists")?;
                let bytes = view
                    .bytes()
                    .get(*byte..)
                    .filter(|b| !b.is_empty())
                    .ok_or("string stream changed length")?;
                let (c, width) = if view.is_multibyte() {
                    let width = match bytes[0] {
                        0..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf7 => 4,
                        0xf8 => 5,
                        _ => return Err("invalid string character".into()),
                    };
                    let bytes = bytes.get(..width).ok_or("truncated string character")?;
                    crate::lstring::string_char(bytes)
                } else {
                    let c = bytes[0] as u32;
                    (if c >= 128 { c + 0x3fff00 } else { c }, 1)
                };
                *character += 1;
                *byte += width;
                Ok(Some(c))
            }
            Input::Function { root } => {
                let function = self.vm.external_roots[*root];
                let result = self.vm.reader_call(function, &[])?;
                Ok(result
                    .integer_value()
                    .filter(|n| (0..=0x3fffff).contains(n))
                    .map(|n| n as u32))
            }
            Input::Buffer { root } | Input::Marker { root } => {
                let source = self.vm.external_roots[*root];
                self.vm.reader_buffer_get(source)
            }
        }
    }
    fn unget(&mut self, c: u32) -> Result<()> {
        match &mut self.input {
            Input::Host { byte, .. } => *byte -= host_width(c),
            Input::String {
                root,
                character,
                byte,
                ..
            } => {
                let view = self
                    .vm
                    .str_view(self.vm.external_roots[*root])
                    .ok_or("string stream no longer exists")?;
                let width = if view.is_multibyte() {
                    let mut bytes = [0; 5];
                    crate::lstring::char_string(c, &mut bytes)
                } else {
                    1
                };
                *character -= 1;
                *byte -= width;
            }
            Input::Function { root } => {
                let function = self.vm.external_roots[*root];
                self.vm
                    .reader_call(function, &[Value::integer(c as i64)?])?;
            }
            Input::Buffer { root } | Input::Marker { root } => {
                let source = self.vm.external_roots[*root];
                self.vm.reader_buffer_unget(source, c)?;
            }
        }
        Ok(())
    }
    fn peek(&mut self) -> Option<u32> {
        if self.lookahead.is_none() && !self.eof && self.error.is_none() {
            match self.get() {
                Ok(c) => {
                    self.eof = c.is_none();
                    self.lookahead = c;
                }
                Err(e) => {
                    self.signal = self.vm.pending_eval_escape();
                    self.error = Some(e);
                }
            }
        }
        self.lookahead
    }
    fn next(&mut self) -> Result<u32> {
        let c = match self.peek() {
            Some(c) => c,
            None => {
                self.eof = false;
                return Err(self
                    .error
                    .clone()
                    .unwrap_or_else(|| "end-of-file reading Lisp".into()));
            }
        };
        self.lookahead = None;
        self.at += match self.input {
            Input::Host { .. } => host_width(c),
            _ => 1,
        };
        Ok(c)
    }
    fn unread_lookahead(&mut self) -> Result<()> {
        if let Some(c) = self.lookahead.take() {
            self.unget(c)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        self.unread_lookahead()?;
        if let Some(e) = self.error.take() {
            Err(e)
        } else {
            Ok(())
        }
    }
    fn space(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    if self.next().is_err() {
                        break;
                    }
                }
                Some(59) => {
                    while let Some(c) = self.peek() {
                        if self.next().is_err() || c == 10 {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }
    /// lread.c `#:' / `#_': what makes the following name empty.
    fn empty_symbol_follows(c: u32) -> bool {
        c <= 32 || c == 0xa0 || b"\"';#()[]`,".iter().any(|&b| u32::from(b) == c)
    }
    /// A symbol token from FIRST up to a delimiter, backslash escapes
    /// taken literally; true when any character was escaped.
    fn symbol_token(&mut self, first: u32) -> Result<(String, bool)> {
        // Raw-byte and non-Unicode characters keep their codes (symkey.rs).
        let mut token = String::new();
        let mut escaped = first == 92;
        crate::symkey::push_char(&mut token, if escaped { self.next()? } else { first });
        while self.peek().is_some_and(|c| !Self::delimiter(c)) {
            let c = self.next()?;
            if c == 92 {
                escaped = true;
                crate::symkey::push_char(&mut token, self.next()?);
            } else {
                crate::symkey::push_char(&mut token, c);
            }
        }
        self.unread_lookahead()?;
        Ok((token, escaped))
    }
    fn delimiter(c: u32) -> bool {
        c.is_ascii_whitespace() || [40, 41, 91, 93, 34, 59, 39, 96, 44].contains(&c)
    }
    /// lread.c read_char_literal: what may follow `?C'.
    fn char_literal_follows(c: u32) -> bool {
        c <= 32 || b"\"';()[]#?`,.".iter().any(|&b| u32::from(b) == c)
    }
    fn dot_follows(next: Option<u32>) -> bool {
        next.is_none_or(|c| {
            c <= 32 || c == 160 || [34, 39, 59, 40, 91, 35, 63, 96, 44].contains(&c)
        })
    }
    fn dot_token(&mut self) -> bool {
        if self.peek() != Some(46) {
            return false;
        }
        let next = self.get();
        match next {
            Ok(c) => {
                if let Some(c) = c {
                    if let Err(e) = self.unget(c) {
                        self.error = Some(e);
                    }
                }
                Self::dot_follows(c)
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }
    /// Record a terminal GNU condition; no subsequent parse or callback runs.
    /// Partial values and the live source use external-root indices throughout
    /// parsing, so callback reentry cannot invalidate an aggregate in progress.
    fn signal(&mut self, condition: &str, data: &[Value], text: String) -> String {
        let condition = self.vm.intern(condition);
        let data = self.vm.list_value(data);
        self.signal = Some((condition, data));
        text
    }
    fn syntax(&mut self, datum: &str) -> String {
        let text = self.vm.make_lisp_string(datum.as_bytes().to_vec(), true);
        self.signal(
            "invalid-read-syntax",
            &[text],
            format!("invalid-read-syntax {datum}"),
        )
    }
    /// GNU lread.c read_bool_vector: `#&LENGTH"BYTES"`, a unibyte string of
    /// exactly ceil(LENGTH/8) bytes (or one more, as Emacs 19 printed it).
    fn bool_vector(&mut self, depth: usize) -> Result<Value> {
        let mut length: usize = 0;
        loop {
            match self.peek() {
                Some(c @ 48..=57) => {
                    self.next()?;
                    length = length.checked_mul(10).and_then(|n| n.checked_add((c - 48) as usize))
                        .filter(|&n| n <= isize::MAX as usize / 8).ok_or_else(|| self.syntax("#&"))?;
                }
                Some(34) => break,
                _ => { self.next()?; return Err(self.syntax("#&")); }
            }
        }
        let string = self.form_inner(depth + 1)?;
        let view = self.vm.str_view(string).ok_or("bool-vector literal string")?;
        // Host-source literals keep ASCII text multibyte; its bytes are the same.
        let (multibyte, bytes) = (view.is_multibyte() && !view.bytes().is_ascii(), view.bytes().to_vec());
        let size = length.div_ceil(8);
        if multibyte || !(bytes.len() == size || length == bytes.len().saturating_sub(1) * 8) {
            return Err(self.syntax("#&..."));
        }
        let mut vector = crate::bool_vector::BoolVector::new(length, false)?;
        vector.bytes.copy_from_slice(&bytes[..size]);
        vector.mask_padding();
        Ok(self.vm.allocate_object(Object::BoolVector(Box::new(vector))))
    }
    /// GNU lread.c:3852 `#s(...)`: a record, or, when the first element is
    /// the symbol `hash-table`, a table built by `hash_table_from_plist`
    /// (lread.c:3153, dispatched at :3770-3780). `#s' must be followed
    /// directly by `(`; `#s()` and a dotted `#s(... . x)` are invalid syntax.
    fn aggregate(&mut self, end: u32, depth: usize) -> Result<Vec<usize>> {
        let mut roots = Vec::new();
        loop {
            self.space();
            if self.peek() == Some(end) {
                self.next()?;
                return Ok(roots);
            }
            self.form(depth + 1)?;
            roots.push(self.vm.external_roots.len() - 1);
        }
    }
    fn table(&mut self, depth: usize) -> Result<Value> {
        let sub = self.peek() == Some(94);
        if sub {
            self.next()?;
        }
        let prefix = if sub { "#^^" } else { "#^" };
        match self.peek() {
            Some(91) => { self.next()?; }
            other => {
                self.unread_lookahead()?;
                let suffix = other.and_then(char::from_u32).map(String::from).unwrap_or_default();
                return Err(self.syntax(&format!("{prefix}{suffix}")));
            }
        }
        let roots = self.aggregate(93, depth)?;
        let values = roots
            .iter()
            .map(|&i| self.vm.external_roots[i])
            .collect::<Vec<_>>();
        let result = if sub {
            self.vm.sub_char_table_from_literal(&values)
        } else {
            self.vm.char_table_from_literal(&values)
        };
        result.map_err(|e| {
            if !sub && e == "Invalid size char-table" {
                return self.syntax(&e);
            }
            let text = self.vm.byte_string(e.as_bytes());
            self.signal("error", &[text], e)
        })
    }
    fn property_string(&mut self, depth: usize) -> Result<Value> {
        let roots = self.aggregate(41, depth)?;
        let Some(&root) = roots.first() else {
            return Err(self.syntax("#"));
        };
        let value = self.vm.external_roots[root];
        if self.vm.str_view(value).is_none() {
            return Err(self.syntax("#"));
        }
        if (roots.len() - 1) % 3 != 0 {
            return Err(self.syntax("Invalid string property list"));
        }
        for triple in roots[1..].chunks_exact(3) {
            let start = self.vm.external_roots[triple[0]];
            let end = self.vm.external_roots[triple[1]];
            let plist = self.vm.external_roots[triple[2]];
            let integerp = self.vm.intern("integer-or-marker-p");
            let a = start.integer_value().ok_or_else(|| {
                self.signal(
                    "wrong-type-argument",
                    &[integerp, start],
                    "invalid property start".into(),
                )
            })?;
            let b = end.integer_value().ok_or_else(|| {
                self.signal(
                    "wrong-type-argument",
                    &[integerp, end],
                    "invalid property end".into(),
                )
            })?;
            let view = self.vm.str_view(value).unwrap();
            if a < 0 || b < 0 || a as usize > view.len() || b as usize > view.len() {
                return Err(self.signal(
                    "args-out-of-range",
                    &[start, end],
                    "property range".into(),
                ));
            }
            let (a, b) = if a <= b {
                (a as usize, b as usize)
            } else {
                (b as usize, a as usize)
            };
            let width = |code| {
                let mut encoded = [0; 5];
                crate::lstring::char_string(code, &mut encoded)
            };
            let offset = |n| {
                if view.is_multibyte() {
                    view.chars().take(n).map(width).sum()
                } else {
                    n
                }
            };
            let range = ByteOffset(offset(a))..ByteOffset(offset(b));
            let mut pairs = Vec::new();
            let mut cursor = plist;
            while cursor != Value::NIL {
                let (key, tail) = self.vm.cons_pair(cursor)?;
                if tail == Value::NIL {
                    let text = self.vm.byte_string(b"Odd length text property list");
                    return Err(self.signal(
                        "error",
                        &[text],
                        "Odd length text property list".into(),
                    ));
                }
                let (val, tail) = self.vm.cons_pair(tail)?;
                let name = if key == Value::NIL {
                    "nil".to_owned()
                } else if key == Value::TRUE {
                    "t".to_owned()
                } else if key.tag() == 7 {
                    self.vm.symbol_names[key.index(7)? - 1].clone()
                } else {
                    return Err("reader properties require interned symbol keys".into());
                };
                if pairs.iter().any(|(previous, _)| previous == &name) {
                    let message = "Reader cannot retain duplicate text property keys";
                    let text = self.vm.byte_string(message.as_bytes());
                    return Err(self.signal("error", &[text], message.into()));
                }
                pairs.push((name, val));
                cursor = tail;
                if pairs.len() > self.vm.list_limit() {
                    return Err("circular property list".into());
                }
            }
            // The new literal may be an unibyte Bytes object: promote while preserving representation.
            let view = self.vm.str_view(value).unwrap();
            let mut string =
                crate::data::LispString::from_bytes(view.bytes().to_vec(), view.is_multibyte());
            if let Object::String(s) | Object::RawString(s) = self.vm.object(value)? {
                string.properties = s.properties.clone();
            }
            for key in string.properties.properties_keys() {
                string.properties.remove(range.clone(), &key);
            }
            // Reverse insertion preserves the plist's printed key order.
            for (key, val) in pairs.into_iter().rev() {
                string.properties.put(range.clone(), &key, val);
            }
            self.vm.objects[value.index(0)? - 1] = if string.is_utf8() {
                Object::String(Box::new(string))
            } else {
                Object::RawString(Box::new(string))
            };
        }
        Ok(value)
    }
    fn record(&mut self, depth: usize) -> Result<Value> {
        match self.peek() {
            Some(40) => {
                self.next()?;
            }
            // invalid_syntax_with_buffer: the datum is what was buffered,
            // `#s' plus the offending character when there is one.
            other => {
                let datum = format!(
                    "#s{}",
                    other
                        .and_then(char::from_u32)
                        .map(String::from)
                        .unwrap_or_default()
                );
                self.unread_lookahead()?;
                return Err(self.syntax(&datum));
            }
        }
        let mut items = vec![];
        loop {
            self.space();
            if self.peek() == Some(41) {
                self.next()?;
                break;
            }
            // A dot token here reaches `form_inner`'s dot arm and is refused.
            self.form(depth + 1)?;
            items.push(self.vm.external_roots.len() - 1);
        }
        // Every element is rooted in `external_roots` until the caller
        // truncates them; nothing below collects.
        let values: Vec<Value> = items.iter().map(|&i| self.vm.external_roots[i]).collect();
        let Some(&head) = values.first() else {
            return Err(self.syntax("#s"));
        };
        if head == self.vm.intern("hash-table") {
            return match self.vm.hash_table_from_plist(&values[1..]) {
                Ok(table) => Ok(table),
                Err(fault) => match self.vm.hash_fault_signal(fault) {
                    Ok((_, condition, data)) => {
                        self.signal = Some((condition, data));
                        Err("hash table read error".into())
                    }
                    Err(message) => Err(message),
                },
            };
        }
        self.vm.allocate_record(values).map_err(|message| {
            let text = self.vm.make_lisp_string(message.as_bytes().to_vec(), true);
            self.signal("error", &[text], message)
        })
    }
    /// GNU lread.c:2942 read_integer: scan all ASCII alphanumerics,
    /// poisoning the entire integer on an out-of-base digit. Punctuation is
    /// left unread, even when it is not an ordinary Lisp token delimiter.
    fn radix_integer(&mut self, radix: u32) -> Result<Value> {
        let mut token = String::new();
        if matches!(self.peek(), Some(43 | 45)) {
            token.push(char::from_u32(self.next()?).unwrap());
        }
        let mut digits = 0;
        let mut valid = true;
        while let Some(c) = self.peek().filter(|c| c.is_ascii_alphanumeric()) {
            self.next()?;
            digits += 1;
            valid &= c.to_digit(radix).is_some();
            token.push(char::from_u32(c).unwrap());
        }
        self.unread_lookahead()?;
        if digits == 0 || !valid {
            return Err(self.syntax(&format!("integer, radix {radix}")));
        }
        self.vm.integer_from_radix(&token, radix)
    }
    fn numbered_dispatch(&mut self, first: u32, depth: usize) -> Result<Value> {
        let mut number = (first - 48) as u64;
        let mut prefix = format!("#{}", char::from_u32(first).unwrap());
        while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
            self.next()?;
            prefix.push(char::from_u32(c).unwrap());
            number = number
                .checked_mul(10)
                .and_then(|n| n.checked_add((c - 48) as u64))
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(|| self.syntax(&prefix))?;
        }
        match self.next()? {
            114 | 82 if (2..=36).contains(&number) => self.radix_integer(number as u32),
            114 | 82 => Err(self.syntax(&format!("integer, radix {number}"))),
            61 => {
                let value = self.form(depth + 1)?;
                let root = self.vm.external_roots.len() - 1;
                self.labels.insert(number, root);
                Ok(value)
            }
            35 => self
                .labels
                .get(&number)
                .copied()
                .map(|r| self.vm.external_roots[r])
                .ok_or_else(|| self.syntax("undefined numbered reference")),
            _ => Err(self.syntax("#")),
        }
    }
    fn prefix(&mut self, name: &str, depth: usize) -> Result<Value> {
        let symbol = self.vm.intern(name);
        let root = self.vm.external_roots.len();
        self.vm.external_roots.push(symbol);
        let value = self.form(depth + 1)?;
        let symbol = self.vm.external_roots[root];
        Ok(self.vm.list_value(&[symbol, value]))
    }
    fn escape(&mut self) -> Result<i64> {
        // GNU lread.c:read_char_escape: modifiers accumulate, but Control
        // applies once per prefix after the base has been decoded.
        let mut modifiers = 0;
        let mut controls = 0;
        let mut c = self.next()?;
        let mut base;
        loop {
            let bit = match c {
                65 => 1 << 22,
                115 if self.peek() == Some(45) => 1 << 23,
                72 => 1 << 24,
                83 => 1 << 25,
                77 => 1 << 27,
                67 | 94 => 0,
                _ => {
                    base = self.escape_base(c)?;
                    break;
                }
            };
            if c != 94 && self.next()? != 45 {
                return Err(format!(
                    "Invalid escape char syntax: \\{c} not followed by -"
                ));
            }
            if matches!(c, 67 | 94) {
                controls += 1;
            } else {
                modifiers |= bit;
            }
            c = self.next()?;
            if c != 92 {
                base = c as i64;
                break;
            }
            c = self.next()?;
        }
        modifiers |= base & 0x0fc00000;
        base &= !0x0fc00000;
        for _ in 0..controls {
            if (64..=95).contains(&base) || (97..=122).contains(&base) {
                base &= 31;
            } else if base == 63 {
                base = 127;
            } else {
                modifiers |= 1 << 26;
            }
        }
        Ok(base | modifiers)
    }
    fn escape_base(&mut self, c: u32) -> Result<i64> {
        Ok(match c {
            10 => return Err("Invalid escape char syntax: backslash-newline".into()),
            97 => 7,
            98 => 8,
            116 => 9,
            110 => 10,
            118 => 11,
            102 => 12,
            114 => 13,
            101 => 27,
            115 => 32,
            100 => 127,
            48..=55 => {
                let mut n = c.to_digit(8).unwrap();
                for _ in 0..2 {
                    if let Some(d) = self.peek().and_then(|c| c.to_digit(8)) {
                        self.next()?;
                        n = n * 8 + d;
                    } else {
                        break;
                    }
                }
                if (128..=255).contains(&n) {
                    n as i64 + 0x3fff00
                } else {
                    n as i64
                }
            }
            117 | 85 | 120 => {
                let count = match c {
                    117 => 4,
                    85 => 8,
                    _ => usize::MAX,
                };
                let mut n = 0u32;
                let mut digits = 0;
                for _ in 0..count {
                    if let Some(d) = self.peek().and_then(|c| c.to_digit(16)) {
                        self.next()?;
                        n = n
                            .checked_mul(16)
                            .and_then(|n| n.checked_add(d))
                            .ok_or("character escape overflow")?;
                        digits += 1;
                    } else {
                        break;
                    }
                }
                if digits == 0 || (c != 120 && digits != count) {
                    return Err("invalid Unicode escape".into());
                }
                if c == 120 {
                    if n > 0x0fffffff {
                        return Err("Hex character out of range".into());
                    }
                    if digits < 3 && (128..=255).contains(&n) {
                        n += 0x3fff00;
                    }
                } else if n > 0x3fffff {
                    return Err("unsupported non-Unicode character".into());
                }
                n as i64
            }
            78 => self.character_name()?,
            // Unsupported modifier syntax must not silently become a different character.
            67 | 77 | 83 | 72 | 65 | 94 => {
                return Err("unsupported character modifier/name escape".into());
            }
            _ => c as i64,
        })
    }
    /// lread.c read_char_escape's `\N{NAME}', NAME read as GNU does
    /// (whitespace runs fold to one space, no non-ASCII). `U+XXXX' is
    /// decoded here; any other NAME is GNU's Lisp `char-from-name' with
    /// IGNORE-CASE, called as a function stream is (roots kept).
    fn character_name(&mut self) -> Result<i64> {
        if self.next()? != 123 {
            return Err("Expected opening brace after \\N".into());
        }
        let mut name = String::new();
        loop {
            let c = self.next()?;
            if c == 125 { break; }
            if c >= 0x80 {
                return Err(format!("Invalid character U+{c:04X} in character name"));
            }
            let c = char::from_u32(c).unwrap();
            if c.is_ascii_whitespace() {
                if name.ends_with(' ') { continue; }
                name.push(' ');
            } else {
                name.push(c);
            }
            if name.len() >= 200 { return Err("Character name too long".into()); }
        }
        if name.is_empty() { return Err("Empty character name".into()); }
        let Some(hex) = name.strip_prefix("U+") else {
            let function = self.vm.intern("char-from-name");
            let string = self.vm.byte_string(name.as_bytes());
            let code = self.vm.reader_call(function, &[string, Value::TRUE])?;
            return code.integer_value()
                .filter(|&code| (0..=0x10ffff).contains(&code) && !(0xd800..=0xdfff).contains(&code))
                .ok_or_else(|| format!("\\N{{{name}}}"));
        };
        u32::from_str_radix(hex, 16).ok()
            .filter(|&code| !hex.starts_with('+') && code <= 0x10ffff && !(0xd800..=0xdfff).contains(&code))
            .map(i64::from)
            .ok_or_else(|| format!("\\N{{{name}}}"))
    }
    fn form(&mut self, depth: usize) -> Result<Value> {
        #[cfg(test)]
        if self.collect_subforms {
            self.vm.byte_string(b"unreachable reader stress allocation");
        }
        let value = self.form_inner(depth)?;
        if depth > 0 {
            self.eof = false;
        }
        let index = self.vm.external_roots.len();
        self.vm.external_roots.push(value);
        #[cfg(test)]
        if self.collect_subforms {
            self.vm.collect(&mut []);
        }
        Ok(self.vm.external_roots[index])
    }
    fn form_inner(&mut self, depth: usize) -> Result<Value> {
        if depth > 256 {
            return Err("reader nesting limit exceeded".into());
        }
        self.space();
        let opening = self.next()?;
        match opening {
            40 | 91 => {
                let vector = opening == 91;
                let end = if vector { 93 } else { 41 };
                let mut values = vec![];
                let mut tail = Value::NIL;
                loop {
                    self.space();
                    if self.peek() == Some(end) {
                        self.next()?;
                        break;
                    }
                    // lread.c:4134-4140: only an RE_list (a list with an
                    // element) takes the dot; at a list start or in a vector
                    // it reaches `form_inner`'s dot arm and is refused.
                    if !vector && !values.is_empty() && self.dot_token() {
                        self.next()?;
                        // A closer where the tail belongs reaches read0's
                        // `)`/`]` case with RE_list_dot on top: its default
                        // arm signals the closer itself (lread.c:3789,3827).
                        self.space();
                        if let Some(closer @ (41 | 93)) = self.peek() {
                            self.next()?;
                            return Err(self.syntax(&char::from_u32(closer).unwrap().to_string()));
                        }
                        self.form(depth + 1)?;
                        let tail_root = self.vm.external_roots.len() - 1;
                        self.space();
                        if self.next()? != end {
                            return Err("dotted list requires one tail".into());
                        }
                        tail = self.vm.external_roots[tail_root];
                        break;
                    }
                    self.form(depth + 1)?;
                    values.push(self.vm.external_roots.len() - 1);
                }
                if vector {
                    let values = values
                        .into_iter()
                        .map(|i| self.vm.external_roots[i])
                        .collect();
                    Ok(self.vm.allocate_object(Object::Vector(values)))
                } else {
                    for index in values.into_iter().rev() {
                        let value = self.vm.external_roots[index];
                        self.vm.conses.push((value, tail));
                        tail = Value::reference(self.vm.conses.len() - 1, 3);
                    }
                    Ok(tail)
                }
            }
            41 | 93 => Err(self.syntax(&char::from_u32(opening).unwrap().to_string())),
            // A dot token where no list can take it: top level, a tail
            // position, a list start, a vector, a record (lread.c:4140).
            46 if Self::dot_follows(self.peek()) => {
                self.unread_lookahead()?;
                Err(self.syntax("."))
            },
            39 => self.prefix("quote", depth),
            96 => self.prefix("`", depth),
            44 => {
                let splice = self.peek() == Some(64);
                if splice {
                    self.next()?;
                }
                self.prefix(if splice { ",@" } else { "," }, depth)
            }
            35 => match self.next()? {
                39 => self.prefix("function", depth),
                115 => self.record(depth),
                94 => self.table(depth),
                40 => self.property_string(depth),
                38 => self.bool_vector(depth),
                98 | 66 => self.radix_integer(2),
                111 | 79 => self.radix_integer(8),
                120 | 88 => self.radix_integer(16),
                c @ 48..=57 => self.numbered_dispatch(c, depth),
                // ## -- the interned symbol whose name is empty.
                35 => Ok(self.vm.intern("")),
                // #:X uninterned, #_X without shorthand: a
                // symbol whatever the token looks like; nothing after it is
                // the empty name.
                dispatch @ (58 | 95) => {
                    let uninterned = dispatch == 58;
                    let name = match self.peek() {
                        Some(c) if !Self::empty_symbol_follows(c) => { let first = self.next()?; self.symbol_token(first)?.0 }
                        _ => String::new(),
                    };
                    Ok(if uninterned { self.vm.uninterned_symbol(&name) } else { self.vm.intern(&name) })
                }
                // #! starts an executable file's first line: skip it.
                33 => {
                    while self.peek().is_some_and(|c| c != 10) { self.next()?; }
                    self.form_inner(depth)
                }
                _ => Err("unsupported # reader syntax".into()),
            },
            63 => {
                if matches!(self.peek(), Some(9 | 32)) {
                    return Value::integer(i64::from(self.next()?));
                }
                let n = if self.peek() == Some(92) {
                    self.next()?;
                    self.escape()?
                } else {
                    self.next()? as i64
                };
                let following = self.peek();
                self.unread_lookahead()?;
                if following.is_some_and(|c| !Self::char_literal_follows(c)) {
                    return Err("invalid character literal".into());
                }
                let base = n & !0x0fc00000;
                let n = if (0x3fff80..=0x3fffff).contains(&base) {
                    (n & 0x0fc00000) | (base - 0x3fff00)
                } else {
                    n
                };
                Value::integer(n)
            }
            34 => {
                let mut s: Vec<u32> = Vec::new();

                let mut unicode = false;
                loop {
                    match self.next()? {
                        34 => break,
                        92 if matches!(self.peek(), Some(10 | 32)) => {
                            self.next()?;
                        }
                        92 if self.peek() == Some(115) => {
                            self.next()?;
                            s.push(32);
                        }
                        92 => {
                            let n = self.escape()?;
                            let mut modifiers = n & 0x0fc00000;
                            let mut ch = n & !0x0fc00000;
                            let mut raw_char = false;
                            if (0x3fff80..=0x3fffff).contains(&ch) {
                                raw_char = true;
                                ch -= 0x3fff00;
                            } else if ch >= 128 {
                                unicode = true;
                            }
                            if modifiers == 1 << 26 && ch == 32 {
                                ch = 0;
                                modifiers = 0;
                            }
                            if ch < 128 {
                                if modifiers & (1 << 25) != 0 && (65..=90).contains(&ch) {
                                    modifiers &= !(1 << 25);
                                } else if modifiers & (1 << 25) != 0 && (97..=122).contains(&ch) {
                                    ch -= 32;
                                    modifiers &= !(1 << 25);
                                }
                                if modifiers & (1 << 27) != 0 {
                                    raw_char = true;
                                    ch |= 128;
                                    modifiers &= !(1 << 27);
                                }
                            }
                            if modifiers != 0 {
                                return Err("Invalid modifier in string".into());
                            }
                            if ch > 0x3fffff {
                                return Err("Invalid character in string".into());
                            }
                            s.push(if raw_char && (128..=255).contains(&ch) {
                                ch as u32 + 0x3fff00
                            } else {
                                ch as u32
                            });
                        }
                        c => {
                            if (128..0x3fff80).contains(&c) {
                                unicode = true;
                            }
                            s.push(c);
                        }
                    }
                }
                // Preserve the host-source compiler adapter during migration;
                // Lisp stream literals follow GNU content-based multibyteness.
                if !unicode && !(matches!(self.input, Input::Host { .. }) && s.iter().all(|c| *c < 128)) {
                    let bytes = s
                        .into_iter()
                        .map(|c| {
                            if c >= 0x3fff80 {
                                (c - 0x3fff00) as u8
                            } else {
                                c as u8
                            }
                        })
                        .collect::<Vec<_>>();
                    Ok(self.vm.allocate_object(Object::String(Box::new(
                        crate::data::LispString::from_bytes(bytes, false),
                    ))))
                } else {
                    let mut bytes = Vec::new();
                    for c in s {
                        let mut encoded = [0; 5];
                        let n = crate::lstring::char_string(c, &mut encoded);
                        bytes.extend_from_slice(&encoded[..n]);
                    }
                    Ok(self.vm.make_lisp_string(bytes, true))
                }
            }
            first => {
                let (token, escaped) = self.symbol_token(first)?;
                if !escaped {
                    // GNU also accepts decimal integers with a trailing dot.
                    let integer = token.strip_suffix('.').unwrap_or(&token);
                    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
                    if !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()) {
                        // A fixnum when it fits, else a bignum.
                        let n: num_bigint::BigInt = integer.parse().map_err(|_| "invalid integer")?;
                        return Ok(self.vm.bigint(n));
                    }
                    // A lone `.' token here was followed by `)` or `]`,
                    // which GNU's dot test excludes (`dot_follows`): it is
                    // the symbol `.`, so `(a .)` is a list of `a` and `.`.
                    // Numeric-looking floats/bignums are not interned as symbols.
                    // Rust also parses "nan" and "inf" as floats; GNU interns
                    // those names. Only numeric-leading tokens can be numbers.
                    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(&token);
                    let numeric_leading = unsigned.starts_with(|c: char| c.is_ascii_digit())
                        || unsigned
                            .strip_prefix('.')
                            .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()));
                    if numeric_leading && (token.contains('.') || token.contains(['e', 'E'])) {
                        let special = match token.as_str() {
                            "1.0e+INF" | "+1.0e+INF" => Some(f64::INFINITY),
                            "-1.0e+INF" => Some(f64::NEG_INFINITY),
                            "0.0e+NaN" | "+0.0e+NaN" => Some(f64::NAN),
                            "-0.0e+NaN" => Some(-f64::NAN),
                            _ => None,
                        };
                        if let Some(n) = special {
                            return Ok(self.vm.allocate_object(Object::Float(n)));
                        }
                        if let Ok(n) = token.parse::<f64>() {
                            return Ok(self.vm.allocate_object(Object::Float(n)));
                        }
                    }
                }
                let longhand = self.vm.shorthand_longhand(&token)?;
                Ok(self.vm.intern(longhand.as_deref().unwrap_or(&token)))
            }
        }
    }
}

impl Vm {
    /// lread.c oblookup_considering_shorthand: the name TOKEN reads as under
    /// `read-symbol-shorthands', the first (SHORTHAND . LONGHAND) pair of
    /// strings whose SHORTHAND prefixes it, or None. A name made only of
    /// `^*+-/<=>_|' (symbol_char_span) is exempt, as is `#_' syntax.
    fn shorthand_longhand(&mut self, token: &str) -> Result<Option<String>> {
        let Some(&slot) = self.names.get("read-symbol-shorthands") else { return Ok(None) };
        let shorthands = self.variable_value(slot)?;
        if matches!(shorthands, Value::NIL | Value::UNBOUND) || token.bytes().all(|b| b"^*+-/<=>_|".contains(&b)) {
            return Ok(None);
        }
        let mut tail = shorthands;
        for _ in 0..=self.list_limit() {
            let Ok((pair, next)) = self.cons_pair(tail) else { break };
            if let Ok((short, long)) = self.cons_pair(pair)
                && let (Some(short), Some(long)) = (self.str_view(short), self.str_view(long))
                && let Some(rest) = token.as_bytes().strip_prefix(short.bytes.as_ref())
            {
                let mut name = long.bytes.to_vec();
                name.extend_from_slice(rest);
                return Ok(Some(String::from_utf8_lossy(&name).into_owned()));
            }
            tail = next;
        }
        Ok(None)
    }
    /// A fresh symbol outside the obarray, as `make-symbol' makes.
    pub(crate) fn uninterned_symbol(&mut self, name: &str) -> Value {
        let value = self.generated_symbol(crate::expand::generated_id());
        let slot = value.index_if(7).unwrap() - 1;
        self.symbol_names[slot] = name.to_owned();
        value
    }
    /// Interning is stable for this VM's lifetime, independent of moving GC.
    pub(crate) fn intern(&mut self, name: &str) -> Value {
        match name {
            "nil" => Value::NIL,
            "t" => Value::TRUE,
            _ => {
                let slot = self.slot(name);
                Value::reference(slot + 1, 7)
            }
        }
    }
    /// Read one datum, returning its UTF-8 byte end offset. Comments/whitespace
    /// alone return None. Root the value explicitly if retaining across execution.
    pub(crate) fn read_source_raw(&mut self, source: &str) -> Result<Option<(Value, usize)>> {
        self.read_source_span(source)
            .map(|v| v.map(|(value, _, end)| (value, end)))
    }
    pub(super) fn read_source_span(
        &mut self,
        source: &str,
    ) -> Result<Option<(Value, usize, usize)>> {
        self.read_internal_span(source.as_bytes())
    }
    pub(super) fn read_internal_span(
        &mut self,
        source: &[u8],
    ) -> Result<Option<(Value, usize, usize)>> {
        self.read_internal_signal(source).map_err(|(text, _)| text)
    }
    /// `read_source_span`, also returning GNU's exact `(CONDITION . DATA)`
    /// when the reader knows it. Both values are unrooted: use them before
    /// anything can collect (a leaf primitive signalling them does).
    pub(super) fn read_source_signal(
        &mut self,
        source: &str,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        self.read_internal_signal(source.as_bytes())
    }
    /// `read_source_signal` over GNU internal multibyte text.
    pub(super) fn read_internal_signal(
        &mut self,
        source: &[u8],
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        self.read_input(Input::Host { text: source, byte: 0 })
    }
    fn read_input(
        &mut self,
        input: Input<'_>,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        let _unwind = self.unwind_mark();
        let roots = self.external_roots.len();
        let initial_byte = match &input { Input::String { byte, .. } => *byte, _ => 0 };
        let mut reader = Reader {
            input,
            at: 0,
            lookahead: None,
            error: None,
            eof: false,
            labels: BTreeMap::new(),
            vm: self,
            signal: None,
            #[cfg(test)]
            collect_subforms: false,
        };
        reader.space();
        let begin = reader.at;
        let result = if reader.peek().is_none() {
            Ok(None)
        } else {
            reader.form(0).map(|_| Some((reader.vm.external_roots.len() - 1, begin, reader.at)))
        };
        let result = result.and_then(|r| {
            reader.finish()?;
            Ok(r.map(|(root, begin, end)| (reader.vm.external_roots[root], begin, end)))
        });
        let signal = reader
            .signal
            .take()
            .or_else(|| reader.vm.pending_eval_escape());
        reader.vm.external_roots.truncate(roots);
        result.map_err(|message| {
            let bytes = match reader.input {
                Input::Host { .. } => Some(reader.at),
                Input::String { byte, root, .. } => {
                    let pending = reader.lookahead.map_or(0, |code| {
                        if reader.vm.str_view(reader.vm.external_roots[root]).is_some_and(|view| view.is_multibyte()) {
                            let mut encoded = [0; 5];
                            crate::lstring::char_string(code, &mut encoded)
                        } else { 1 }
                    });
                    Some(byte - initial_byte - pending)
                },
                _ => None,
            };
            let message = if let Some(bytes) = bytes {
                format!("{message} at byte {bytes}")
            } else { message };
            (message, signal)
        })
    }
    pub(super) fn read_lisp_string(
        &mut self,
        string: Value,
        start: usize,
        end: usize,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        let root = self.external_roots.len();
        self.external_roots.push(string);
        let view = self.str_view(string).unwrap();
        let byte = if view.is_multibyte() {
            view.chars()
                .take(start)
                .map(|c| {
                    let mut b = [0; 5];
                    crate::lstring::char_string(c, &mut b)
                })
                .sum()
        } else {
            start
        };
        let result = self.read_input(Input::String {
            root,
            byte,
            character: start,
            end,
        });
        self.external_roots.truncate(root);
        result
    }
    pub(super) fn read_lisp_stream(
        &mut self,
        stream: Value,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        let root = self.external_roots.len();
        self.external_roots.push(stream);
        let input = if let Some(view) = self.str_view(stream) {
            Input::String {
                root,
                byte: 0,
                character: 0,
                end: view.len(),
            }
        } else {
            match self.object_if(stream) {
                Some(Object::Buffer(_) | Object::ByteBuffer(_)) => Input::Buffer { root },
                Some(Object::Marker(_)) => Input::Marker { root },
                _ => Input::Function { root },
            }
        };
        let result = self.read_input(input);
        self.external_roots.truncate(root);
        result
    }
    fn reader_buffer_location(&self, source: Value) -> Result<(Value, usize, bool)> {
        if let Some(Object::Marker(m)) = self.object_if(source) {
            let anchor = m.anchor.ok_or("Reader refuses a detached marker")?;
            self.require_live_buffer(m.buffer)?;
            let at = self.marker_anchors(m.buffer)?.resolve(anchor).ok_or("invalid marker anchor")? as usize;
            Ok((m.buffer, at, true))
        } else {
            self.require_live_buffer(source)?;
            let at = match self.object(source)? {
                Object::Buffer(b) => b.point.0,
                Object::ByteBuffer(b) => b.point.0,
                _ => unreachable!(),
            };
            Ok((source, at, false))
        }
    }
    fn reader_buffer_position(
        &mut self,
        source: Value,
        buffer: Value,
        at: usize,
        marker: bool,
    ) -> Result<()> {
        if marker {
            let Object::Marker(m) = self.object(source)? else {
                unreachable!()
            };
            let (anchor, bias) = (m.anchor.unwrap(), m.bias);
            if !self.marker_anchors_mut(buffer)?.set(anchor, at as u32, bias) {
                return Err("invalid marker anchor".into());
            }
        } else {
            match &mut self.objects[buffer.index(0)? - 1] {
                Object::Buffer(b) => b.point = ByteOffset(at),
                Object::ByteBuffer(b) => b.point = ByteOffset(at),
                _ => unreachable!(),
            }
        }
        Ok(())
    }
    fn reader_buffer_get(&mut self, source: Value) -> Result<Option<u32>> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let (code, width) = match self.object(buffer)? {
            Object::Buffer(b) => {
                if at >= b.accessible().end.0 {
                    return Ok(None);
                }
                let c = b.as_str()[at..]
                    .chars()
                    .next()
                    .ok_or("invalid buffer reader point")?;
                (c as u32, c.len_utf8())
            }
            Object::ByteBuffer(b) => {
                if at >= b.restriction.end.0 {
                    return Ok(None);
                }
                let c = *b.bytes.get(at).ok_or("invalid byte buffer reader point")? as u32;
                (if c >= 128 { c + 0x3fff00 } else { c }, 1)
            }
            _ => unreachable!(),
        };
        self.reader_buffer_position(source, buffer, at + width, marker)?;
        Ok(Some(code))
    }
    fn reader_buffer_unget(&mut self, source: Value, code: u32) -> Result<()> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let width = match self.object(buffer)? {
            Object::Buffer(_) => char::from_u32(code)
                .ok_or("raw character cannot be unread into Unicode buffer")?
                .len_utf8(),
            _ => 1,
        };
        self.reader_buffer_position(
            source,
            buffer,
            at.checked_sub(width)
                .ok_or("reader unread before buffer start")?,
            marker,
        )
    }

    pub(super) fn source_form(&self, value: Value, depth: usize) -> Result<Form> {
        self.datum_form(value, depth, false)
    }
    /// Like `source_form`, but an object with no source spelling becomes
    /// `Form::Object` instead of an error: GNU evaluates the object itself
    /// (src/eval.c eval_sub). Object constants own traced roots until every
    /// clone of the form is dropped. These VM-local handles are not persistent.
    pub(super) fn expansion_form(&self, value: Value, depth: usize) -> Result<Form> {
        self.datum_form(value, depth, true)
    }
    fn datum_form(&self, value: Value, depth: usize, objects: bool) -> Result<Form> {
        if depth > 256 {
            return Err("source nesting limit exceeded".into());
        }
        if value == Value::NIL {
            return Ok(Form::Sym("nil".into()));
        }
        if value == Value::TRUE {
            return Ok(Form::Sym("t".into()));
        }
        match value.tag() {
            1 => Ok(Form::Int(value.as_integer()?)),
            7 => {
                let slot = value.index(7)? - 1;
                Ok(if let Some(&id) = self.generated_slots.get(&slot) {
                    Form::Generated(id)
                } else {
                    Form::Sym(self.symbol_names[slot].clone())
                })
            }
            0 => match self.object(value)? {
                Object::Bignum(_)
                | Object::Record(_)
                | Object::BoolVector(_)
                | Object::HashTable(_)
                | Object::CharTable(_)
                | Object::SubCharTable(_) => {
                    Ok(Form::Object(crate::handle::FormObject::new(self, value)))
                }
                Object::Float(n) => Ok(Form::Float(*n)),
                Object::Bytes(bytes) => Ok(Form::Bytes(bytes.clone())),
                Object::String(s) | Object::RawString(s) if !s.properties.is_empty() => {
                    Ok(Form::Object(crate::handle::FormObject::new(self, value)))
                }
                Object::String(s) | Object::RawString(s) if !s.is_multibyte() => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                Object::String(_) => Ok(Form::String(
                    String::from_utf8(self.string_bytes(value)?).map_err(|e| e.to_string())?,
                )),
                Object::RawString(_) => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                Object::Vector(a) => Ok(Form::Vector(
                    a.iter()
                        .map(|v| self.datum_form(*v, depth + 1, objects))
                        .collect::<Result<_>>()?,
                )),
                _ if objects => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                _ => Err("source datum required".into()),
            },
            3 => {
                let mut values = vec![];
                let mut tail = value;
                while tail.tag() == 3 {
                    let (car, cdr) = self.cons_pair(tail)?;
                    values.push(self.datum_form(car, depth + 1, objects)?);
                    tail = cdr;
                    if values.len() > self.list_limit() {
                        return Err("circular source list".into());
                    }
                }
                if tail == Value::NIL {
                    Ok(Form::List(values))
                } else {
                    Ok(Form::Dotted(
                        values,
                        Box::new(self.datum_form(tail, depth + 1, objects)?),
                    ))
                }
            }
            // Closures and primitives: function objects with no spelling.
            4 | 6 if objects => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
            _ => Err("source datum required".into()),
        }
    }
    /// Compile source as one progn for readable IR inspection, without executing.
    /// Source-file eval still executes top-level forms sequentially.
    pub fn compile_source(&mut self, source: &str) -> Result<usize> {
        let form = self.source_progn(source)?;
        self.compile(&form)
    }
    pub(super) fn request_source_ir(&mut self, source: &str) -> Result<String> {
        let outcome = (|| {
            let form = self.source_progn(source)?;
            let (id, _) = self.compile_request(&form)?;
            self.dump_ir(id)
        })();
        self.finish_request(outcome.as_ref().map(|_| Value::NIL).map_err(Clone::clone))?;
        outcome
    }
    fn source_progn(&mut self, mut source: &str) -> Result<Form> {
        let mut forms = vec![Form::Sym("progn".into())];
        while let Some((value, used)) = self.read_source_raw(source)? {
            forms.push(self.source_form(value, 0)?);
            source = &source[used..];
        }
        Ok(Form::List(forms))
    }
    /// Evaluate a UTF-8 source file sequentially in this persistent strand. No
    /// reader Values survive an executor safepoint: the compiler roots literals.
    ///
    /// This call may mark/compact even after a short request or error.
    /// Only the returned `Value` is rewritten for the embedder. Other raw heap
    /// `Value`s held by the caller must not be reused after this call. Short
    /// requests previously rarely collected; that was not a lifetime guarantee.
    /// Use `compile` and `execute_rooted` with mutable roots to retain such values.
    pub(crate) fn eval_source_raw(&mut self, source: &str) -> Result<Value> {
        let outcome = self.eval_source_inner(source);
        self.finish_request(outcome)
    }
    fn eval_source_inner(&mut self, source: &str) -> Result<Value> {
        if let Some(f) = self.cached_submission(source) {
            return self.execute_raw(f);
        }
        // The last form's value is rooted across reading the next one, which
        // can call Lisp (a `\N{NAME}' character).
        let last = self.root_frame(&[Value::NIL]);
        let outcome = self.eval_source_forms(source, &last);
        let [value] = self.release_frame(last);
        outcome.map(|()| value)
    }
    fn eval_source_forms(&mut self, mut source: &str, last: &crate::gc::RootFrame) -> Result<()> {
        while let Some((value, used)) = self.read_source_raw(source)? {
            let form = self.source_form(value, 0)?;
            let result = if let Some(result) = self.direct_form(&form) {
                result?
            } else {
                let function = self.cached_source_form(&source[..used], &form)?;
                self.execute_raw(function)?
            };
            self.set_rooted(last, 0, result);
            source = &source[used..];
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("reader_oracle_tests.rs");

    #[test]
    fn reader_string_basic() {
        let mut vm = Vm::new();
        let value = vm
            .eval_source_raw(r#"(read "(a 17 . b) trailing")"#)
            .unwrap();
        assert_eq!(vm.display(value).unwrap(), "(a 17 . b)");
    }

    #[test]
    fn partial_reader_forms_survive_compaction_between_subforms() {
        let mut vm = Vm::new();
        let source = r#"("first" ["second" ("third")] '"fourth" . "tail")"#;
        let mut reader = Reader {
            input: Input::Host {
                text: source.as_bytes(),
                byte: 0,
            },
            at: 0,
            lookahead: None,
            error: None,
            eof: false,
            labels: BTreeMap::new(),
            vm: &mut vm,
            signal: None,
            collect_subforms: true,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reader.form(0).and_then(|v| reader.vm.display(v))
        }));
        assert!(
            result.is_ok(),
            "partial reader forms must remain valid during compaction"
        );
        let result = result.unwrap();
        assert_eq!(
            result,
            Ok(r#"("first" ["second" ("third")] (quote "fourth") . "tail")"#.to_owned()),
            "partial reader forms must be forwarded"
        );
    }
}
