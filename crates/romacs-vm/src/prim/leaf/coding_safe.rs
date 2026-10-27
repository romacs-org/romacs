//! coding.c's safe-coding search: which coding systems can encode a text,
//! as `select-safe-coding-system' asks before writing or hashing.
use super::*;
use crate::coordinates::CharOffset;
use crate::data::Object;

impl Leaf<'_> {
    /// get_translation_table (ATTRS, true): the coding system's encode
    /// table, with symbols read through their `translation-table'
    /// property, then the standard one; nil when translation is off.
    fn coding_encode_translation(&mut self, attrs: Value) -> std::result::Result<Vec<Value>, Fault> {
        let enabled = self.intern("enable-character-translation");
        if self.variable_value(enabled)?.unwrap_or(Value::NIL) == Value::NIL { return Ok(Vec::new()); }
        let standard = self.intern("standard-translation-table-for-encode");
        let standard = self.variable_value(standard)?.unwrap_or(Value::NIL);
        let own = self.vector_get(attrs, Value::integer(7).expect("index"))?;
        let property = self.intern("translation-table");
        let mut tables = Vec::new();
        let mut table_of = |leaf: &mut Self, v: Value| -> std::result::Result<Value, Fault> {
            if leaf.kind(v) == Kind::Symbol && v != Value::NIL { leaf.vm.symbol_property(v, property).map_err(Fault::Error) } else { Ok(v) }
        };
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
        Ok(tables.into_iter().filter(|&t| self.kind(t) == Kind::CharTable).collect())
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
    /// Ffind_coding_systems_region_internal: t when the text needs no
    /// encoding (unibyte or ASCII), else the base coding systems of
    /// `coding-system-list', less EXCLUDE, that encode every character,
    /// then `raw-text' and `no-conversion'.
    pub(crate) fn find_coding_systems_region(&mut self, start: Value, end: Value, exclude: Value) -> super::super::Out {
        let chars: Vec<u32> = if let Some(view) = self.vm.str_view(start) {
            if !view.multibyte || view.bytes().is_ascii() { return Ok(Value::TRUE); }
            view.chars().collect()
        } else {
            let s = self.vm.text_property_position(start)?;
            let e = self.vm.text_property_position(end)?;
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            let Object::Buffer(b) = &self.vm.objects[index] else {
                let z = match &self.vm.objects[index] { Object::ByteBuffer(b) => b.bytes.len() as i64 + 1, _ => 1 };
                if !(1 <= s && s <= e && e <= z) { return Err(Fault::Range(start, end)); }
                return Ok(Value::TRUE);
            };
            let z = b.len_chars() as i64 + 1;
            if !(1 <= s && s <= e && e <= z) { return Err(Fault::Range(start, end)); }
            let byte = |c: i64| b.byte_of_char(CharOffset(c as usize - 1)).map_err(Fault::Error);
            let (from, to) = (byte(s)?, byte(e)?);
            if (e - s) as usize == to.0 - from.0 { return Ok(Value::TRUE); }
            let text: &crate::text::Text = b;
            text.as_str()[from.0..to.0].chars().map(u32::from).collect()
        };
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
                string(self, key) && self.vm.string_match_no_data(key, target, None)?.is_some()
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
