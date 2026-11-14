//! The reader's grammar: characters from its input kinds, tokens and forms.
//! It is orchestration (`crate::brand`): it holds plain data only (positions,
//! tokens, the indices of what it has read so far in `external_roots`), and
//! its raw work runs in leaves (`build`). So nothing it holds goes stale when
//! a stream's Lisp or `char-from-name` collects, and the compiler says so.
use super::build;
use crate::brand::{Gc, Mx, Plain};
use crate::gc::RootFrame;
use crate::{Result, Vm};
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

pub(super) enum Input<'a> {
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
    /// A buffer, read from its point, or a marker, from its position.
    Buffer {
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

/// One read: orchestration over the VM. Lisp (a function stream, `\N{NAME}`
/// by name) runs only under GC, the permission; a read without one refuses it.
struct Reader<'a, 'r, 'm, 'b, 'g> {
    m: &'r mut Mx<'m, 'b, Vm>,
    gc: Option<Gc<'g, 'b>>,
    input: Input<'a>,
    at: usize,
    lookahead: Option<u32>,
    error: Option<String>,
    eof: bool,
    labels: BTreeMap<u64, usize>,
    /// The root of a `(CONDITION . DATA)` the read failed with, when known.
    signal: Option<usize>,
    #[cfg(test)]
    collect_subforms: bool,
}
/// The character at the start of BYTES of a Lisp string read as a stream,
/// and its width: a unibyte string's bytes above 127 are raw bytes.
pub(super) fn string_input_char(bytes: &[u8], multibyte: bool) -> Result<(u32, usize)> {
    if !multibyte {
        let c = bytes[0] as u32;
        return Ok((if c >= 128 { c + 0x3fff00 } else { c }, 1));
    }
    let width = match bytes[0] {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        0xf8 => 5,
        _ => return Err("invalid string character".into()),
    };
    let bytes = bytes.get(..width).ok_or("truncated string character")?;
    Ok(crate::lstring::string_char(bytes))
}

