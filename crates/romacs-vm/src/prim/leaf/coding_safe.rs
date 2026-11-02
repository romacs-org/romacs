//! coding.c's safe-coding search: which coding systems can encode a text,
//! as `select-safe-coding-system' asks before writing or hashing.
use super::*;
use crate::coordinates::CharOffset;
use crate::data::Object;

/// Text a primitive scans: internal bytes, their character count, and
/// whether they are multibyte.
pub(crate) struct TextSpan { pub bytes: Vec<u8>, pub chars: usize, pub multibyte: bool }

impl TextSpan {
    /// Only ASCII: what needs no encoding and has only the ascii charset.
    pub(crate) fn ascii(&self) -> bool { if self.multibyte { self.chars == self.bytes.len() } else { self.bytes.is_ascii() } }
    pub(crate) fn view(&self) -> crate::lstring::StrView<'_> {
        crate::lstring::StrView { bytes: &self.bytes, chars: self.chars, multibyte: self.multibyte }
    }
}

impl Leaf<'_> {
    /// get_translation_table (ATTRS, true) as translate_char applies it: its
    /// char-tables in order, none when translation is off. A nil own table
    /// leaves the standard value as it is; otherwise the own table (a symbol
    /// read through its `translation-table' property, or a list whose symbol
    /// elements are) comes first, then the standard one if a char-table.
    pub(crate) fn coding_encode_translation(&mut self, attrs: Value) -> std::result::Result<Vec<Value>, Fault> {
        let enabled = self.intern("enable-character-translation");
        if self.variable_value(enabled)?.unwrap_or(Value::NIL) == Value::NIL { return Ok(Vec::new()); }
        let standard = self.intern("standard-translation-table-for-encode");
        let standard = self.variable_value(standard)?.unwrap_or(Value::NIL);
        let own = self.vector_get(attrs, Value::integer(7).expect("index"))?;
        let property = self.intern("translation-table");
        let table_of = |leaf: &mut Self, v: Value| -> std::result::Result<Value, Fault> {
            if leaf.kind(v) == Kind::Symbol && v != Value::NIL { leaf.vm.symbol_property(v, property).map_err(Fault::Error) } else { Ok(v) }
        };
        let mut tables = Vec::new();
        if own == Value::NIL {
            tables.push(standard);
        } else {
            if self.pair(own).is_some() {
                for v in self.coding_items(own)? { tables.push(table_of(self, v)?); }
            } else {
                tables.push(table_of(self, own)?);
            }
            if self.kind(standard) == Kind::CharTable { tables.push(standard); }
        }
        let mut out = Vec::new();
        let mut seen = HashSet::default();
        for table in tables { self.translation_char_tables(table, &mut seen, &mut out)?; }
        Ok(out)
    }
    /// translate_char's walk: a char-table, or the tables of each element
    /// of a list; anything else translates nothing.
    pub(crate) fn translation_char_tables(&self, table: Value, seen: &mut HashSet<u64>, out: &mut Vec<Value>) -> std::result::Result<(), Fault> {
        if self.kind(table) == Kind::CharTable { out.push(table); return Ok(()); }
        let mut tail = table;
        while let Some((car, cdr)) = self.pair(tail) {
            if !seen.insert(tail.0) { return Err(Fault::Circular(table)); }
            self.translation_char_tables(car, seen, out)?;
            tail = cdr;
        }
        Ok(())
    }
    /// char_encodable_p: some charset of the coding system encodes C after
    /// translation (CHAR_CHARSET_P is encodability).
    fn coding_encodable(&mut self, charsets: &[usize], tables: &[Value], c: u32) -> std::result::Result<bool, Fault> {
        let mut c = c;
        for &table in tables {
            let to = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
            if let Some(to) = to.integer_value().filter(|to| (0..=i64::from(crate::lstring::MAX_CHAR)).contains(to)) { c = to as u32; }
        }
        for &id in charsets {
            let record = self.vm.charsets.ids[id];
            if self.charset_encode(record, c)?.is_some() { return Ok(true); }
        }
        Ok(false)
    }
    /// A string START's text, or the current buffer's from START to END, as
    /// internal bytes; END before START is out of range unless SWAP
    /// (validate_region's order).
    pub(crate) fn text_span(&mut self, start: Value, end: Value, swap: bool) -> std::result::Result<TextSpan, Fault> {
        if let Some(view) = self.vm.str_view(start) {
            return Ok(TextSpan { bytes: view.bytes().to_vec(), chars: view.chars, multibyte: view.multibyte });
        }
        let (s, e) = (self.vm.text_property_position(start)?, self.vm.text_property_position(end)?);
        let (s, e) = if swap { (s.min(e), s.max(e)) } else { (s, e) };
        // validate_region's error names the buffer; the coding search's
        // args_out_of_range only the positions.
        let buffer = if swap { Some(self.current_buffer()?) } else { None };
        let out_of_range = || buffer.map_or(Fault::Range(start, end), |b| Fault::Range3(b, start, end));
        if s > e { return Err(out_of_range()); }
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let Object::Buffer(b) = &self.vm.objects[index] else {
            let bytes = match &self.vm.objects[index] { Object::ByteBuffer(b) => b.bytes.as_slice(), _ => &[] };
            if !(1 <= s && e <= bytes.len() as i64 + 1) { return Err(out_of_range()); }
            let bytes = bytes[s as usize - 1..e as usize - 1].to_vec();
            return Ok(TextSpan { chars: bytes.len(), bytes, multibyte: false });
        };
        if !(1 <= s && e <= b.len_chars() as i64 + 1) { return Err(out_of_range()); }
        let byte = |c: i64| b.byte_of_char(CharOffset(c as usize - 1)).map_err(Fault::Error);
        let (from, to) = (byte(s)?, byte(e)?);
        let text: &crate::text::Text = b;
        Ok(TextSpan { bytes: text.internal(from..to).map_err(Fault::Error)?.to_vec(), chars: (e - s) as usize, multibyte: true })
    }
    /// One side of compare-buffer-substrings (editfns.c:1752): BUFFER (nil,
    /// a buffer or its name) from START to END, each nil for that end of its
    /// accessible text and in either order, as character codes; a unibyte
    /// buffer's bytes above 127 are raw-byte characters (make_char_multibyte).
    pub(crate) fn buffer_substring_codes(&mut self, buffer: Value, start: Value, end: Value) -> std::result::Result<Vec<u32>, Fault> {
        let buffer = if buffer == Value::NIL { self.current_buffer()? } else {
            let found = self.lifecycle_get(buffer)?;
            if found == Value::NIL {
                return Err(Fault::Error(match self.vm.str_view(buffer) {
                    Some(_) => format!("No buffer named {}", self.string_text(buffer)?),
                    None => "Invalid buffer argument".into(),
                }));
            }
            if !self.lifecycle_live_p(found) { return Err(Fault::Error("Selecting deleted buffer".into())); }
            found
        };
        let position = |leaf: &Self, v: Value, default: i64| if v == Value::NIL { Ok(default) } else { leaf.fix_position(v) };
        match self.vm.object_if(buffer) {
            Some(Object::Buffer(b)) => {
                let r = b.accessible();
                let char_pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                let (begv, zv) = (char_pos(r.start)?, char_pos(r.end)?);
                let (s, e) = (position(self, start, begv)?, position(self, end, zv)?);
                let (s, e) = (s.min(e), s.max(e));
                if !(begv <= s && e <= zv) { return Err(Fault::Range(start, end)); }
                let byte = |c: i64| b.byte_of_char(CharOffset(c as usize - 1)).map_err(Fault::Error);
                let text: &crate::text::Text = b;
                let bytes = text.internal(byte(s)?..byte(e)?).map_err(Fault::Error)?;
                Ok(crate::lstring::StrView { bytes: &bytes, chars: (e - s) as usize, multibyte: true }.chars().collect())
            }
            Some(Object::ByteBuffer(b)) => {
                let (begv, zv) = (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1);
                let (s, e) = (position(self, start, begv)?, position(self, end, zv)?);
                let (s, e) = (s.min(e), s.max(e));
                if !(begv <= s && e <= zv) { return Err(Fault::Range(start, end)); }
                Ok(b.bytes[s as usize - 1..e as usize - 1].iter()
                    .map(|&c| if c < 0x80 { u32::from(c) } else { u32::from(c) + crate::lstring::BYTE8_OFFSET }).collect())
            }
            _ => unreachable!("a live buffer"),
        }
    }
    /// Ffind_coding_systems_region_internal: t when the text needs no
    /// encoding (unibyte or ASCII), else the base coding systems of
    /// `coding-system-list', less EXCLUDE, that encode every character,
    /// then `raw-text' and `no-conversion'.
    pub(crate) fn find_coding_systems_region(&mut self, start: Value, end: Value, exclude: Value) -> super::super::Out {
        let text = self.text_span(start, end, false)?;
        if !text.multibyte || text.ascii() { return Ok(Value::TRUE); }
        let chars: Vec<u32> = text.view().chars().collect();
        let list = self.intern("coding-system-list");
        let names = self.variable_value(list)?.unwrap_or(Value::NIL);
        let excluded = if exclude == Value::NIL { Vec::new() } else { self.coding_items(exclude)? };
        let mut candidates = Vec::new();
        for name in self.coding_items(names)? {
            if excluded.contains(&name) { continue; }
            let spec = self.coding_spec(name)?;
            if spec == Value::NIL { continue; }
            let attrs = self.vector_get(spec, Value::integer(0).expect("index"))?;
            if self.vector_get(attrs, Value::integer(0).expect("index"))? != name { continue; }
            let charsets = self.vector_get(attrs, Value::integer(4).expect("index"))?;
            let charsets = self.coding_items(charsets)?.iter().filter_map(|id| id.integer_value()).map(|id| id as usize).collect::<Vec<_>>();
            let tables = self.coding_encode_translation(attrs)?;
            candidates.push((name, charsets, tables));
        }
        let first = chars.iter().position(|&c| c >= 0x80).unwrap_or(chars.len());
        let last = chars.iter().rposition(|&c| c >= 0x80).map_or(first, |i| i + 1);
        let mut seen = std::collections::HashSet::new();
        for &c in &chars[first..last] {
            if c < 0x80 || !seen.insert(c) { continue; }
            let mut kept = Vec::with_capacity(candidates.len());
            for candidate in std::mem::take(&mut candidates) {
                if self.coding_encodable(&candidate.1, &candidate.2, c)? { kept.push(candidate); }
            }
            candidates = kept;
        }
        let mut result = vec![];
        result.extend(candidates.iter().map(|(name, _, _)| *name));
        result.push(self.intern("raw-text"));
        result.push(self.intern("no-conversion"));
        Ok(self.coding_list(&result))
    }
    /// Ffind_operation_coding_system up to calling a function entry: the
    /// answer, or Err with the function to call with the argument list.
    pub(crate) fn find_operation_coding(&mut self, args: &[Value]) -> std::result::Result<std::result::Result<Value, Value>, Fault> {
        self.coding_init()?;
        if args.len() < 2 { return Err(Fault::Error("Too few arguments".into())); }
        let operation = args[0];
        let target_idx = self.intern("target-idx");
        let index = if self.kind(operation) == Kind::Symbol {
            self.vm.symbol_property(operation, target_idx).map_err(Fault::Error)?.integer_value().filter(|&n| n >= 0)
        } else { None };
        let Some(index) = index.map(|n| n as usize) else { return Err(Fault::Error("Invalid first argument".into())) };
        let name = self.symbol_name(operation)?;
        if args.len() <= 1 + index {
            return Err(Fault::Error(format!("Too few arguments for operation ‘{name}’")));
        }
        let (insert, network) = (self.intern("insert-file-contents"), self.intern("open-network-stream"));
        let mut target = args[index + 1];
        let string = |leaf: &Self, v: Value| leaf.vm.str_view(v).is_some();
        let valid = string(self, target)
            || (operation == insert && self.pair(target).is_some_and(|(file, buffer)| string(self, file) && self.kind(buffer) == Kind::Buffer))
            || (operation == network && (target.integer_value().is_some() || target == Value::TRUE));
        if !valid { return Err(Fault::Error(format!("Invalid argument {} of operation ‘{name}’", index + 1))); }
        if let Some((file, _)) = self.pair(target) { target = file; }
        let alist = if operation == insert || operation == self.intern("write-region") { "file-coding-system-alist" }
            else if operation == network { "network-coding-system-alist" } else { "process-coding-system-alist" };
        let alist = self.intern(alist);
        let mut chain = self.variable_value(alist)?.unwrap_or(Value::NIL);
        while let Some((element, rest)) = self.pair(chain) {
            chain = rest;
            let Some((key, val)) = self.pair(element) else { continue };
            // fast_string_match: no match data, no case folding.
            let hit = if string(self, target) {
                string(self, key) && self.vm.string_match_no_data(key, target, crate::regexp::Translation::None, None)?.is_some()
            } else {
                target.integer_value().is_some() && key == target
            };
            if !hit { continue; }
            if self.pair(val).is_some() { return Ok(Ok(val)); }
            if self.kind(val) != Kind::Symbol { return Ok(Ok(Value::NIL)); }
            if self.coding_p(val)? != Value::NIL { return Ok(Ok(self.cons(val, val))); }
            if val != Value::NIL && self.fboundp(val) { return Ok(Err(val)); }
            return Ok(Ok(Value::NIL));
        }
        Ok(Ok(Value::NIL))
    }
    /// What a file-coding-system-alist function's VAL means: a pair, a
    /// coding system as (VAL . VAL), else nil.
    pub(crate) fn operation_coding_pair(&mut self, val: Value) -> super::super::Out {
        if self.pair(val).is_some() { return Ok(val); }
        if self.kind(val) == Kind::Symbol && self.coding_p(val)? != Value::NIL { return Ok(self.cons(val, val)); }
        Ok(Value::NIL)
    }
}
