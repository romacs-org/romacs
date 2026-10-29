//! Ordinary data calls. No allocation here is a collector safepoint: transient
//! Values live only inside this synchronous call; returned graphs use the
//! existing traced cons/object arenas and the caller's register roots.
use super::*;

impl Vm {
    pub(super) fn eval_data_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        match p {
            DeepEqual => {
                if n != 2 {
                    return Err("wrong primitive arity".into());
                }
                Ok(
                    if self.eval_data_equal(self.stack[start], self.stack[start + 1])? {
                        Value::TRUE
                    } else {
                        Value::NIL
                    },
                )
            }
            Min => {
                if n == 0 {
                    return Err("wrong primitive arity".into());
                }
                let args = self.stack[start..start + n].to_vec();
                self.minmax(&args, std::cmp::Ordering::Less)
            }
            Append => {
                if n == 0 {
                    return Ok(Value::NIL);
                }
                let mut prefix = Vec::new();
                for i in 0..n - 1 {
                    let v = self.stack[start + i];
                    if let Some(text) = self.str_view(v) {
                        prefix.extend(
                            text.chars()
                                .map(|c| Value::integer(c as i64))
                                .collect::<Result<Vec<_>>>()?,
                        );
                    } else {
                        match self.object_if(v) {
                            Some(data::Object::Vector(values)) => prefix.extend_from_slice(values),
                            Some(data::Object::BoolVector(bits)) => prefix.extend(
                                (0..bits.len).map(|i| if bits.get(i) == Some(true) { Value::TRUE } else { Value::NIL }),
                            ),
                            _ => prefix.extend(self.list_values(v)?),
                        }
                    }
                }
                let mut tail = self.stack[start + n - 1];
                for value in prefix.into_iter().rev() {
                    self.conses.push((value, tail));
                    tail = Value::reference(self.conses.len() - 1, 3);
                }
                Ok(tail)
            }
            Prin1ToString => {
                if !(1..=3).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let quoted = n == 1 || self.stack[start + 1] == Value::NIL;
                let overrides = if n == 3 { self.stack[start + 2] } else { Value::NIL };
                let text = self.print_render(self.stack[start], quoted, overrides, true)?;
                if text.contains(crate::symkey::ESC) { return Ok(self.printed_value(&text)); }
                self.literal(&Form::String(text))
            }
            _ => unreachable!("eval data dispatch"),
        }
    }

    fn eval_data_equal(&self, a: Value, b: Value) -> Result<bool> {
        // Iterative worklist also handles cyclic conses/vectors without keeping
        // a Rust recursive borrow or imposing a limit on long flat lists.
        let mut pending = vec![(a, b)];
        let mut seen = HashSet::default();
        while let Some((a, b)) = pending.pop() {
            if a == b || !seen.insert((a.0, b.0)) {
                continue;
            }
            if let (Some(x), Some(y)) = (self.str_view(a), self.str_view(b)) {
                if x.len() != y.len() || x.bytes() != y.bytes() { return Ok(false); }
                continue;
            }
            if a.tag() == 3 && b.tag() == 3 {
                let (ac, ad) = self.cons_pair(a)?;
                let (bc, bd) = self.cons_pair(b)?;
                pending.push((ad, bd));
                pending.push((ac, bc));
                continue;
            }
            match (self.object_if(a), self.object_if(b)) {
                (Some(data::Object::Bignum(x)), Some(data::Object::Bignum(y))) if x == y => {}
                (Some(data::Object::Float(x)), Some(data::Object::Float(y))) if x.to_bits() == y.to_bits() => {}
                (Some(data::Object::Bytes(x)), Some(data::Object::Bytes(y))) if x == y => {}
                (Some(data::Object::BoolVector(x)), Some(data::Object::BoolVector(y))) if x == y => {}
                (Some(data::Object::Vector(x)), Some(data::Object::Vector(y)))
                | (Some(data::Object::Record(x)), Some(data::Object::Record(y)))
                    if x.len() == y.len() =>
                {
                    pending.extend(x.iter().copied().zip(y.iter().copied()))
                }
                // ASCII unibyte and multibyte strings compare equal in GNU.
                (Some(data::Object::Bytes(x)), _)
                    if self
                        .source_string_if(b)
                        .is_some_and(|y| x.is_ascii() && x == y.as_bytes()) => {}
                (_, Some(data::Object::Bytes(y)))
                    if self
                        .source_string_if(a)
                        .is_some_and(|x| y.is_ascii() && x.as_bytes() == y) => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    pub(super) fn print_render(&mut self, value: Value, quoted: bool, overrides: Value, buffer_output: bool) -> Result<String> {
        for name in ["float-output-format", "print-integers-as-characters", "print-unreadable-function", "print-charset-text-property", "print-symbols-bare"] {
            let slot = self.slot(name);
            let v = self.variable_value(slot)?;
            // Text properties print unpruned, which is GNU's `t'.
            let supported = v == Value::UNBOUND || v == Value::NIL || (name == "print-charset-text-property" && v == Value::TRUE);
            if !supported { return Err(format!("printing {name} is not supported")); }
        }
        let mut options = PrintOptions::default();
        for (key, name) in [("length", "print-length"), ("level", "print-level"), ("quoted", "print-quoted"),
            ("escape-newlines", "print-escape-newlines"), ("escape-nonascii", "print-escape-nonascii"),
            ("escape-multibyte", "print-escape-multibyte"), ("escape-control-characters", "print-escape-control-characters"),
            ("circle", "print-circle"), ("gensym", "print-gensym")] {
            let slot = self.slot(name);
            let v = self.variable_value(slot)?;
            if v != Value::UNBOUND { options.set(key, v); }
        }
        let continuous = self.slot("print-continuous-numbering");
        let mut continuous = !matches!(self.variable_value(continuous)?, Value::NIL | Value::UNBOUND);
        // An overriding number table is bound for this call only, as
        // print_bind_overrides binds it; `t' binds every default.
        let mut bound_table = None;
        let settings = if overrides == Value::TRUE { vec![overrides] } else { self.list_values(overrides)? };
        for setting in settings {
            if setting == Value::TRUE {
                (options, continuous, bound_table) = (PrintOptions::default(), false, Some(Value::NIL));
                continue;
            }
            let (key, v) = self.cons_pair(setting)?;
            let key = key.index_if(7).and_then(|n| n.checked_sub(1)).and_then(|n| self.symbol_names.get(n)).ok_or("symbol required")?.clone();
            match key.as_str() {
                "continuous-numbering" => continuous = v != Value::NIL,
                "number-table" => bound_table = Some(v),
                _ if options.set(&key, v) => {}
                _ => return Err("unsupported printing override".into()),
            }
        }
        if buffer_output { (options.nonascii, options.buffer) = (true, true); }
        // print.c print: the number table and its index survive between
        // calls only while print-continuous-numbering and the table are set.
        let table_slot = self.slot("print-number-table");
        let mut table = match bound_table { Some(table) => table, None => self.variable_value(table_slot)? };
        if !continuous || matches!(table, Value::NIL | Value::UNBOUND) {
            self.print_number_index = 0;
            table = Value::NIL;
            if bound_table.is_none() { self.set_print_number_table(table_slot, table)?; }
        }
        if !options.circle { return self.print_object(value, quoted, &options, 0); }
        self.print_preprocess(value, quoted, &mut options, continuous, table)?;
        let printed = self.print_object(value, quoted, &options, 0);
        let slot = if bound_table.is_none() { Some(table_slot) } else { None };
        self.publish_print_number_table(slot, table, options.graph.get_mut())?;
        printed
    }

    /// print_string for princ of a plain string into a multibyte buffer:
    /// the printer's own escaping, without print's numbering state.
    pub(super) fn print_plain_string(&self, value: Value) -> Result<String> {
        let options = PrintOptions { nonascii: true, buffer: true, ..PrintOptions::default() };
        self.print_body(value, false, &options, 0)
    }

    /// GNU print_preprocess: mark each circle candidate in OBJECT's
    /// traversal order, numbering one when it is reached a second time.
    fn print_preprocess(&mut self, value: Value, quoted: bool, options: &mut PrintOptions, continuous: bool, table: Value) -> Result<()> {
        use char_table::CharTableSlot;
        let gensym = options.gensym;
        let graph = options.graph.get_mut();
        if let Some(data::Object::HashTable(t)) = self.object_if(table) {
            for (_, key, mark) in t.entries() {
                let mark = if mark == Value::TRUE { CircleMark::Once }
                    else if let Some(n) = mark.integer_value() { if n < 0 { CircleMark::Pending(-n) } else { CircleMark::Printed(n) } }
                    else if mark == Value::NIL { continue }
                    else { CircleMark::Other(mark) };
                // print_object consults the table only for candidates.
                if self.circle_candidate(key, gensym) { graph.marks.insert((0, key.0), mark); }
            }
        }
        let mut pending = vec![CharTableSlot::Value(value)];
        while let Some(slot) = pending.pop() {
            let v = match slot {
                CharTableSlot::Node(node) => {
                    let key = (1, char_table::char_table_node_id(&node) as u64);
                    match graph.marks.get(&key) {
                        None => {
                            graph.marks.insert(key, CircleMark::Once);
                            pending.extend(node.lock().unwrap().slots.iter().rev().cloned());
                        }
                        Some(CircleMark::Once) => {
                            self.print_number_index += 1;
                            graph.marks.insert(key, CircleMark::Pending(self.print_number_index as i64));
                        }
                        Some(_) => {}
                    }
                    continue;
                }
                CharTableSlot::Value(v) => v,
            };
            if let Some(data::Object::SubCharTable(node)) = self.object_if(v) {
                pending.push(CharTableSlot::Node(std::sync::Arc::clone(node)));
                continue;
            }
            if !self.circle_candidate(v, gensym) { continue; }
            let key = (0, v.0);
            let mark = graph.marks.get(&key).copied();
            // A gensym under continuous numbering is always numbered, for
            // bytecomp's byte-compile-output-docform.
            if mark.is_some() || (continuous && v.tag() == 7) {
                if matches!(mark, None | Some(CircleMark::Once)) || matches!(mark, Some(CircleMark::Other(m)) if m.tag() == 7) {
                    if mark.is_none() { graph.inserted.push(v); }
                    self.print_number_index += 1;
                    graph.marks.insert(key, CircleMark::Pending(self.print_number_index as i64));
                }
                continue;
            }
            graph.marks.insert(key, CircleMark::Once);
            graph.inserted.push(v);
            // The stack pops children left to right, as print.c's does.
            if v.tag() == 3 { let (a, d) = self.cons_pair(v)?; pending.extend([CharTableSlot::Value(d), CharTableSlot::Value(a)]); }
            else if let Some(data::Object::Vector(v) | data::Object::Record(v)) = self.object_if(v) { pending.extend(v.iter().rev().copied().map(CharTableSlot::Value)); }
            else if let Some(data::Object::CharTable(t)) = self.object_if(v) {
                pending.extend(t.print_slots().into_iter().rev());
            }
            else if let Some(data::Object::HashTable(t)) = self.object_if(v) {
                let entries: Vec<Value> = t.entries().flat_map(|(_, k, v)| [k, v]).collect();
                pending.extend(entries.into_iter().rev().map(CharTableSlot::Value));
            }
            else if quoted {
                for (_, _, properties) in self.print_string_properties(v).into_iter().rev() {
                    pending.extend(properties.into_iter().rev().map(|(_, value)| CharTableSlot::Value(value)));
                }
            }
        }
        // Objects seen once never take a label.
        graph.marks.retain(|_, mark| *mark != CircleMark::Once);
        Ok(())
    }

    /// print_circle_candidate_p: conses, nonempty strings and arrays, and
    /// under print-gensym uninterned symbols.
    fn circle_candidate(&self, v: Value, gensym: bool) -> bool {
        v.tag() == 3
            || (gensym && self.printed_as_gensym(v) && !self.obarray_symbol_names.contains_key(&(v.payload() as usize - 1)))
            || matches!(self.object_if(v), Some(data::Object::Record(_) | data::Object::CharTable(_) | data::Object::HashTable(_)))
            || matches!(self.object_if(v), Some(data::Object::Vector(v)) if !v.is_empty())
            || self.str_view(v).is_some_and(|s| !s.bytes().is_empty())
    }

    /// Store the marks in print-number-table as print.c does: entries keep
    /// the insertion order of the traversal, and objects seen once are
    /// removed. Sub-char-tables have no Lisp key and stay private. A new
    /// table goes to the variable's SLOT; without one (an overriding
    /// binding) nothing outlives the call to see it.
    fn publish_print_number_table(&mut self, slot: Option<usize>, mut table: Value, graph: &PrintGraph) -> Result<()> {
        if !matches!(self.object_if(table), Some(data::Object::HashTable(_))) {
            let Some(slot) = slot.filter(|_| !graph.inserted.is_empty()) else { return Ok(()) };
            table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new(hash::Test::Eq, hash::Weakness::None, 0))));
            self.set_print_number_table(slot, table)?;
        }
        for &key in &graph.inserted { self.declaration_hash_put(table, key, Value::TRUE)?; }
        let Some(data::Object::HashTable(t)) = self.object_if(table) else { unreachable!() };
        let entries: Vec<(Value, Value)> = t.entries().map(|(_, key, mark)| (key, mark)).collect();
        for (key, mark) in entries {
            match graph.marks.get(&(0, key.0)) {
                Some(CircleMark::Pending(n)) => self.declaration_hash_put(table, key, Value::integer(-n)?)?,
                Some(CircleMark::Printed(n)) => self.declaration_hash_put(table, key, Value::integer(*n)?)?,
                Some(CircleMark::Other(_)) => {}
                Some(CircleMark::Once) | None => if mark == Value::TRUE { self.declaration_hash_remove(table, key)? },
            }
        }
        Ok(())
    }

    /// print.c assigns Vprint_number_table directly, without watchers.
    fn set_print_number_table(&mut self, slot: usize, value: Value) -> Result<()> {
        let local = self.variable_binding_slot(slot, true)?;
        if self.slots[local].value != value { self.slots[local].value = value; }
        Ok(())
    }

    fn print_char_table_slots(&self, prefix: &str, slots: &[char_table::CharTableSlot], quoted: bool, options: &PrintOptions, depth: usize) -> Result<String> {
        use char_table::CharTableSlot;
        if depth > 100 { return Err("print nesting limit exceeded".into()); }
        let limit = if prefix == "#^^" { options.length.map(|n| n.saturating_sub(1).saturating_add(2)) } else { options.length };
        let n = limit.unwrap_or(slots.len()).min(slots.len());
        let mut parts = Vec::with_capacity(n);
        for slot in &slots[..n] {
            parts.push(match slot {
                CharTableSlot::Value(v) => self.print_object(*v, quoted, options, depth + 1)?,
                CharTableSlot::Node(node) => {
                    let key = (1, char_table::char_table_node_id(node) as u64);
                    let label = options.graph.borrow_mut().label(key);
                    let prefix = match label {
                        Label::Refer(id) => { parts.push(format!("#{id}#")); continue; }
                        Label::Define(id) => format!("#{id}="),
                        Label::Plain | Label::Text(_) => String::new(),
                    };
                    prefix + &self.print_sub_char_table_body(node, quoted, options, depth + 1)?
                }
            });
        }
        if n < slots.len() { parts.push("...".into()); }
        Ok(format!("{prefix}[{}]", parts.join(" ")))
    }

    fn print_sub_char_table_body(&self, node: &char_table::CharTableNodeHandle, quoted: bool, options: &PrintOptions, depth: usize) -> Result<String> {
        let slots = {
            let node = node.lock().unwrap();
            let mut slots = vec![char_table::CharTableSlot::Value(Value::integer(node.depth as i64)?),
                char_table::CharTableSlot::Value(Value::integer(node.min_char as i64)?)];
            slots.extend(node.slots.iter().cloned());
            slots
        };
        self.print_char_table_slots("#^^", &slots, quoted, options, depth)
    }

    // Properties use byte coordinates; printed intervals use Lisp characters.
    // This snapshot borrows keys and copies Values without allocating Lisp data.
    fn print_string_properties(&self, value: Value) -> Vec<(usize, usize, Vec<(&str, Value)>)> {
        let Some(data::Object::String(s) | data::Object::RawString(s)) = self.object_if(value) else { return Vec::new(); };
        if s.properties.is_empty() { return Vec::new(); }
        let offsets: Vec<_> = (0..=s.len()).map(|i| s.byte_offset(coordinates::CharOffset(i)).unwrap()).collect();
        let mut runs = Vec::new();
        let mut start = 0;
        while start < s.len() {
            let at = coordinates::ByteOffset(offsets[start]);
            let next = s.properties.next_change(at, coordinates::ByteOffset(s.bytes().len()));
            let end = offsets.partition_point(|&b| b < next.0).max(start + 1).min(s.len());
            let properties = s.properties.properties_at(at);
            if !properties.is_empty() { runs.push((start, end, properties)); }
            start = end;
        }
        runs
    }

    /// A symbol print-gensym marks with `#:': one not interned in the
    /// initial obarray.
    fn printed_as_gensym(&self, value: Value) -> bool {
        value.index_if(7).and_then(|n| n.checked_sub(1))
            .is_some_and(|slot| self.symbol_names.get(slot).is_some_and(|name| self.names.get(name) != Some(&slot)))
    }

    fn print_object(&self, value: Value, quoted: bool, options: &PrintOptions, depth: usize) -> Result<String> {
        let key = match self.object_if(value) {
            Some(data::Object::SubCharTable(node)) => (1, char_table::char_table_node_id(node) as u64),
            _ => (0, value.0),
        };
        let container = value.tag() == 3 || matches!(self.object_if(value),
            Some(data::Object::Vector(_) | data::Object::Record(_) | data::Object::HashTable(_)));
        let prefix = {
            let mut graph = options.graph.borrow_mut();
            if graph.nesting + graph.quotes >= 100 { return Err("print nesting limit exceeded".into()); }
            if !options.circle && container && let Some(depth) = graph.path.iter().position(|&p| p == value.0) {
                return Ok(format!("#{depth}"));
            }
            match graph.label(key) {
                Label::Refer(id) => return Ok(format!("#{id}#")),
                Label::Define(id) => format!("#{id}="),
                // A caller's string entry prints in place of the object.
                Label::Text(text) if self.str_view(text).is_some() => {
                    drop(graph);
                    return self.print_body(text, false, options, depth);
                }
                Label::Plain | Label::Text(_) => String::new(),
            }
        };
        {
            let mut graph = options.graph.borrow_mut();
            graph.nesting += 1;
            graph.path.push(value.0);
        }
        let body = self.print_body(value, quoted, options, depth);
        {
            let mut graph = options.graph.borrow_mut();
            graph.nesting -= 1;
            graph.path.pop();
        }
        body.map(|body| prefix + &body)
    }

    fn print_body(&self, value: Value, quoted: bool, options: &PrintOptions, depth: usize) -> Result<String> {
        if depth > 100 { return Err("print nesting limit exceeded".into()); }
        if let Some(view) = self.str_view(value) {
            let properties = if quoted { self.print_string_properties(value) } else { Vec::new() };
            let mut out = if quoted { String::from("\"") } else { String::new() };
            let codes: Vec<_> = view.chars().collect();
            let mut hex = false;
            for (i, &code) in codes.iter().enumerate() {
                use std::fmt::Write;
                // princ into a buffer escapes a unibyte string's raw bytes too.
                if code > lstring::MAX_5_BYTE_CHAR || (!view.is_multibyte() && code >= 128 && (quoted || options.buffer)) {
                    if !view.is_multibyte() && code >= 128 && !options.nonascii {
                        return Err("printing raw-byte strings to callbacks is not supported".into());
                    }
                    let byte = if code > lstring::MAX_5_BYTE_CHAR { code - lstring::BYTE8_OFFSET } else { code };
                    write!(out, "\\{:03o}", byte).unwrap();
                    hex = false;
                } else if quoted && options.multibyte && code >= 128 {
                    write!(out, "\\x{code:04x}").unwrap(); hex = true;
                } else if let Some(c) = char::from_u32(code) {
                    if hex && c.is_ascii_hexdigit() { out.push_str("\\ "); }
                    hex = false;
                    match c {
                        '\"' | '\\' if quoted => { out.push('\\'); out.push(c); }
                        '\n' if quoted && options.newlines => out.push_str("\\n"),
                        '\u{c}' if quoted && options.newlines => out.push_str("\\f"),
                        _ if quoted && options.controls && (code < 32 || code == 127) => {
                            let width = if code > 63 || codes.get(i+1).is_some_and(|c| (48..=55).contains(c)) { 3 } else if code > 7 { 2 } else { 1 };
                            write!(out, "\\{code:0width$o}").unwrap();
                        }
                        // Printed text is decoded as a symbol key (symkey.rs).
                        crate::symkey::ESC => { out.push(c); out.push(c); }
                        _ => out.push(c),
                    }
                } else { return Err("not-unicode-string: printer extended character".into()); }
            }
            if quoted { out.push('\"'); }
            if !properties.is_empty() {
                out.insert_str(0, "#(");
                for (start, end, plist) in properties {
                    let printed = if options.level.is_some_and(|n| depth as i64 + 2 > n) {
                        "...".to_owned()
                    } else {
                        let count = plist.len() * 2;
                        let limit = options.length.unwrap_or(count).min(count);
                        let mut parts = Vec::with_capacity(limit);
                        for (key, v) in plist {
                            if parts.len() >= limit { break; }
                            let slot = self.names.get(key).ok_or("unknown text-property symbol")?;
                            parts.push(self.print_object(Value::reference(slot + 1, 7), true, options, depth + 2)?);
                            if parts.len() >= limit { break; }
                            parts.push(self.print_object(v, true, options, depth + 2)?);
                        }
                        if limit < count { parts.push("...".into()); }
                        format!("({})", parts.join(" "))
                    };
                    use std::fmt::Write;
                    write!(out, " {start} {end} {printed}").unwrap();
                }
                out.push(')');
            }
            return Ok(out);
        }
        if value.tag() == 3 {
            if options.level.is_some_and(|n| depth as i64 + 1 > n) { return Ok("...".into()); }
            let (car, cdr) = self.cons_pair(value)?;
            if options.quoted {
                if let Ok((arg, end)) = self.cons_pair(cdr) {
                    if end == Value::NIL {
                        // Only the interned `quote' and `function', not namesakes.
                        let interned = |name: &str| self.names.get(name).is_some_and(|&slot| car == Value::reference(slot + 1, 7));
                        let prefix = if interned("quote") { Some("'") } else if interned("function") { Some("#'") } else { None };
                        if let Some(prefix) = prefix {
                            // print.c's shorthand is tail recursion: the quoted
                            // object takes this list's depth, so it is not on
                            // the path, and a quote cycle is bounded by depth.
                            let own = {
                                let mut graph = options.graph.borrow_mut();
                                graph.nesting -= 1;
                                graph.quotes += 1;
                                graph.path.pop()
                            };
                            let printed = self.print_object(arg, quoted, options, depth);
                            let mut graph = options.graph.borrow_mut();
                            graph.nesting += 1;
                            graph.quotes -= 1;
                            graph.path.extend(own);
                            return Ok(format!("{prefix}{}", printed?));
                        }
                    }
                }
            }
            let mut parts = Vec::new();
            let mut tail = value;
            let mut seen = HashSet::default();
            while tail.tag() == 3 {
                if options.length.is_some_and(|n| parts.len() >= n) { parts.push("...".into()); return Ok(format!("({})", parts.join(" "))); }
                if !seen.insert(tail.0) { return Err("circular list printing requires print-circle".into()); }
                let (car, cdr) = self.cons_pair(tail)?;
                parts.push(self.print_object(car, quoted, options, depth + 1)?);
                tail = cdr;
                if tail.tag() == 3 && options.graph.borrow().shared((0, tail.0)) {
                    parts.push(format!(". {}", self.print_object(tail, quoted, options, depth + 1)?));
                    tail = Value::NIL;
                }
            }
            if tail != Value::NIL { parts.push(format!(". {}", self.print_object(tail, quoted, options, depth + 1)?)); }
            return Ok(format!("({})", parts.join(" ")));
        }
        if let Some(object @ (data::Object::Vector(values) | data::Object::Record(values))) = self.object_if(value) {
            let n = options.length.unwrap_or(values.len()).min(values.len());
            let mut parts = values[..n].iter().map(|&v| self.print_object(v, quoted, options, depth + 1)).collect::<Result<Vec<_>>>()?;
            if n < values.len() { parts.push("...".into()); }
            return Ok(if matches!(object, data::Object::Record(_)) {
                format!("#s({})", parts.join(" "))
            } else {
                format!("[{}]", parts.join(" "))
            });
        }
        if let Some(data::Object::HashTable(t)) = self.object_if(value) {
            let bytes = self.print_table(t, options.length, |vm, v| vm.print_object(v, quoted, options, depth + 1).map(String::into_bytes))?;
            return String::from_utf8(bytes).map_err(|e| e.to_string());
        }
        if let Some(data::Object::Float(n)) = self.object_if(value) {
            if n.is_nan() {
                return Ok(format!("{}{}.0e+NaN", if n.is_sign_negative() { "-" } else { "" }, n.to_bits() & 0x0007_ffff_ffff_ffff));
            }
        }
        // Text from Lisp data joins output decoded as symbol keys (symkey.rs).
        if let Ok(state) = self.buffer_state(value) {
            return Ok(match &state.name {
                Some(name) if quoted => format!("#<buffer {}>", crate::symkey::escape_text(name)),
                Some(name) => crate::symkey::escape_text(name).into_owned(),
                None => "#<killed buffer>".into(),
            });
        }
        if let Some(text) = self.marker_text(value, true) { return Ok(text); }
        if let Some(name) = value.index_if(7).and_then(|n| n.checked_sub(1)).and_then(|n| self.symbol_names.get(n)) {
            let prefix = if options.gensym && self.printed_as_gensym(value) { "#:" } else { "" };
            if name.is_empty() { return Ok(if prefix.is_empty() { "##" } else { prefix }.into()); }
            if !quoted { return Ok(format!("{prefix}{name}")); }
            let unsigned = name.strip_prefix(['+', '-']).unwrap_or(name);
            let numeric = unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.') && name.parse::<f64>().is_ok();
            let confusing = numeric || name.starts_with('?') || (name.starts_with('.') && !name.chars().nth(1).is_some_and(char::is_alphabetic));
            let mut text = String::from(prefix);
            for (i, c) in name.chars().enumerate() {
                if (i == 0 && confusing) || c <= ' ' || c == '\u{a0}' || "\"\\';#(),`[]".contains(c) { text.push('\\'); }
                text.push(c);
            }
            return Ok(text);
        }
        if let Some(data::Object::Obarray(table)) = self.object_if(value) {
            return Ok(format!("#<obarray n={}>", table.len()));
        }
        if let Some(data::Object::Process(process)) = self.object_if(value) {
            // print.c PVEC_PROCESS: the name, inside #<process ...> when escaping.
            let name = crate::symkey::escape_text(&self.process_name_text(process)).into_owned();
            return Ok(if quoted { format!("#<process {name}>") } else { name });
        }
        if let Some(data::Object::CharTable(table)) = self.object_if(value) {
            return self.print_char_table_slots("#^", &table.print_slots(), quoted, options, depth);
        }
        if let Some(data::Object::SubCharTable(node)) = self.object_if(value) {
            return self.print_sub_char_table_body(node, quoted, options, depth);
        }
        if matches!(self.object_if(value), Some(data::Object::SyntaxTable(_) | data::Object::Overlay(_) | data::Object::Frame(_) | data::Object::Terminal(_))) {
            return Err("printing this object representation is not supported".into());
        }
        self.display(value).map(|text| crate::symkey::escape_text(&text).into_owned())
    }

    /// print.c PVEC_MARKER: `#<marker [(moves after insertion) ]at N in
    /// BUFFER>' or `in no buffer'; ESCAPED for printer output (symkey.rs).
    pub(crate) fn marker_text(&self, value: Value, escaped: bool) -> Option<String> {
        let Some(data::Object::Marker(marker)) = self.object_if(value) else { return None };
        let mut text = String::from("#<marker ");
        if marker.bias == crate::anchors::Bias::Right { text.push_str("(moves after insertion) "); }
        let name = self.buffer_state(marker.buffer).ok().and_then(|state| state.name.clone());
        match (name, self.marker_position_value(value).ok().and_then(|p| p.as_integer().ok())) {
            (Some(name), Some(position)) => {
                let name = if escaped { crate::symkey::escape_text(&name).into_owned() } else { name };
                text.push_str(&format!("at {position} in {name}"));
            }
            _ => text.push_str("in no buffer"),
        }
        text.push('>');
        Some(text)
    }
}