impl Reader<'_, '_, '_, '_, '_> {
    /// FRAME's function called on the rest of it (a stream's function, or
    /// `char-from-name`), and the frame dropped: its value, when an integer.
    /// A read without the permission refuses: it reads Romacs's own text.
    fn call(&mut self, frame: RootFrame) -> Result<Option<i64>> {
        let called = match self.gc.as_mut() {
            Some(gc) => crate::threaded::call_frame(self.m, gc.by_ref(), &frame, (), |_, (), value| value.map(crate::Value::integer_value)),
            None => Err("this read calls no Lisp: no function stream, and no `\\N{NAME}' by name".into()),
        };
        self.m.leaf(frame, Vm::drop_frame);
        called
    }
    /// Inside a string literal read from a Lisp string, the characters up
    /// to the next quote or backslash, taken in one pass over the string's bytes
    /// instead of one stream read each: pushed onto S.  Whether any is a
    /// non-ASCII character.  Stops early, leaving the rest to `next', at
    /// anything unusual (a lookahead, an error, a malformed character).
    fn string_literal_run(&mut self, s: &mut Vec<u32>) -> bool {
        let (None, None, &Input::String { root, byte, character, end }) = (self.lookahead, &self.error, &self.input) else { return false };
        let (run, unicode, byte, after) = self.m.leaf((std::mem::take(s), (root, byte, character, end)), build::string_run);
        *s = run;
        self.at += after - character;
        self.input = Input::String { root, byte, character: after, end };
        unicode
    }
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
                let (c, width) = self.m.leaf((*root, *byte), build::string_char)?;
                *character += 1;
                *byte += width;
                Ok(Some(c))
            }
            Input::Function { root } => {
                let frame = self.m.leaf((*root, None), build::stream_call)?;
                Ok(self.call(frame)?.filter(|n| (0..=0x3fffff).contains(n)).map(|n| n as u32))
            }
            Input::Buffer { root } => self.m.leaf(*root, build::buffer_get),
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
                let width = self.m.leaf((*root, c), build::string_width)?;
                *character -= 1;
                *byte -= width;
            }
            Input::Function { root } => {
                let frame = self.m.leaf((*root, Some(c)), build::stream_call)?;
                self.call(frame)?;
            }
            Input::Buffer { root } => self.m.leaf((*root, c), build::buffer_unget)?,
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
                    self.signal = self.m.leaf((), build::pending_signal);
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
    /// A failure's text, its GNU condition (rooted, when known) recorded as
    /// terminal: no subsequent parse or callback runs.
    fn fail(&mut self, (text, signal): (String, Option<usize>)) -> String {
        if signal.is_some() {
            self.signal = signal;
        }
        text
    }
    fn syntax(&mut self, datum: &str) -> String {
        let failure = self.m.leaf(datum, build::syntax);
        self.fail(failure)
    }
    /// What a leaf built: its root, or its failure recorded.
    fn built(&mut self, built: build::Built) -> Result<usize> {
        built.map_err(|failure| self.fail(failure))
    }
    /// GNU lread.c read_bool_vector: `#&LENGTH"BYTES"`, a unibyte string of
    /// exactly ceil(LENGTH/8) bytes (or one more, as Emacs 19 printed it).
    fn bool_vector(&mut self, depth: usize) -> Result<usize> {
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
        let built = self.m.leaf((length, string), build::bool_vector);
        self.built(built)
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
            roots.push(self.form(depth + 1)?);
        }
    }
    fn table(&mut self, depth: usize) -> Result<usize> {
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
        let built = self.m.leaf((sub, roots), build::char_table);
        self.built(built)
    }
    fn property_string(&mut self, depth: usize) -> Result<usize> {
        let roots = self.aggregate(41, depth)?;
        let built = self.m.leaf(roots.as_slice(), build::property_string);
        self.built(built)
    }
    fn record(&mut self, depth: usize) -> Result<usize> {
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
            items.push(self.form(depth + 1)?);
        }
        let built = self.m.leaf(items.as_slice(), build::record);
        self.built(built)
    }
    /// GNU lread.c:2942 read_integer: scan all ASCII alphanumerics,
    /// poisoning the entire integer on an out-of-base digit. Punctuation is
    /// left unread, even when it is not an ordinary Lisp token delimiter.
    fn radix_integer(&mut self, radix: u32) -> Result<usize> {
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
        self.m.leaf((token.as_str(), radix), build::radix_integer)
    }
    fn numbered_dispatch(&mut self, first: u32, depth: usize) -> Result<usize> {
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
                let slot = self.m.leaf((), build::label);
                self.labels.insert(number, slot);
                let value = self.form(depth + 1)?;
                let built = self.m.leaf((slot, value), build::labelled);
                self.built(built)
            }
            35 => match self.labels.get(&number).copied() {
                Some(slot) => Ok(self.m.leaf(slot, build::reference)),
                None => Err(self.syntax("undefined numbered reference")),
            },
            _ => Err(self.syntax("#")),
        }
    }
    fn prefix(&mut self, name: &str, depth: usize) -> Result<usize> {
        let symbol = self.m.leaf((name, true), build::symbol);
        let value = self.form(depth + 1)?;
        Ok(self.m.leaf((symbol, value), build::prefixed))
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
    /// IGNORE-CASE, called as a function stream is.
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
            let frame = self.m.leaf(name.as_str(), build::name_call);
            return self.call(frame)?
                .filter(|&code| (0..=0x10ffff).contains(&code) && !(0xd800..=0xdfff).contains(&code))
                .ok_or_else(|| format!("\\N{{{name}}}"));
        };
        u32::from_str_radix(hex, 16).ok()
            .filter(|&code| !hex.starts_with('+') && code <= 0x10ffff && !(0xd800..=0xdfff).contains(&code))
            .map(i64::from)
            .ok_or_else(|| format!("\\N{{{name}}}"))
    }
    /// The next form, rooted: its index in `external_roots`.
    fn form(&mut self, depth: usize) -> Result<usize> {
        #[cfg(test)]
        if self.collect_subforms {
            self.m.leaf((), |vm, ()| { vm.byte_string(b"unreachable reader stress allocation"); });
        }
        let root = self.form_inner(depth)?;
        if depth > 0 {
            self.eof = false;
        }
        #[cfg(test)]
        if self.collect_subforms && let Some(gc) = self.gc.as_mut() {
            crate::threaded::collect_now(self.m, gc.by_ref());
        }
        Ok(root)
    }
    fn form_inner(&mut self, depth: usize) -> Result<usize> {
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
                let mut tail = None;
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
                        let root = self.form(depth + 1)?;
                        self.space();
                        if self.next()? != end {
                            return Err("dotted list requires one tail".into());
                        }
                        tail = Some(root);
                        break;
                    }
                    values.push(self.form(depth + 1)?);
                }
                Ok(self.m.leaf((vector, values, tail), build::sequence))
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
                35 => Ok(self.m.leaf(("", true), build::symbol)),
                // #:X uninterned, #_X without shorthand: a
                // symbol whatever the token looks like; nothing after it is
                // the empty name.
                dispatch @ (58 | 95) => {
                    let uninterned = dispatch == 58;
                    let name = match self.peek() {
                        Some(c) if !Self::empty_symbol_follows(c) => { let first = self.next()?; self.symbol_token(first)?.0 }
                        _ => String::new(),
                    };
                    Ok(self.m.leaf((name.as_str(), !uninterned), build::symbol))
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
                    let c = self.next()?;
                    return self.m.leaf(i64::from(c), build::integer);
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
                self.m.leaf(n, build::integer)
            }
            34 => {
                let mut s: Vec<u32> = Vec::new();

                let mut unicode = false;
                loop {
                    unicode |= self.string_literal_run(&mut s);
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
                // GNU read_string_literal: multibyte only for a character
                // above 127 that is not a raw byte, whatever the source.
                Ok(self.m.leaf((s, unicode), build::string_literal))
            }
            first => {
                let (token, escaped) = self.symbol_token(first)?;
                self.m.leaf((token.as_str(), escaped), build::atom)
            }
        }
    }
}

