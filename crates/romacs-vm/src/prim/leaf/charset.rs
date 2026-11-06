use super::*;
include!("charset_define.rs");

/// One FUNCTION call of `map-charset-chars': its character range and the
/// walk whose cons it shares, if any.
pub(crate) struct Run { pub start: i64, pub end: i64, pub walk: Option<usize> }

impl Leaf<'_> {
    fn charset_record(&mut self, name: Value) -> std::result::Result<usize, Fault> {
        self.vm.ensure_charsets();
        if self.kind(name) != Kind::Symbol {
            return Err(Fault::Wrong("charsetp", name));
        }
        self.vm
            .charsets
            .lookup(name)
            .ok_or(Fault::Wrong("charsetp", name))
    }
    pub fn is_charset(&mut self, object: Value) -> bool {
        self.charset_record(object).is_ok()
    }
    pub fn charset_plist(&mut self, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        Ok(self.vm.charsets.records[i].attrs[2])
    }
    pub fn set_charset_plist(&mut self, name: Value, plist: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        self.vm.charsets.records[i].attrs[2] = plist;
        Ok(plist)
    }
    /// charset.c Funify_charset. De-unifying a unified charset needs its
    /// decoder (DECODE_CHAR) and is refused until decoders exist.
    pub fn unify_charset(&mut self, name: Value, unify_map: Value, deunify: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        let id = self.vm.charsets.records[i].attrs[0].integer_value().unwrap_or(0) as usize;
        let record = &self.vm.charsets.records[i];
        let unified = record.unified;
        if if deunify == Value::NIL { unified && record.attrs[9] != Value::NIL } else { !unified } {
            return Ok(Value::NIL);
        }
        if deunify != Value::NIL {
            return Err(Fault::Error("VM2 unify-charset DEUNIFY of a unified charset is unsupported".into()));
        }
        self.vm.charsets.records[i].unified = false;
        let record = &self.vm.charsets.records[i];
        if record.offset.is_none_or(|offset| offset < 0x110000) {
            let symbol = self.symbol_name(name)?;
            return Err(Fault::Error(format!("Can\u{2019}t unify charset: {symbol}")));
        }
        if unify_map != Value::NIL {
            if !matches!(self.kind(unify_map), Kind::String | Kind::Bytes | Kind::Vector) {
                let message = self.string("Bad unify-map");
                let tail = self.cons(unify_map, Value::NIL);
                let data = self.cons(message, tail);
                return Err(Fault::Signal(self.intern("error"), data));
            }
            self.vm.charsets.records[i].attrs[8] = unify_map;
        }
        let (min, max) = (self.vm.charsets.records[i].min_char, self.vm.charsets.records[i].max_char);
        let ranges = &mut self.vm.charsets.unify_ranges;
        ranges.retain(|&(_, _, owner)| owner != id);
        ranges.push((min, max, id));
        self.vm.charsets.records[i].unified = true;
        self.vm.charsets.records[i].tables = None;
        Ok(Value::NIL)
    }
    /// charset.c Fdecode_char.
    pub fn decode_char(&mut self, name: Value, code: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        let code = self.charset_unsigned(code)?;
        Ok(self.charset_decode(i, code)?.map_or(Value::NIL, |c| Value::integer(c.into()).unwrap()))
    }
    /// charset.c Fencode_char.
    pub fn encode_char(&mut self, ch: Value, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        let c = ch.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c)).ok_or(Fault::Wrong("characterp", ch))?;
        Ok(self.charset_encode(i, c as u32)?.map_or(Value::NIL, |code| Value::integer(code.into()).unwrap()))
    }
    pub(super) fn named_charset_id(&mut self, name: &str) -> std::result::Result<usize, Fault> {
        let symbol = self.intern(name);
        let i = self.charset_record(symbol)?;
        Ok(self.vm.charsets.records[i].attrs[0].integer_value().unwrap() as usize)
    }
    /// charset.h CHAR_CHARSET with charset.c char_charset: the first charset
    /// in priority order that encodes C; for a Unicode character past the
    /// prioritized head, `unicode'; else `emacs' or `eight-bit'.
    pub fn char_charset_id(&mut self, c: u32) -> std::result::Result<usize, Fault> {
        if c < 0x80 { return self.named_charset_id("ascii"); }
        self.vm.ensure_charsets();
        let ordered = self.vm.charsets.ordered.clone();
        let head = self.vm.charsets.non_preferred_head;
        for (at, &id) in ordered.iter().enumerate() {
            let i = self.vm.charsets.ids[id];
            if self.charset_encode(i, c)?.is_some() { return Ok(id); }
            if c <= 0x10ffff && ordered.get(at + 1).copied() == head { return self.named_charset_id("unicode"); }
        }
        self.named_charset_id(if c <= crate::lstring::MAX_5_BYTE_CHAR { "emacs" } else { "eight-bit" })
    }
    /// charset.c Fsplit_char: CH's charset by priority (CHAR_CHARSET), then
    /// ENCODE_CHAR's code as one byte per dimension, most significant first.
    pub fn split_char(&mut self, ch: Value) -> super::super::Out {
        let c = ch.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c))
            .ok_or(Fault::Wrong("characterp", ch))? as u32;
        let id = self.char_charset_id(c)?;
        let i = self.vm.charsets.ids[id];
        let mut code = self.charset_encode(i, c)?.ok_or_else(|| Fault::Error("split-char: CH has no code in its charset".into()))?;
        let mut list = Value::NIL;
        for _ in 0..self.vm.charsets.records[i].dimension {
            list = self.cons(Value::integer(i64::from(code & 0xff)).unwrap(), list);
            code >>= 8;
        }
        let name = self.charset_id_name(id);
        Ok(self.cons(name, list))
    }
    fn charset_id_name(&self, id: usize) -> Value {
        self.vm.charsets.records[self.vm.charsets.ids[id]].attrs[1]
    }
    /// charset.c Fchar_charset.
    pub fn char_charset(&mut self, ch: Value, restriction: Value) -> super::super::Out {
        let c = ch.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c))
            .ok_or(Fault::Wrong("characterp", ch))? as u32;
        if restriction == Value::NIL {
            let id = self.char_charset_id(c)?;
            return Ok(self.charset_id_name(id));
        }
        if self.pair(restriction).is_none() {
            return Err(Fault::Error("VM2 char-charset with a coding-system RESTRICTION is not implemented".into()));
        }
        let mut tail = restriction;
        while let Some((name, rest)) = self.pair(tail) {
            let i = self.charset_record(name)?;
            if self.charset_encode(i, c)?.is_some() { return Ok(name); }
            tail = rest;
        }
        Ok(Value::NIL)
    }
    /// charset.c find_charsets_in_text over a string START or the region
    /// START..END: the names of the charsets of its characters after
    /// TABLE's translation, in charset id order.  Multibyte ASCII text is
    /// `ascii' alone; unibyte text is `ascii' and `eight-bit'.
    pub fn find_charsets(&mut self, start: Value, end: Value, table: Value) -> super::super::Out {
        let text = self.text_span(start, end, true)?;
        let mut tables = Vec::new();
        self.translation_char_tables(table, &mut HashSet::default(), &mut tables)?;
        let mut found = std::collections::BTreeSet::new();
        if text.multibyte && text.ascii() {
            found.insert(self.named_charset_id("ascii")?);
        } else {
            let chars: Vec<u32> = text.view().chars().collect();
            for c in chars {
                let mut c = c;
                for &table in &tables {
                    let to = self.vm.char_table_get(table, c as usize).map_err(Fault::Error)?;
                    if let Some(to) = to.integer_value().filter(|to| (0..=i64::from(crate::lstring::MAX_CHAR)).contains(to)) { c = to as u32; }
                }
                let id = if text.multibyte { self.char_charset_id(c)? } else { self.named_charset_id(if c < 0x80 { "ascii" } else { "eight-bit" })? };
                found.insert(id);
            }
        }
        let names: Vec<Value> = found.into_iter().map(|id| self.charset_id_name(id)).collect();
        Ok(self.coding_list(&names))
    }
    /// charset.c Fmap_charset_chars's argument handling: the clamped code
    /// range, then map_charset_chars's calls in order.
    pub fn charset_map_calls(&mut self, name: Value, from: Value, to: Value) -> std::result::Result<Vec<Run>, Fault> {
        let i = self.charset_record(name)?;
        let (min, max) = (self.vm.charsets.records[i].min, self.vm.charsets.records[i].max);
        let code = |v: Value, default: u32| -> std::result::Result<u32, Fault> {
            if v == Value::NIL { return Ok(default); }
            v.integer_value().filter(|&n| (0..=i64::from(u32::MAX)).contains(&n)).map(|n| n as u32).ok_or(Fault::Wrong("wholenump", v))
        };
        let (from, to) = (code(from, min)?.max(min), code(to, max)?.min(max));
        let mut calls = Vec::new();
        self.charset_map_into(i, from, to, &mut calls)?;
        Ok(calls)
    }
    /// load_charset: the record's tables, read from its map (MAP) or
    /// unify map (OFFSET) once and kept until unify-charset changes it.
    fn charset_tables(&mut self, i: usize) -> std::result::Result<std::rc::Rc<crate::charset::Tables>, Fault> {
        if let Some(tables) = &self.vm.charsets.records[i].tables { return Ok(tables.clone()); }
        let record = &self.vm.charsets.records[i];
        let (map, min, max) = (if record.offset.is_some() { record.attrs[8] } else { record.attrs[3] }, record.min, record.max);
        let entries = if map == Value::NIL { Vec::new() } else { self.charset_map_entries(map, min, max)? };
        let tables = std::rc::Rc::new(self.vm.charsets.records[i].tables(&entries));
        self.vm.charsets.records[i].tables = Some(tables.clone());
        Ok(tables)
    }
    fn subset_info(&self, info: Value) -> std::result::Result<(usize, u32, u32, u32), Fault> {
        let info = self.vector_values(info)?;
        let [parent, low, high, offset] = info[..] else { unreachable!("subset info has four slots") };
        let field = |v: Value| v.integer_value().unwrap() as u32;
        Ok((self.charset_by_id(parent)?, field(low), field(high), field(offset)))
    }
    /// The (charset id . code offset) parents of a SUPERSET charset.
    fn superset_parents(&self, mut parents: Value) -> std::result::Result<Vec<(usize, u32)>, Fault> {
        let mut out = Vec::new();
        while let Some((head, rest)) = self.pair(parents) {
            let (id, offset) = self.pair(head).expect("superset parent is a cons");
            out.push((self.charset_by_id(id)?, offset.integer_value().unwrap() as u32));
            parents = rest;
        }
        Ok(out)
    }
    /// charset.h DECODE_CHAR with charset.c decode_char.
    pub fn charset_decode(&mut self, i: usize, code: u32) -> std::result::Result<Option<u32>, Fault> {
        let record = &self.vm.charsets.records[i];
        if code < 128 && record.ascii_compatible { return Ok(Some(code)); }
        if code < record.min || code > record.max { return Ok(None); }
        if record.attrs[6] != Value::NIL && record.offset.is_none() {
            let (parent, low, high, offset) = self.subset_info(record.attrs[6])?;
            let code = code.wrapping_sub(offset);
            return if code < low || code > high { Ok(None) } else { self.charset_decode(parent, code) };
        }
        if record.offset.is_none() && record.attrs[3] == Value::NIL {
            for (parent, offset) in self.superset_parents(record.attrs[7])? {
                if let Some(c) = self.charset_decode(parent, code.wrapping_sub(offset))? { return Ok(Some(c)); }
            }
            return Ok(None);
        }
        let index = record.code_index(code);
        if index < 0 { return Ok(None); }
        let index = index as u32;
        let Some(offset) = record.offset else {
            return Ok(self.charset_tables(i)?.decoder.get(&index).copied());
        };
        let c = index + offset;
        // Vchar_unify_table maps the upper code space to Unicode.
        if record.unified && (0x110000..=0x3FFF7F).contains(&c) {
            return Ok(Some(self.charset_tables(i)?.decoder.get(&index).copied().unwrap_or(c)));
        }
        Ok(Some(c))
    }
    /// charset.h ENCODE_CHAR with charset.c encode_char; None is the
    /// invalid code.
    pub fn charset_encode(&mut self, i: usize, c: u32) -> std::result::Result<Option<u32>, Fault> {
        let record = &self.vm.charsets.records[i];
        if c < 128 && record.ascii_compatible { return Ok(Some(c)); }
        let invalid = record.invalid;
        let mut c = c;
        if record.unified {
            let offset = record.offset.expect("only OFFSET charsets unify");
            if let Some(&index) = self.charset_tables(i)?.encoder.get(&c) { c = offset + index; }
        }
        let record = &self.vm.charsets.records[i];
        let code = if record.attrs[6] != Value::NIL && record.offset.is_none() {
            let (parent, low, high, offset) = self.subset_info(record.attrs[6])?;
            self.charset_encode(parent, c)?.filter(|&code| low <= code && code <= high).map(|code| code.wrapping_add(offset))
        } else if record.offset.is_none() && record.attrs[3] == Value::NIL {
            let mut found = None;
            for (parent, offset) in self.superset_parents(record.attrs[7])? {
                if let Some(code) = self.charset_encode(parent, c)? { found = Some(code.wrapping_add(offset)); break; }
            }
            found
        } else if !record.fast_map_has(c) || i64::from(c) < record.min_char || i64::from(c) > record.max_char {
            None
        } else if let Some(offset) = record.offset {
            Some(record.code_point(c - offset))
        } else {
            let tables = self.charset_tables(i)?;
            tables.encoder.get(&c).map(|&index| self.vm.charsets.records[i].code_point(index))
        };
        Ok(code.filter(|&code| code != invalid))
    }
    /// charset.c map_charset_chars. Each encoder or deunifier walk passes one
    /// reused cons (WALK names it); an OFFSET range is a fresh cons.
    fn charset_map_into(&mut self, i: usize, from: u32, to: u32, calls: &mut Vec<Run>) -> std::result::Result<(), Fault> {
        let record = &self.vm.charsets.records[i];
        let partial = from > record.min || to < record.max;
        let walk = |leaf: &mut Self, calls: &mut Vec<Run>| -> std::result::Result<(), Fault> {
            let tables = leaf.charset_tables(i)?;
            let mut codes = std::collections::HashMap::new();
            if partial { for &c in tables.encoder.keys() { codes.insert(c, leaf.charset_encode(i, c)?); } }
            let id = calls.len();
            let runs = crate::charset::char_runs(tables.encoder.keys().copied(), partial, from, to, |c| codes[&c]);
            calls.extend(runs.into_iter().map(|(start, end)| Run { start: start.into(), end: end.into(), walk: Some(id) }));
            Ok(())
        };
        if let Some(offset) = record.offset {
            let unified = record.unified;
            let (start, end) = (record.code_index(from) as i64 + i64::from(offset), record.code_index(to) as i64 + i64::from(offset));
            if unified { walk(self, calls)?; }
            calls.push(Run { start, end, walk: None });
        } else if record.attrs[3] != Value::NIL {
            walk(self, calls)?;
        } else if record.attrs[6] != Value::NIL {
            let (parent, low, high, offset) = self.subset_info(record.attrs[6])?;
            self.charset_map_into(parent, from.wrapping_sub(offset).max(low), to.wrapping_sub(offset).min(high), calls)?;
        } else {
            for (parent, offset) in self.superset_parents(record.attrs[7])? {
                let (min, max) = (self.vm.charsets.records[parent].min, self.vm.charsets.records[parent].max);
                let this_from = if from > offset { from - offset } else { 0 }.max(min);
                let this_to = if to > offset { to - offset } else { 0 }.min(max);
                self.charset_map_into(parent, this_from, this_to, calls)?;
            }
        }
        Ok(())
    }
    fn charset_by_id(&self, id: Value) -> std::result::Result<usize, Fault> {
        let id = id.integer_value().unwrap() as usize;
        self.vm.charsets.ids.get(id).copied().ok_or(Fault::Error(format!("Invalid charset id: {id}")))
    }
    /// charset.c Fset_charset_priority: move CHARSETS, in order, to the
    /// front of Vcharset_ordered_list and refilter the ISO/emacs-mule lists.
    pub fn set_charset_priority(&mut self, charsets: &[Value]) -> super::super::Out {
        // Fset_charset_priority works on a copy: a bad argument changes nothing.
        self.vm.ensure_charsets();
        let mut rest = self.vm.charsets.ordered.clone();
        let mut head = Vec::new();
        for &charset in charsets {
            let i = self.charset_record(charset)?;
            let id = self.vm.charsets.records[i].attrs[0].integer_value().unwrap_or(0) as usize;
            if let Some(at) = rest.iter().position(|&o| o == id) {
                rest.remove(at);
                head.push(id);
            }
        }
        let registry = &mut self.vm.charsets;
        registry.non_preferred_head = rest.first().copied();
        registry.ordered = head.into_iter().chain(rest).collect();
        registry.ordered_tick += 1;
        let ordered = registry.ordered.clone();
        registry.iso_list = ordered.iter().copied().filter(|id| registry.iso_list.contains(id)).collect();
        registry.mule_list = ordered.iter().copied().filter(|id| registry.mule_list.contains(id)).collect();
        Ok(Value::NIL)
    }
    /// charset.c CHAR_CHARSET_P for the charset ID.
    pub fn char_in_charset_id(&mut self, id: usize, c: u32) -> std::result::Result<bool, Fault> {
        self.vm.ensure_charsets();
        let i = self.vm.charsets.ids[id];
        Ok(self.charset_encode(i, c)?.is_some())
    }
    /// CHARSET_ASCII_COMPATIBLE_P.
    pub fn charset_ascii_compatible(&mut self, charset: Value) -> std::result::Result<bool, Fault> {
        let i = self.charset_record(charset)?;
        Ok(self.vm.charsets.records[i].ascii_compatible)
    }
    /// charset.c Fcharset_id_internal.
    pub fn charset_id_internal(&mut self, charset: Value) -> super::super::Out {
        let i = self.charset_record(charset)?;
        Ok(self.vm.charsets.records[i].attrs[0])
    }
    /// charset.c Fsort_charsets: CHARSETS reordered in place by priority.
    pub fn sort_charsets(&mut self, charsets: Value) -> super::super::Out {
        let mut cells = Vec::new();
        let mut tail = charsets;
        while let Some((name, rest)) = self.pair(tail) {
            cells.push((tail, name));
            tail = rest;
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
        if cells.is_empty() { return Ok(Value::NIL); }
        let rank: std::collections::HashMap<usize, usize> =
            { self.vm.ensure_charsets(); self.vm.charsets.ordered.iter().enumerate().map(|(rank, &id)| (id, rank)).collect() };
        let mut sorted = Vec::with_capacity(cells.len());
        for &(_, name) in &cells {
            let i = self.charset_record(name)?;
            let id = self.vm.charsets.records[i].attrs[0].integer_value().unwrap_or(0) as usize;
            sorted.push((rank.get(&id).copied().unwrap_or(usize::MAX), name));
        }
        sorted.sort_by_key(|&(rank, _)| rank);
        for (&(cell, _), &(_, name)) in cells.iter().zip(&sorted) { self.setcar(cell, name)?; }
        Ok(charsets)
    }
    /// charset.c Fcharset_priority_list.
    pub fn charset_priority_list(&mut self, highestp: Value) -> super::super::Out {
        self.vm.ensure_charsets();
        let names: Vec<Value> = self.vm.charsets.ordered.iter()
            .map(|&id| self.vm.charsets.records[self.vm.charsets.ids[id]].attrs[1]).collect();
        if highestp != Value::NIL { return Ok(names.first().copied().unwrap_or(Value::NIL)); }
        Ok(names.into_iter().rev().fold(Value::NIL, |tail, name| self.cons(name, tail)))
    }
    pub fn charset_alias(&mut self, alias: Value, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        if let Some(binding) = self.vm.charsets.bindings.iter_mut().find(|(key, _)| *key == alias) {
            binding.1 = i;
        } else {
            self.vm.charsets.bindings.push((alias, i));
        }
        let slot = self.vm.slot("charset-list");
        self.vm.slots[slot].value = self.cons(alias, self.vm.slots[slot].value);
        Ok(Value::NIL)
    }
}

