//! Keymaps use a traced, Lisp-mutable cons graph, with character tables for dense maps.
//! No Rust side table caches bindings: setcdr/nconc edits remain authoritative.
use super::*;

/// A walk stopped at a menu filter it has no result for yet.
pub(crate) const KEYMAP_FILTER_PENDING: &str = "keymap menu filter must be evaluated (VM2 define-key does not)";
use data::Object;

struct KeyEvent { value: Value, input_index: usize }

impl Vm {
    fn keymap_symbol(&mut self, name: &str) -> Value {
        Value::reference(self.slot(name) + 1, 7)
    }
    pub(super) fn keymap_cons(&mut self, car: Value, cdr: Value) -> Value {
        self.alloc_cons(car, cdr)
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
        // Installing on first keymap use keeps both properties true. The
        // internal map marks installation: reading a form interns
        // `global-map' before any keymap primitive runs.
        if self.names.contains_key("#:vm2-global-keymap") {
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
        // keyboard.c init_kboard: the terminal's input-decode-map starts
        // empty and its local-function-key-map inherits function-key-map.
        let decode = self.sparse_keymap(Value::NIL);
        self.declare_special_raw("input-decode-map", decode);
        let head = self.keymap_symbol("keymap");
        let local = self.keymap_cons(head, self.slots[self.names["function-key-map"]].value);
        self.declare_special_raw("local-function-key-map", local);
        for (name, alias, event) in [("esc-map", "ESC-prefix", 27), ("ctl-x-map", "Control-X-prefix", 24)] {
            let map = self.slots[self.names[name]].value;
            let slot = self.slot(alias);
            // Through the one writer of a function cell, which moves the
            // slot's version on: a call site that cached what the alias named
            // before must not go on trusting it.
            self.define_function(slot, map, false).expect("a function cell's version");
            let global = self.slots[self.names["global-map"]].value;
            self.keymap_store(global, Value::integer(event).unwrap(), Value::reference(slot + 1, 7), false).unwrap();
        }
        let global = self.slots[self.names["global-map"]].value;
        self.declare_special_raw("#:vm2-global-keymap", global);
        self.declare_special_raw("meta-prefix-char", Value::integer(27).unwrap());
        // keyboard.c keys_of_keyboard: the file notification and focus
        // events' special bindings.
        let special = self.slots[self.names["special-event-map"]].value;
        for (event, command) in [("file-notify", "file-notify-handle-event"), ("focus-in", "handle-focus-in"), ("focus-out", "handle-focus-out")] {
            let (event, command) = (self.intern(event), self.intern(command));
            self.keymap_store(special, event, command, false).unwrap();
        }
        self.initialize_command_bindings(global);
    }
    /// MAP's binding of EVENT alone (a character, a meta character standing
    /// for ESC and its base, or a function-key symbol), or of `[remap
    /// EVENT]' when REMAP, with default bindings when DEFAULTS: nil when
    /// unbound or only a prefix of more.  An
    /// error when a menu item's `:filter' would have to run, which a caller
    /// that must not run Lisp takes as "cannot tell".
    pub(crate) fn lookup_one_event(&mut self, map: Value, event: Value, remap: bool, defaults: bool) -> Result<Value> {
        let events = if remap {
            vec![KeyEvent { value: self.keymap_symbol("remap"), input_index: 0 }, KeyEvent { value: event, input_index: 1 }]
        } else if let Some(c) = event.integer_value() {
            let meta = 1 << 27;
            if c & meta != 0 {
                let prefix = self.slots[self.names["meta-prefix-char"]].value;
                vec![KeyEvent { value: prefix, input_index: 0 }, KeyEvent { value: Value::integer(c & !meta)?, input_index: 0 }]
            } else { vec![KeyEvent { value: event, input_index: 0 }] }
        } else {
            vec![KeyEvent { value: self.canonical_key_event_symbol(event)?, input_index: 0 }]
        };
        self.keymap_filters.clear();
        self.keymap_pending_filter = None;
        let binding = self.lookup_keymap_key(map, &events, defaults);
        self.keymap_pending_filter = None;
        let binding = binding?;
        Ok(if binding.integer_value().is_some() { Value::NIL } else { binding })
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
    /// The local keymap of the buffer at INDEX, text or unibyte.
    pub(crate) fn local_keymap(&mut self, index: usize) -> &mut Value {
        match self.objects.entry_mut(index) {
            Object::Buffer(buffer) => &mut buffer.local_keymap,
            Object::ByteBuffer(buffer) => &mut buffer.local_keymap,
            _ => unreachable!("buffer_index names a buffer"),
        }
    }
    pub(super) fn resolve_keymap(&self, value: Value) -> Option<Value> {
        self.resolve_keymap_as(value, Value::reference(*self.names.get("keymap")? + 1, 7))
    }
    /// `resolve_keymap' with KEYMAP, the symbol, already found: a walk
    /// resolves every entry it meets.
    fn resolve_keymap_as(&self, mut value: Value, keymap: Value) -> Option<Value> {
        for _ in 0..=self.slots.len() {
            // The tag first: a failed `cons_pair' formats its error.
            if value.tag() == 3 {
                let (head, _) = self.cons_pair(value).ok()?;
                return (head == keymap).then_some(value);
            }
            let slot = value.index_if(7)?.checked_sub(1)?;
            value = self.slots.get(slot)?.function;
        }
        None
    }
    /// keymap.c Fdefine_key: a definition vector whose first event is a
    /// list is an XEmacs-style keyboard macro, stored with each modifier
    /// list converted (`event-convert-list'), as bindings.el's keypad
    /// translations `[(48)]' and `[(control 48)]' are `[48]' and `[C-0]'.
    fn keyboard_macro_definition(&mut self, value: Value) -> Result<Value> {
        let Some(Object::Vector(events)) = self.object_if(value) else { return Ok(value) };
        if events.first().is_none_or(|event| event.tag() != 3) { return Ok(value); }
        let mut events = events.clone();
        for event in events.iter_mut() {
            if event.tag() == 3 && self.lucid_event_type_list_p(*event) { *event = self.key_event_from_list(*event)?; }
        }
        Ok(self.allocate_object(Object::Vector(events)))
    }
    /// keymap.c get_keymap without AUTOLOAD, for `keymapp': a symbol whose
    /// function is a keymap's autoload, `(autoload FILE DOC INTERACTIVE
    /// keymap)', is a keymap before its file loads.
    pub(crate) fn autoloaded_keymap(&self, object: Value) -> bool {
        let (Some(&autoload), Some(&keymap)) = (self.names.get("autoload"), self.names.get("keymap")) else { return false };
        if object == Value::NIL || object.index_if(7).is_none() { return false; }
        let mut value = object;
        for _ in 0..=self.slots.len() {
            let Some(slot) = value.index_if(7).and_then(|i| i.checked_sub(1)) else { break };
            match self.slots.get(slot) { Some(s) if s.function != Value::NIL => value = s.function, _ => return false }
        }
        let Ok((head, mut rest)) = self.cons_pair(value) else { return false };
        if head != Value::reference(autoload + 1, 7) { return false; }
        // Past FILE, DOC and INTERACTIVE to TYPE.
        for _ in 0..3 {
            let Ok((_, tail)) = self.cons_pair(rest) else { return false };
            rest = tail;
        }
        self.cons_pair(rest).is_ok_and(|(kind, _)| kind == Value::reference(keymap + 1, 7))
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
        let keymap = Value::reference(self.names["keymap"] + 1, 7);
        let (_, mut tail) = self.cons_pair(map)?;
        let mut fallback = None;
        let mut found = None;
        let mut prefixes: Vec<Value> = Vec::new();
        let limit = self.list_limit();
        for _ in 0..=limit {
            if tail == Value::NIL { return self.keymap_prefixes(found, &prefixes).or(fallback).map(|v| self.keymap_definition(v)).transpose(); }
            if tail.tag() != 3 {
                let parent = self.resolve_keymap_as(tail, keymap).ok_or("keymap parent required")?;
                tail = parent;
            }
            let (entry, next) = self.cons_pair(tail)?;
            let binding = if entry == keymap {
                if !inherit || found == Some(Value::NIL) { return self.keymap_prefixes(found, &prefixes).or(fallback).map(|v| self.keymap_definition(v)).transpose(); }
                if let Some(child) = found {
                    let parent = self.keymap_entry_at(tail, event, true, defaults, depth + 1)?;
                    if let Some(parent) = parent.and_then(|v| self.resolve_keymap_as(v, keymap)) {
                        // The prefixes found, then the parent's as their tail.
                        let prefixes = if prefixes.is_empty() { vec![child] } else { prefixes };
                        let composed = prefixes.iter().rev().fold(parent, |tail, &prefix| self.keymap_cons(prefix, tail));
                        let head = self.keymap_symbol("keymap");
                        return Ok(Some(self.keymap_cons(head, composed)));
                    }
                    return Ok(self.keymap_prefixes(found, &prefixes));
                }
                None
            } else if let Some(submap) = self.resolve_keymap_as(entry, keymap) {
                self.keymap_entry_at(submap, event, inherit, defaults, depth + 1)?
            } else if let Some((key, value)) = (entry.tag() == 3).then(|| self.cons_pair(entry).ok()).flatten() {
                let range_match = (key.tag() == 3).then(|| self.cons_pair(key).ok()).flatten().and_then(|(lo,hi)| Some((lo.integer_value()?,hi.integer_value()?,event.integer_value()?))).is_some_and(|(lo,hi,event)| lo <= event && event <= hi);
                if key == event || (range_match && found.is_none()) { Some(value) }
                else {
                    if defaults && key == Value::TRUE && fallback.is_none() { fallback = Some(value); }
                    None
                }
            } else if matches!(self.object_if(entry), Some(Object::CharTable(_))) {
                event.integer_value().filter(|&c| c >= 0 && c < char_table::CharTable::LEN as i64)
                    .map(|c| self.char_table_get(entry, c as usize)).transpose()?.filter(|&v| v != Value::NIL)
            } else if let Some(Object::Vector(slots)) = self.object_if(entry) {
                // access_keymap_1: a vector binds each character below its
                // length, a nil slot an explicit nil that shadows a parent.
                event.integer_value().and_then(|c| usize::try_from(c).ok()).and_then(|c| slots.get(c).copied())
            } else if matches!(self.object_if(entry), Some(Object::SyntaxTable(_))) {
                return Err("dense keymaps are not supported".into());
            } else { None };
            if let Some(value) = binding {
                let value = self.keymap_definition(if value == Value::TRUE { Value::NIL } else { value })?;
                if self.resolve_keymap_as(value, keymap).is_some() {
                    // FOUND is the first prefix binding; the walk's answer
                    // is all of them (`keymap_prefixes').
                    if prefixes.is_empty() { found = Some(value); }
                    prefixes.push(value);
                } else {
                    if found.is_none() || found == Some(Value::NIL) { found = Some(value); }
                    if value != Value::NIL { return Ok(self.keymap_prefixes(found, &prefixes)); }
                }
            }
            tail = next;
        }
        Err("circular keymap".into())
    }
    /// GNU's `access_keymap_1`: several prefix bindings of one event compose as
    /// one flat (keymap PREFIX...), in the order found; made once, when
    /// the walk ends.  FOUND otherwise.
    fn keymap_prefixes(&mut self, found: Option<Value>, prefixes: &[Value]) -> Option<Value> {
        if prefixes.len() < 2 { return found; }
        let head = self.keymap_symbol("keymap");
        let composed: Vec<Value> = std::iter::once(head).chain(prefixes.iter().copied()).collect();
        Some(self.list_value(&composed))
    }
    pub(super) fn keymap_store(&mut self, map: Value, event: Value, value: Value, remove: bool) -> Result<()> {
        self.note_keymap_change();
        let keymap_symbol = Value::reference(self.names["keymap"] + 1, 7);
        let range = self.character_range(event)?;
        // GNU uses t for an explicit nil binding in a char-table; nil means absent.
        let table_value = if remove { Value::NIL } else if value == Value::NIL { Value::TRUE } else { value };
        // Where a brand-new binding gets prepended. Normally the map itself, but a
        // nested sub-keymap element takes over below.
        let mut insertion = map;
        let mut previous = map;
        let (_, mut tail) = self.cons_pair(map)?;
        for _ in 0..=self.list_limit() {
            if tail == Value::NIL { break; }
            let (entry, next) = self.cons_pair(tail)?;
            if entry == keymap_symbol { break; }
            if let Some(Object::Vector(slots)) = self.object_if(entry) {
                // store_in_keymap: a vector takes a character below its
                // length, and the part of a range below it; the rest of the
                // range, and a key past it, go after it.
                let len = slots.len();
                if let Some(c) = event.integer_value().and_then(|c| usize::try_from(c).ok()).filter(|&c| c < len) {
                    self.keymap_vector_fill(entry, c, c, value)?;
                    return Ok(());
                }
                if let Some((from, to)) = range.filter(|_| len > 0) {
                    let last = to.min(len - 1);
                    if from <= last { self.keymap_vector_fill(entry, from, last, value)?; }
                    if last == to { return Ok(()); }
                }
                insertion = tail;
            }
            if matches!(self.object_if(entry), Some(Object::CharTable(_))) {
                let single = event.integer_value().filter(|&c| c >= 0 && c < char_table::CharTable::LEN as i64).map(|c| (c as usize, c as usize));
                if let Some((from, to)) = single.or(range) {
                    self.set_keymap_table_range(entry, from, to, table_value)?;
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
                if let Some((from, to)) = range {
                    // An alist binding of a character inside the range is set
                    // (or removed); a one-character range is then done.
                    if key.integer_value().is_some_and(|c| (from as i64..=to as i64).contains(&c)) {
                        if remove { self.traversal(Primitive::Setcdr, previous, next)?; }
                        else { self.traversal(Primitive::Setcdr, entry, value)?; }
                        if from == to { return Ok(()); }
                        if remove { tail = next; continue; }
                    }
                } else if key == event {
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
            // A range this keymap has no char-table for gets a new one.
            let binding = if let Some((from, to)) = range {
                let head = self.keymap_symbol("keymap");
                let table = self.allocate_object(Object::CharTable(Box::new(char_table::CharTable::new(head))));
                self.set_keymap_table_range(table, from, to, table_value)?;
                table
            } else { self.keymap_cons(event, value) };
            let old = self.cons_pair(insertion)?.1;
            let new = self.keymap_cons(binding, old);
            self.traversal(Primitive::Setcdr, insertion, new)?;
        }
        Ok(())
    }
    /// A dense keymap's VECTOR binds FROM through TO to VALUE, as given.
    fn keymap_vector_fill(&mut self, vector: Value, from: usize, to: usize, value: Value) -> Result<()> {
        let Object::Vector(slots) = self.object_mut(vector)? else { return Err("vector required".into()) };
        slots[from..=to].fill(value);
        Ok(())
    }
    fn set_keymap_table_range(&mut self, table: Value, from: usize, to: usize, value: Value) -> Result<()> {
        let slot = self.literal_table_slot(value);
        let Object::CharTable(table) = self.object_mut(table)? else { unreachable!() };
        table.set_range_slot(from, to, slot)
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
        let (modifiers, base) = parse_event_modifiers(name.as_bytes());
        if base == 0 { return Ok(event); }
        Ok(self.modified_key_event_symbol(&name[base..], modifiers))
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
    /// keyboard.c lucid_event_type_list_p: a proper list of symbols and
    /// fixnums not headed by a mode-line-like area symbol.
    fn lucid_event_type_list_p(&self, object: Value) -> bool {
        let Ok((head, _)) = self.cons_pair(object) else { return false };
        if ["help-echo", "vertical-line", "mode-line", "tab-line", "header-line"].iter()
            .any(|name| self.names.get(*name).is_some_and(|&slot| head == Value::reference(slot + 1, 7))) {
            return false;
        }
        let mut tail = object;
        for _ in 0..=self.list_limit() {
            match self.cons_pair(tail) {
                Ok((element, rest)) => {
                    if element.integer_value().is_none() && element.tag() != 7 && element != Value::NIL && element != Value::TRUE { return false; }
                    tail = rest;
                }
                Err(_) => return tail == Value::NIL,
            }
        }
        false
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
                // keymap.c: a modifier list (lucid_event_type_list_p) is
                // converted; a (FROM . TO) character range stays as it is
                // for store_in_keymap; another list, such as a mouse click,
                // stands for its EVENT_HEAD.
                if self.lucid_event_type_list_p(event) {
                    event = self.key_event_from_list(event)?;
                } else if self.character_range(event)?.is_some() {
                    events.push(KeyEvent { value: event, input_index });
                    continue;
                } else {
                    event = self.cons_pair(event)?.0;
                }
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
    /// keymap.c's flush of where_is_cache: a keymap changed.
    pub(crate) fn note_keymap_change(&mut self) {
        self.keymap_generation = self.keymap_generation.wrapping_add(1);
    }
    pub(crate) fn keymap_generation(&self) -> u64 {
        self.keymap_generation
    }
    /// A (FROM . TO) key event of characters (store_in_keymap's range idx,
    /// CHECK_CHARACTER_CDR); None for any other event.
    fn character_range(&self, event: Value) -> Result<Option<(usize, usize)>> {
        let Ok((from, to)) = self.cons_pair(event) else { return Ok(None) };
        let character = |v: Value| v.integer_value().filter(|c| (0..char_table::CharTable::LEN as i64).contains(c));
        let Some(from) = character(from) else { return Ok(None) };
        let to = character(to).ok_or("character range end must be a character")?;
        Ok(Some((from as usize, to as usize)))
    }
    /// EVENT_HEAD of a range: lookups use its first character.
    fn event_head(&self, event: Value) -> Value {
        if event.tag() == 3 { self.cons_pair(event).map_or(event, |(head, _)| head) } else { event }
    }
    fn define_keymap_key(&mut self, map: Value, events: &[KeyEvent], value: Value, remove: bool) -> Result<Value> {
        let mut map = self.resolve_keymap(map).ok_or("keymap required")?;
        for (i, event) in events.iter().enumerate() {
            let event = event.value;
            if i + 1 == events.len() { self.keymap_store(map, event, value, remove)?; return Ok(value); }
            // Fdefine_key: a range must be a leaf.
            if event.tag() == 3 { return Err("Key sequence contains invalid event".into()); }
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
            let event = self.event_head(event.value);
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
        let valid = match p { MakeSparseKeymap => n <= 1, Keymapp | UseLocalMap | Kbd => n == 1, DefineKey => (3..=4).contains(&n), LookupKey => (2..=3).contains(&n), GlobalSetKey => n == 2, CurrentGlobalMap => n == 0, _ => false };
        if !valid { return Err("wrong primitive arity".into()); }
        let args = self.stack[start..start + n].to_vec();
        match p {
            MakeSparseKeymap => Ok(self.sparse_keymap(args.first().copied().unwrap_or(Value::NIL))),
            Keymapp => Ok(if self.resolve_keymap(args[0]).is_some() || self.autoloaded_keymap(args[0]) { Value::TRUE } else { Value::NIL }),
            Kbd => self.parse_kbd(args[0]),
            CurrentGlobalMap => Ok(self.slots[self.names["#:vm2-global-keymap"]].value),
            DefineKey | GlobalSetKey => {
                let (map, sequence, value, remove) = if p == DefineKey { (args[0], args[1], args[2], n == 4 && args[3] != Value::NIL) }
                else { (self.slots[self.names["#:vm2-global-keymap"]].value, args[0], args[1], false) };
                let value = self.keyboard_macro_definition(value)?;
                let events = self.key_events(sequence)?;
                let result = self.define_keymap_key(map, &events, value, remove)?;
                Ok(if p == GlobalSetKey { Value::NIL } else { result })
            }
            LookupKey => { let events = self.key_events(args[1])?; self.lookup_keymap_key(args[0], &events, n == 3 && args[2] != Value::NIL) }
            UseLocalMap => {
                if args[0] != Value::NIL && self.resolve_keymap(args[0]).is_none() { return Err("keymap required".into()); }
                let index = self.buffer_index()?;
                *self.local_keymap(index) = args[0];
                Ok(Value::NIL)
            }
            _ => unreachable!(),
        }
    }
}

/// keyboard.c parse_modifiers_uncached: the modifier bits NAME's prefixes
/// spell (each an abbreviation followed by `-'), with `click' for a bare
/// mouse button or wheel event, and the byte NAME's base starts at.
pub(crate) fn parse_event_modifiers(name: &[u8]) -> (i64, usize) {
    const PREFIXES: [&str; 11] = ["A", "C", "H", "M", "S", "s", "drag", "down", "double", "triple", "up"];
    let bit = |prefix| Vm::key_event_modifier(prefix).expect("a modifier prefix");
    let (mut modifiers, mut i) = (0, 0);
    while i + 1 < name.len() {
        let Some(prefix) = PREFIXES.iter().find(|p| name[i..].starts_with(p.as_bytes()) && name.get(i + p.len()) == Some(&b'-'))
        else { break };
        modifiers |= bit(prefix);
        i += prefix.len() + 1;
    }
    let base = &name[i..];
    let clicks = bit("double") | bit("triple");
    if modifiers & (clicks | bit("down") | bit("drag")) == 0
        && base.len() == 7 && base.starts_with(b"mouse-") && base[6].is_ascii_digit()
        || modifiers & clicks == 0 && base.len() > 6 && base.starts_with(b"wheel-") {
        modifiers |= bit("click");
    }
    (modifiers, i)
}
