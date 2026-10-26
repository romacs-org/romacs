//! Keymaps use a traced, Lisp-mutable cons graph, with character tables for dense maps.
//! No Rust side table caches bindings: setcdr/nconc edits remain authoritative.
use super::*;

/// A walk stopped at a menu filter it has no result for yet.
pub(crate) const KEYMAP_FILTER_PENDING: &str = "keymap menu filter must be evaluated (VM2 key-binding and define-key do not)";
use data::Object;

struct KeyEvent { value: Value, input_index: usize }

impl Vm {
    fn keymap_symbol(&mut self, name: &str) -> Value {
        Value::reference(self.slot(name) + 1, 7)
    }
    pub(super) fn keymap_cons(&mut self, car: Value, cdr: Value) -> Value {
        let index = self.conses.len();
        self.conses.push((car, cdr));
        Value::reference(index, 3)
    }
    fn sparse_keymap(&mut self, prompt: Value) -> Value {
        let head = self.keymap_symbol("keymap");
        if prompt == Value::NIL { self.list_value(&[head]) }
        else { self.list_value(&[head, prompt]) }
    }
    pub(super) fn dense_keymap(&mut self, prompt: Value) -> Value {
        let head = self.keymap_symbol("keymap");
        let table = self.allocate_object(Object::CharTable(Box::new(char_table::CharTable::new(head))));
        if prompt == Value::NIL { self.list_value(&[head, table]) }
        else { self.list_value(&[head, table, prompt]) }
    }
    pub(super) fn ensure_keymaps(&mut self) {
        // Lazy, not eager at `Vm::new`. GNU installs the prefix/global maps from
        // subr.el -- they are Lisp-level state, not part of constructing a VM --
        // and a bare VM must stay bare: the collector's tests assert exact heap
        // contents and arena indices, so a skeleton allocated in the constructor
        // silently rewrites what "empty heap" means for every one of them.
        // Installing on first keymap use keeps both properties true.
        if self.names.contains_key("global-map") {
            return;
        }
        self.initialize_keymaps();
    }
    pub(super) fn initialize_keymaps(&mut self) {
        // GNU installs the prefix/global maps from subr.el; VM2 boots without
        // loading subr.el. Only the data skeleton is installed, not commands.
        for name in ["global-map", "esc-map", "ctl-x-map", "function-key-map", "key-translation-map", "special-event-map"] {
            let map = self.sparse_keymap(Value::NIL);
            self.declare_special_raw(name, map);
        }
        for (name, alias, event) in [("esc-map", "ESC-prefix", 27), ("ctl-x-map", "Control-X-prefix", 24)] {
            let map = self.slots[self.names[name]].value;
            let slot = self.slot(alias);
            self.slots[slot].function = map;
            let global = self.slots[self.names["global-map"]].value;
            self.keymap_store(global, Value::integer(event).unwrap(), Value::reference(slot + 1, 7), false).unwrap();
        }
        let global = self.slots[self.names["global-map"]].value;
        self.declare_special_raw("#:vm2-global-keymap", global);
        self.declare_special_raw("meta-prefix-char", Value::integer(27).unwrap());
        self.initialize_command_bindings(global);
    }
    /// One `lookup-key' walk with FILTERS as the known menu-filter results:
    /// the binding, or the first filter still to run.
    pub(crate) fn lookup_key_walk(&mut self, map: Value, key: Value, defaults: bool, filters: Vec<(Value, Value)>)
        -> Result<std::result::Result<Value, (Value, Value, Value)>> {
        self.keymap_filters = filters;
        self.keymap_pending_filter = None;
        let result = self.key_events(key).and_then(|events| self.lookup_keymap_key(map, &events, defaults));
        self.keymap_filters.clear();
        match result {
            Err(e) if e == KEYMAP_FILTER_PENDING => Ok(Err(self.keymap_pending_filter.take().expect("pending filter"))),
            other => other.map(Ok),
        }
    }
    pub(super) fn resolve_keymap(&self, mut value: Value) -> Option<Value> {
        for _ in 0..=self.slots.len() {
            if let Ok((head, _)) = self.cons_pair(value) {
                return (head == Value::reference(*self.names.get("keymap")? + 1, 7)).then_some(value);
            }
            let slot = value.index_if(7)?.checked_sub(1)?;
            value = self.slots.get(slot)?.function;
        }
        None
    }
    /// keymap.c get_keyelt with AUTOLOAD: a menu item's `:filter' result
    /// comes from `keymap_filters'; one not computed yet is left pending
    /// for the caller to run (KEYMAP_FILTER_PENDING).
    fn keymap_definition(&mut self, mut value: Value) -> Result<Value> {
        for _ in 0..=64 {
            let Ok((head, tail)) = self.cons_pair(value) else { return Ok(value); };
            if self.names.get("menu-item").is_some_and(|&s| head == Value::reference(s + 1, 7)) {
                let item = value;
                let Ok((_, definition)) = self.cons_pair(tail) else { return Ok(value); };
                let object = self.cons_pair(definition).map_or(definition, |p| p.0);
                let mut properties = definition;
                let mut filter = None;
                for _ in 0..=self.list_limit() {
                    let Ok((key, next)) = self.cons_pair(properties) else { break; };
                    if self.cons_pair(next).is_err() { break; }
                    if self.names.get(":filter").is_some_and(|&s| key == Value::reference(s + 1, 7)) {
                        filter = Some(self.cons_pair(next)?.0);
                        break;
                    }
                    properties = next;
                    if properties == definition { return Err("circular menu item".into()); }
                }
                value = match filter {
                    None => object,
                    Some(filter) => match self.keymap_filters.iter().find(|(i, _)| *i == item) {
                        Some(&(_, result)) => result,
                        None => {
                            self.keymap_pending_filter = Some((item, filter, object));
                            return Err(KEYMAP_FILTER_PENDING.into());
                        }
                    },
                };
            } else if self.str_view(head).is_some() {
                value = tail;
            } else { return Ok(value); }
        }
        Err("keymap menu nesting limit exceeded".into())
    }
    fn keymap_entry(&mut self, map: Value, event: Value, inherit: bool, defaults: bool) -> Result<Option<Value>> {
        self.keymap_entry_at(map, event, inherit, defaults, 0)
    }
    fn keymap_entry_at(&mut self, map: Value, event: Value, inherit: bool, defaults: bool, depth: usize) -> Result<Option<Value>> {
        if depth > 64 { return Err("keymap nesting limit exceeded".into()); }
        let (_, mut tail) = self.cons_pair(map)?;
        let mut fallback = None;
        let mut found = None;
        let limit = self.list_limit();
        for _ in 0..=limit {
            if tail == Value::NIL { return found.or(fallback).map(|v| self.keymap_definition(v)).transpose(); }
            if tail.tag() != 3 {
                let parent = self.resolve_keymap(tail).ok_or("keymap parent required")?;
                tail = parent;
            }
            let (entry, next) = self.cons_pair(tail)?;
            let binding = if entry == Value::reference(self.names["keymap"] + 1, 7) {
                if !inherit || found == Some(Value::NIL) { return found.or(fallback).map(|v| self.keymap_definition(v)).transpose(); }
                if let Some(child) = found {
                    let parent = self.keymap_entry_at(tail, event, true, defaults, depth + 1)?;
                    if let Some(parent) = parent.and_then(|v| self.resolve_keymap(v)) {
                        let child_cell = self.keymap_cons(child, parent);
                        let head = self.keymap_symbol("keymap");
                        return Ok(Some(self.keymap_cons(head, child_cell)));
                    }
                    return Ok(found);
                }
                None
            } else if let Some(submap) = self.resolve_keymap(entry) {
                self.keymap_entry_at(submap, event, inherit, defaults, depth + 1)?
            } else if let Ok((key, value)) = self.cons_pair(entry) {
                let range_match = self.cons_pair(key).ok().and_then(|(lo,hi)| Some((lo.integer_value()?,hi.integer_value()?,event.integer_value()?))).is_some_and(|(lo,hi,event)| lo <= event && event <= hi);
                if key == event || (range_match && found.is_none()) { Some(value) }
                else {
                    if defaults && key == Value::TRUE && fallback.is_none() { fallback = Some(value); }
                    None
                }
            } else if matches!(self.object_if(entry), Some(Object::CharTable(_))) {
                event.integer_value().filter(|&c| c >= 0 && c < char_table::CharTable::LEN as i64)
                    .map(|c| self.char_table_get(entry, c as usize)).transpose()?.filter(|&v| v != Value::NIL)
            } else if matches!(self.object_if(entry), Some(Object::Vector(_) | Object::SyntaxTable(_))) {
                return Err("dense keymaps are not supported".into());
            } else { None };
            if let Some(value) = binding {
                let value = self.keymap_definition(if value == Value::TRUE { Value::NIL } else { value })?;
                if self.resolve_keymap(value).is_some() {
                    if let Some(child) = found.filter(|&v| v != Value::NIL) {
                        let head = self.keymap_symbol("keymap");
                        found = Some(self.list_value(&[head, child, value]));
                    } else { found = Some(value); }
                } else {
                    if found.is_none() || found == Some(Value::NIL) { found = Some(value); }
                    if value != Value::NIL { return Ok(found); }
                }
            }
            tail = next;
        }
        Err("circular keymap".into())
    }
    pub(super) fn keymap_store(&mut self, map: Value, event: Value, value: Value, remove: bool) -> Result<()> {
        let keymap_symbol = Value::reference(self.names["keymap"] + 1, 7);
        // Where a brand-new binding gets prepended. Normally the map itself, but a
        // nested sub-keymap element takes over below.
        let mut insertion = map;
        let mut previous = map;
        let (_, mut tail) = self.cons_pair(map)?;
        for _ in 0..=self.list_limit() {
            if tail == Value::NIL { break; }
            let (entry, next) = self.cons_pair(tail)?;
            if entry == keymap_symbol { break; }
            if matches!(self.object_if(entry), Some(Object::CharTable(_))) {
                if let Some(c) = event.integer_value().filter(|&c| c >= 0 && c < char_table::CharTable::LEN as i64) {
                    // GNU uses t for an explicit nil binding; nil means absent.
                    let value = if remove { Value::NIL } else if value == Value::NIL { Value::TRUE } else { value };
                    let slot = self.literal_table_slot(value);
                    let Object::CharTable(table) = &mut self.objects[entry.index(0)? - 1] else { unreachable!() };
                    table.set_range_slot(c as usize, c as usize, slot)?;
                    return Ok(());
                }
                insertion = tail;
            }
            if let Ok((key, _)) = self.cons_pair(entry) {
                // GNU `store_in_keymap`: an element that is itself `(keymap ...)` is a
                // sub keymap, and the write is redirected into it -- "we want to update
                // the sub-keymap since the main one might be temporary (built by
                // access_keymap)". That is exactly the composite `access_keymap` returns
                // for an inherited prefix: its first element is the LIVE own sub-map and
                // its tail is the parent's. Appending to the composite instead would
                // write into a temporary and be lost.
                if key == keymap_symbol {
                    insertion = entry;
                    previous = entry;
                    tail = self.cons_pair(entry)?.1;
                    continue;
                }
                if key == event {
                    if remove { self.traversal(Primitive::Setcdr, previous, next)?; }
                    else { self.traversal(Primitive::Setcdr, entry, value)?; }
                    return Ok(());
                }
            }
            previous = tail;
            tail = next;
            if previous == map { return Err("circular keymap".into()); }
        }
        if tail != Value::NIL && self.cons_pair(tail)?.0 != keymap_symbol {
            return Err("circular keymap".into());
        }
        if !remove {
            let binding = self.keymap_cons(event, value);
            let old = self.cons_pair(insertion)?.1;
            let new = self.keymap_cons(binding, old);
            self.traversal(Primitive::Setcdr, insertion, new)?;
        }
        Ok(())
    }
    fn key_event_modifier(name: &str) -> Option<i64> {
        Some(match name {
            "A" | "alt" => 1 << 22,
            "C" | "ctrl" | "control" => 1 << 26,
            "H" | "hyper" => 1 << 24,
            "M" | "meta" => 1 << 27,
            "S" | "shift" => 1 << 25,
            "s" | "super" => 1 << 23,
            "up" => 1,
            "down" => 1 << 1,
            "drag" => 1 << 2,
            "click" => 1 << 3,
            "double" => 1 << 4,
            "triple" => 1 << 5,
            _ => return None,
        })
    }
    fn key_event_name(&self, value: Value) -> Option<&str> {
        if value == Value::NIL { return Some("nil"); }
        if value == Value::TRUE { return Some("t"); }
        self.symbol_names.get(value.index_if(7)?.checked_sub(1)?).map(String::as_str)
    }
    fn modified_key_event_symbol(&mut self, base: &str, modifiers: i64) -> Value {
        // GNU 45b0f769 keyboard.c:7479-7524, apply_modifiers_uncached.
        // Click has no printed prefix; down and drag follow the click count.
        let mut name = String::new();
        for prefix in ["A", "C", "H", "M", "S", "s", "double", "triple", "up", "down", "drag"] {
            if modifiers & Self::key_event_modifier(prefix).unwrap() != 0 {
                name.push_str(prefix);
                name.push('-');
            }
        }
        name.push_str(base);
        self.keymap_symbol(&name)
    }
    fn canonical_key_event_symbol(&mut self, event: Value) -> Result<Value> {
        // GNU keymap.c:338,762 reorder both stored and looked-up symbols.
        // Only abbreviated keyboard modifiers (not `control-`, etc.) are
        // prefixes; solitary list modifiers accept both spellings.
        let name = self.key_event_name(event).ok_or("symbol event required")?.to_owned();
        let mut base = name.as_str();
        let mut modifiers = 0;
        while let Some((prefix, rest)) = base.split_once('-') {
            if !matches!(prefix, "A" | "C" | "H" | "M" | "S" | "s" | "up" | "down" | "drag" | "double" | "triple") {
                break;
            }
            modifiers |= Self::key_event_modifier(prefix).unwrap();
            base = rest;
        }
        if base == name { return Ok(event); }
        Ok(self.modified_key_event_symbol(base, modifiers))
    }
    fn key_event_from_list(&mut self, event: Value) -> Result<Value> {
        // GNU keyboard.c:7832-7888: collect modifier bits before converting
        // the base, so (shift control a) and (control shift a) agree.
        let parts = self.list_values(event)?;
        let Some((&last, modifiers)) = parts.split_last() else { return Err("Invalid base event".into()); };
        let mut mask = 0;
        for &modifier in modifiers {
            mask |= self.key_event_name(modifier).and_then(Self::key_event_modifier)
                .ok_or("Two bases given in one event")?;
        }
        let mut base = last;
        if let Some(name) = self.key_event_name(base) {
            if name.chars().count() == 1 {
                // event-convert-list uses SREF, the first byte, even for a
                // one-character multibyte symbol (keyboard.c:7864-7866).
                base = Value::integer(name.as_bytes()[0] as i64)?;
            }
        }
        if let Some(mut code) = base.integer_value() {
            if mask & (1 << 25) != 0 && (97..=122).contains(&code) {
                code -= 32;
                mask &= !(1 << 25);
            }
            if mask & (1 << 26) != 0 {
                // make_ctrl_char (keyboard.c:2178-2212), not the kbd
                // escape convention: C-SPC/C-? retain the control bit.
                mask &= !(1 << 26);
                code = if (64..=95).contains(&code) {
                    (code & 31) | if (65..=90).contains(&code) { 1 << 25 } else { 0 }
                } else if (97..=122).contains(&code) { code & 31 }
                else if !(0..32).contains(&code) { code | (1 << 26) }
                else { code };
            }
            Value::integer(code | mask)
        } else if let Some(name) = self.key_event_name(base).map(str::to_owned) {
            Ok(self.modified_key_event_symbol(&name, mask))
        } else {
            Err("Invalid base event".into())
        }
    }
    fn key_events(&mut self, sequence: Value) -> Result<Vec<KeyEvent>> {
        let (raw, bytes) = if let Some(view) = self.str_view(sequence) {
            (view.chars().map(|c| Value::integer(c as i64).unwrap()).collect(), !view.is_multibyte())
        } else { match self.object_if(sequence) {
            Some(Object::Vector(v)) => (v.clone(), false),
            _ => return Err("key sequence string or vector required".into()),
        }};
        let mut events = vec![];
        for (input_index, mut event) in raw.into_iter().enumerate() {
            if event.tag() == 3 {
                event = self.key_event_from_list(event)?;
            }
            if event.tag() == 7 {
                event = self.canonical_key_event_symbol(event)?;
            }
            if let Some(mut c) = event.integer_value() {
                if c < 0 { return Err("invalid event".into()); }
                let meta = if bytes { 128 } else { 1 << 27 };
                if c & meta != 0 {
                    let slot = self.names["meta-prefix-char"];
                    events.push(KeyEvent { value: self.slots[slot].value, input_index });
                    c &= !meta;
                }
                event = Value::integer(c)?;
            } else if event.tag() != 7 && event != Value::NIL && event != Value::TRUE && self.str_view(event).is_none() {
                return Err("integer or symbol event required".into());
            }
            events.push(KeyEvent { value: event, input_index });
        }
        Ok(events)
    }
    fn define_keymap_key(&mut self, map: Value, events: &[KeyEvent], value: Value, remove: bool) -> Result<Value> {
        let mut map = self.resolve_keymap(map).ok_or("keymap required")?;
        for (i, event) in events.iter().enumerate() {
            let event = event.value;
            if i + 1 == events.len() { self.keymap_store(map, event, value, remove)?; return Ok(value); }
            let binding = self.keymap_entry(map, event, false, false)?.unwrap_or(Value::NIL);
            map = if binding == Value::NIL {
                let prefix = self.sparse_keymap(Value::NIL);
                self.keymap_store(map, event, prefix, false)?;
                prefix
            } else { self.resolve_keymap(binding).ok_or("key sequence starts with non-prefix key")? };
        }
        Ok(Value::NIL)
    }
    fn lookup_keymap_key(&mut self, map: Value, events: &[KeyEvent], defaults: bool) -> Result<Value> {
        // lookup_key_1: a cons is searched as it is, so a list of keymaps
        // acts as their composition; nil is an empty keymap.
        let is_list = map == Value::NIL || map.tag() == 3;
        if events.is_empty() && is_list { return Ok(map); }
        let mut map = match self.resolve_keymap(map) {
            Some(map) => map,
            None if is_list => { let keymap = self.intern("keymap"); self.keymap_cons(keymap, map) }
            None => return Err("keymap required".into()),
        };
        for (i, event) in events.iter().enumerate() {
            let event = event.value;
            let binding = self.keymap_entry(map, event, true, defaults)?.unwrap_or(Value::NIL);
            if i + 1 == events.len() { return Ok(binding); }
            let Some(prefix) = self.resolve_keymap(binding) else {
                let consumed = events[i].input_index + 1;
                return if events.last().unwrap().input_index < consumed { Ok(Value::NIL) }
                else { Value::integer(consumed as i64) };
            };
            map = prefix;
        }
        Ok(map)
    }
    fn key_modifier(c: i64, modifier: &str) -> Result<i64> {
        Ok(match modifier {
            "C" | "control" => if (64..=95).contains(&c) || (97..=122).contains(&c) { c & 31 } else if c == 63 { 127 } else if c == 32 { 0 } else { c | (1 << 26) },
            "M" | "meta" => c | (1 << 27),
            "S" | "shift" => if (97..=122).contains(&c) { c - 32 } else { c | (1 << 25) },
            "A" | "alt" => c | (1 << 22),
            "s" | "super" => c | (1 << 23),
            "H" | "hyper" => c | (1 << 24),
            _ => return Err("unknown event modifier".into()),
        })
    }
    fn parse_kbd(&mut self, input: Value) -> Result<Value> {
        let text = self.source_string(input)?.to_owned();
        let mut events = vec![];
        for token in text.split_whitespace() {
            let mut rest = token;
            let mut modifiers = vec![];
            while rest.len() >= 2 && rest.as_bytes()[1] == b'-' {
                modifiers.push(&rest[..1]); rest = &rest[2..];
            }
            let named = match rest { "SPC" => Some(32), "RET" => Some(13), "LFD" => Some(10), "TAB" => Some(9), "ESC" => Some(27), "DEL" => Some(127), _ => None };
            let base = named.or_else(|| { let mut cs = rest.chars(); let c = cs.next()?; cs.next().is_none().then_some(c as i64) });
            if let Some(mut c) = base {
                for modifier in modifiers.iter().rev() { c = Self::key_modifier(c, modifier)?; }
                events.push(Value::integer(c)?);
            } else if rest.starts_with('<') && rest.ends_with('>') && rest.len() > 2 {
                let name = format!("{}{}", &token[..token.len() - rest.len()], &rest[1..rest.len() - 1]);
                events.push(self.keymap_symbol(&name));
            } else { return Err("unsupported kbd token".into()); }
        }
        if events.iter().all(|v| v.integer_value().is_some_and(|c| (0..128).contains(&c))) {
            Ok(self.byte_string(&events.iter().map(|v| v.as_integer().unwrap() as u8).collect::<Vec<_>>()))
        } else { Ok(self.allocate_object(Object::Vector(events))) }
    }
    pub(super) fn keymap_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        self.ensure_keymaps();
        let valid = match p { MakeSparseKeymap => n <= 1, Keymapp | UseLocalMap | Kbd => n == 1, DefineKey => (3..=4).contains(&n), LookupKey => (2..=3).contains(&n), KeyBinding => (1..=4).contains(&n), GlobalSetKey => n == 2, CurrentGlobalMap => n == 0, _ => false };
        if !valid { return Err("wrong primitive arity".into()); }
        let args = self.stack[start..start + n].to_vec();
        match p {
            MakeSparseKeymap => Ok(self.sparse_keymap(args.first().copied().unwrap_or(Value::NIL))),
            Keymapp => Ok(if self.resolve_keymap(args[0]).is_some() { Value::TRUE } else { Value::NIL }),
            Kbd => self.parse_kbd(args[0]),
            CurrentGlobalMap => Ok(self.slots[self.names["#:vm2-global-keymap"]].value),
            DefineKey | GlobalSetKey => {
                let (map, sequence, value, remove) = if p == DefineKey { (args[0], args[1], args[2], n == 4 && args[3] != Value::NIL) }
                else { (self.slots[self.names["#:vm2-global-keymap"]].value, args[0], args[1], false) };
                let events = self.key_events(sequence)?;
                let result = self.define_keymap_key(map, &events, value, remove)?;
                Ok(if p == GlobalSetKey { Value::NIL } else { result })
            }
            LookupKey => { let events = self.key_events(args[1])?; self.lookup_keymap_key(args[0], &events, n == 3 && args[2] != Value::NIL) }
            UseLocalMap => {
                if args[0] != Value::NIL && self.resolve_keymap(args[0]).is_none() { return Err("keymap required".into()); }
                let index = self.buffer_index()?;
                let Object::Buffer(buffer) = &mut self.objects[index] else { return Err("Unicode buffer required".into()); };
                buffer.local_keymap = args[0];
                Ok(Value::NIL)
            }
            KeyBinding => {
                // VM2 key-binding never remaps, which is NO-REMAP t's lookup.
                if n > 3 && args[3] != Value::NIL { return Err("key-binding POSITION not supported".into()); }
                let events = self.key_events(args[0])?;
                let index = self.buffer_index()?;
                let Object::Buffer(buffer) = &self.objects[index] else { return Err("Unicode buffer required".into()); };
                let local = buffer.local_keymap;
                let defaults = n > 1 && args[1] != Value::NIL;
                if local != Value::NIL {
                    let binding = self.lookup_keymap_key(local, &events, defaults)?;
                    if binding != Value::NIL && binding.integer_value().is_none() { return Ok(binding); }
                }
                let global = self.slots[self.names["#:vm2-global-keymap"]].value;
                let binding = self.lookup_keymap_key(global, &events, defaults)?;
                Ok(if binding.integer_value().is_some() { Value::NIL } else { binding })
            }
            _ => unreachable!(),
        }
    }
}