#[derive(Default)]
/// NESTING is the print depth; PATH the objects being printed at each depth
/// (print.c being_printed), which a container met again prints as #DEPTH
/// without print-circle; QUOTES counts the quote shorthands in progress,
/// which take no depth of their own but still bound the recursion.
/// MARKS is print.c's number table for print-circle; INSERTED lists the
/// Lisp keys preprocessing added to it, in order.
struct PrintGraph { marks: HashMap<(u8, u64), CircleMark>, inserted: Vec<Value>, nesting: usize, path: Vec<u64>, quotes: usize }
/// A Vprint_number_table value: `t' seen once, -N shared and unprinted,
/// N printed, or any other value a caller stored.
#[derive(Clone, Copy, PartialEq)]
enum CircleMark { Once, Pending(i64), Printed(i64), Other(Value) }
enum Label { Plain, Define(i64), Refer(i64), Text(Value) }
impl PrintGraph {
    /// print_object's table step: `#N=` the first time a shared object
    /// prints, `#N#` after.
    fn label(&mut self, key: (u8, u64)) -> Label {
        match self.marks.get(&key).copied() {
            Some(CircleMark::Pending(n)) => { self.marks.insert(key, CircleMark::Printed(n)); Label::Define(n) }
            Some(CircleMark::Printed(n)) => Label::Refer(n),
            Some(CircleMark::Other(v)) => Label::Text(v),
            Some(CircleMark::Once) | None => Label::Plain,
        }
    }
    /// A list tail with a table entry prints as a dotted cdr.
    fn shared(&self, key: (u8, u64)) -> bool {
        matches!(self.marks.get(&key), Some(CircleMark::Pending(_) | CircleMark::Printed(_) | CircleMark::Other(_)))
    }
}
struct PrintOptions {
    graph: std::cell::RefCell<PrintGraph>,
    length: Option<usize>, level: Option<i64>, quoted: bool, newlines: bool,
    nonascii: bool, multibyte: bool, controls: bool, circle: bool, gensym: bool,
    /// Output goes to a multibyte buffer, as print_string's does.
    buffer: bool,
}
impl Default for PrintOptions {
    fn default() -> Self { Self { graph: std::cell::RefCell::default(), length: None, level: None, quoted: true, newlines: false, nonascii: false, multibyte: false, controls: false, circle: false, gensym: false, buffer: false } }
}
impl PrintOptions {
    fn set(&mut self, key: &str, value: Value) -> bool {
        let on = value != Value::NIL;
        match key {
            "length" => self.length = value.integer_value().and_then(|n| usize::try_from(n).ok()),
            "level" => self.level = value.integer_value(),
            "quoted" => self.quoted = on,
            "escape-newlines" => self.newlines = on,
            "escape-nonascii" => self.nonascii = on,
            "escape-multibyte" => self.multibyte = on,
            "escape-control-characters" => self.controls = on,
            "circle" => self.circle = on,
            "gensym" => self.gensym = on,
            _ => return false,
        }
        true
    }
}

impl Vm {
    /// A process's name, for printing.
    pub(crate) fn process_name_text(&self, process: &crate::subprocess::Process) -> String {
        self.str_view(process.get(crate::subprocess::Slot::Name))
            .map(|view| String::from_utf8_lossy(view.bytes).into_owned())
            .unwrap_or_default()
    }
    /// data.c minmax_driver: markers become their positions, a later
    /// argument wins only when strictly better (ties keep the first, with
    /// its own type), and a NaN that does not win is returned at once.
    pub(super) fn minmax(&mut self, args: &[Value], better: std::cmp::Ordering) -> Result<Value> {
        let mut accum = self.coerce_number(args[0], true)?;
        for &arg in &args[1..] {
            let value = self.coerce_number(arg, true)?;
            if self.numeric_cmp(value, accum)? == Some(better) {
                accum = value;
            } else if self.number(value)?.is_nan() {
                return Ok(value);
            }
        }
        Ok(accum)
    }
}