/// One read of INPUT, under GC when it may call Lisp: THEN gets what it read
/// (`super::Read`) at once, in a leaf, the read's roots already released, as
/// a request's value goes straight to its leaf.
pub(super) fn read_input<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Vm>, gc: Option<Gc<'_, 'b>>, input: Input<'_>, b: B,
    then: impl FnOnce(&mut Vm, B, super::Read) -> R + Copy + 'static) -> R {
    let mark = m.leaf((), |vm, ()| vm.external_roots.len());
    let initial_byte = match &input { Input::String { byte, .. } => *byte, _ => 0 };
    let mut reader = Reader {
        m,
        gc,
        input,
        at: 0,
        lookahead: None,
        error: None,
        eof: false,
        labels: BTreeMap::new(),
        signal: None,
        #[cfg(test)]
        collect_subforms: false,
    };
    reader.space();
    let begin = reader.at;
    let result = if reader.peek().is_none() {
        Ok(None)
    } else {
        reader.form(0).map(|root| Some((root, begin, reader.at)))
    };
    let result = result.and_then(|r| {
        reader.finish()?;
        Ok(r)
    });
    let result = match result {
        Ok(found) => Ok(found),
        Err(message) => {
            let bytes = match reader.input {
                Input::Host { .. } => Some(reader.at),
                Input::String { byte, root, .. } => {
                    let pending = match reader.lookahead {
                        Some(code) => reader.m.leaf((root, code), build::string_width).unwrap_or(1),
                        None => 0,
                    };
                    Some(byte - initial_byte - pending)
                }
                _ => None,
            };
            Err(if let Some(bytes) = bytes { format!("{message} at byte {bytes}") } else { message })
        }
    };
    reader.m.leaf((result, reader.signal, mark, b), move |vm, (result, signal, mark, b)| {
        let read = match result {
            Ok(found) => Ok(found.map(|(root, begin, end)| (vm.external_roots[root], begin, end))),
            Err(message) => {
                let signal = signal.and_then(|root| vm.cons_pair(vm.external_roots[root]).ok());
                Err((message, signal.or_else(|| vm.pending_eval_escape())))
            }
        };
        vm.external_roots.truncate(mark);
        then(vm, b, read)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Value, data::Object};
    include!("oracle_tests.rs");

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
        let shown = crate::threaded::toplevel(&mut vm, (), |mut m, gc, ()| {
            let source = r#"("first" ["second" ("third")] '"fourth" . "tail")"#;
            let mark = m.leaf((), |vm, ()| vm.external_roots.len());
            let mut reader = Reader {
                m: &mut m,
                gc: Some(gc),
                input: Input::Host { text: source.as_bytes(), byte: 0 },
                at: 0,
                lookahead: None,
                error: None,
                eof: false,
                labels: BTreeMap::new(),
                signal: None,
                collect_subforms: true,
            };
            let root = reader.form(0);
            m.leaf((root, mark), |vm, (root, mark)| {
                let shown = root.and_then(|root| vm.display(vm.external_roots[root]));
                vm.external_roots.truncate(mark);
                shown
            })
        });
        assert_eq!(
            shown,
            Ok(r#"("first" ["second" ("third")] (quote "fourth") . "tail")"#.to_owned()),
            "partial reader forms must be forwarded"
        );
    }

    /// SOURCE's value, printed, or the VM's error.
    fn evaluated(vm: &mut Vm, source: &str) -> String {
        match vm.eval_source_raw(source) {
            Ok(value) => vm.display(value).unwrap(),
            Err(e) => format!("VM-ERROR {e}"),
        }
    }

    #[test]
    fn a_streams_throw_crosses_read() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        assert_eq!(evaluated(&mut vm, "(catch 'out (read (lambda (&optional _) (throw 'out 'thrown))))"), "thrown");
    }

    /// A stream's signal reaches `read`'s caller as the stream raised it: a
    /// handler runs once for it, not again for a signal the reader re-raised.
    #[test]
    fn a_streams_signal_reaches_its_handler_once() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        let ran = evaluated(&mut vm, r#"(let ((ran 0))
            (list (condition-case e
                      (handler-bind-1 (lambda () (read (lambda (&optional _) (error "stream"))))
                                      '(error) (lambda (_) (setq ran (1+ ran))))
                    (error e))
                  ran))"#);
        assert_eq!(ran, r#"((error "stream") 1)"#);
    }

    /// A function stream whose every call (each `get` and `unget`) collects
    /// and moves every live object (GC stress 1, through the production
    /// `read`), under a shared `#1=` list, a property string and a dotted
    /// list: what the reader has read so far stays whole.
    #[test]
    fn partial_forms_survive_a_stream_that_collects() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        let before = vm.heap_stats().collections;
        vm.set_gc_stress(1);
        let shown = evaluated(&mut vm, r#"(let* ((s "(#1=(a \"x\") #1# #(\"pro\" 0 1 (face bold)) (b . c))") (i 0)
                 (v (read (lambda (&optional c)
                            (if c (setq i (1- i))
                              (if (< i (length s)) (prog1 (aref s i) (setq i (1+ i))) nil))))))
            (list (eq (nth 0 v) (nth 1 v)) (nth 0 v) (get-text-property 0 'face (nth 2 v)) (nth 3 v)))"#);
        vm.set_gc_stress(0);
        assert!(vm.heap_stats().collections > before + 40, "a collection at every stream call");
        assert_eq!(shown, r#"(t (a "x") bold (b . c))"#);
    }

    /// `char-from-name`, called for a `\N{NAME}` by name, may collect, signal
    /// or throw, as a stream may.
    #[test]
    fn char_from_name_may_collect_signal_or_throw() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        evaluated(&mut vm, r#"(defun char-from-name (name &optional _) (and (equal name "BULLET") 8226))"#);
        vm.set_gc_stress(1);
        let read = evaluated(&mut vm, r#"(read "(\"keep\" ?\\N{BULLET} [x])")"#);
        vm.set_gc_stress(0);
        assert_eq!(read, r#"("keep" 8226 [x])"#);
        evaluated(&mut vm, r#"(defun char-from-name (&rest _) (error "named"))"#);
        assert_eq!(evaluated(&mut vm, r#"(condition-case e (read "?\\N{BULLET}") (error (car (cdr e))))"#), r#""named""#);
        evaluated(&mut vm, r"(defun char-from-name (&rest _) (throw 'name 'thrown))");
        assert_eq!(evaluated(&mut vm, r#"(catch 'name (read "?\\N{BULLET}"))"#), "thrown");
    }

    /// A read without the permission (`Vm::read_string_unlent`) calls no
    /// Lisp: it refuses a `\N{NAME}` by name, and still reads `\N{U+XXXX}`.
    #[test]
    fn a_read_without_the_permission_calls_no_lisp() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        evaluated(&mut vm, r#"(defun char-from-name (&rest _) (error "called"))"#);
        for (text, read) in [(r"?\N{BULLET}", "refused"), (r"?\N{U+2022}", "8226")] {
            let string = vm.make_lisp_string(text.as_bytes().to_vec(), true);
            let end = vm.str_view(string).unwrap().len();
            let got = match vm.read_string_unlent(string, 0, end) {
                Ok(Some((value, ..))) => vm.display(value).unwrap(),
                Ok(None) => "nothing".into(),
                Err((message, _)) if message.contains("calls no Lisp") => "refused".into(),
                Err((message, _)) => message,
            };
            assert_eq!(got, read, "{text}");
        }
    }

    /// Every Lisp half a leaf primitive installs (`lisp/romacs/vm/`) reads without
    /// the permission, as its primitive reads it at its first call: no
    /// `\N{NAME}` by name in them.
    #[test]
    fn the_leaves_lisp_halves_read_without_lisp() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/romacs/vm");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "el")).collect();
        files.sort();
        assert!(files.len() >= 10, "the halves under {}", dir.display());
        let mut vm = Vm::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let string = vm.make_lisp_string(text.into_bytes(), true);
            let end = vm.str_view(string).unwrap().len();
            let mut at = 0;
            let mut forms = 0;
            loop {
                match vm.read_string_unlent(string, at, end) {
                    Ok(Some((_, _, read))) => { at += read; forms += 1; }
                    Ok(None) => break,
                    Err((message, _)) => panic!("{}: form {forms}: {message}", file.display()),
                }
            }
            assert!(forms > 0, "{}", file.display());
        }
    }
}