impl Leaf<'_> {
    /// charset.c Fmake_char: CODE1 is the most significant position code;
    /// a missing one is its dimension's minimum.
    pub fn make_char(&mut self, name: Value, codes: [Value; 4]) -> super::super::Out {
        let i = self.charset_record(name)?;
        let record = &self.vm.charsets.records[i];
        let (dimension, space, iso, ascii) = (record.dimension as usize, record.space, record.iso_final >= 0, record.ascii_compatible);
        let position = |cx: &mut Self, v: Value| -> std::result::Result<u32, Fault> {
            let n = v.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("wholenump", v))?;
            if n >= 0x100 {
                let data = cx.cons(v, Value::NIL);
                let data = cx.cons(Value::integer(0xFF).unwrap(), data);
                let condition = cx.intern("args-out-of-range");
                return Err(Fault::Signal(condition, data));
            }
            Ok(n as u32)
        };
        let mut code = if codes[0] == Value::NIL {
            if ascii { 0 } else { self.vm.charsets.records[i].min }
        } else {
            let mut code = position(self, codes[0])?;
            for k in 1..dimension.min(4) {
                code <<= 8;
                code |= if codes[k] == Value::NIL { space[(dimension - 1 - k) * 2] } else { position(self, codes[k])? };
            }
            code
        };
        if iso { code &= 0x7F7F_7F7F; }
        let c = self.charset_decode(i, code)?.ok_or_else(|| Fault::Error("Invalid code(s)".into()))?;
        Ok(Value::integer(c.into()).unwrap())
    }
}
