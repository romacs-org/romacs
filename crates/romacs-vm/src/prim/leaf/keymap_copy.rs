//! keymap.c copy-keymap: sub-keymaps and menu-item cells are copied;
//! a symbol whose function is a keymap is not.
use super::*;

impl Leaf<'_> {
    pub(crate) fn copy_keymap(&mut self, keymap: Value, depth: usize) -> super::super::Out {
        if depth > 100 { return Err(Fault::Error("Possible infinite recursion when copying keymap".into())); }
        // get_keymap (keymap, 1, 0): a symbol's function cell, no autoloading.
        let keymap = self.vm.resolve_keymap(keymap).ok_or(Fault::Wrong("keymapp", keymap))?;
        let head = self.intern("keymap");
        let mut items = Vec::new();
        let mut tail = self.pair(keymap).map_or(Value::NIL, |(_, rest)| rest);
        while let Some((element, rest)) = self.pair(tail) {
            if element == head { break; }
            let copy = if self.kind(element) == Kind::CharTable {
                self.copy_keymap_char_table(element, depth + 1)?
            } else if self.kind(element) == Kind::Vector {
                let values = self.vector_values(element)?;
                let mut copies = Vec::with_capacity(values.len());
                for value in values { copies.push(self.copy_keymap_item(value, depth + 1)?); }
                self.vector(&copies)
            } else if let Some((car, cdr)) = self.pair(element) {
                if car == head { self.copy_keymap(element, depth + 1)? }
                else { let item = self.copy_keymap_item(cdr, depth + 1)?; self.cons(car, item) }
            } else { element };
            items.push(copy);
            tail = rest;
        }
        let mut copy = tail;
        for item in items.into_iter().rev() { copy = self.cons(item, copy); }
        Ok(self.cons(head, copy))
    }

    fn copy_keymap_char_table(&mut self, table: Value, depth: usize) -> super::super::Out {
        let Some(data::Object::CharTable(original)) = self.vm.object_if(table) else { unreachable!() };
        // map_char_table visits effective bindings, which a parent supplies too.
        if original.parent() != Value::NIL {
            return Err(Fault::Error("VM2 copy-keymap of a char-table with a parent is unsupported".into()));
        }
        let mut copy = (**original).clone();
        // Each entry is copied on its own: equal prefixes stay independent.
        let entries = copy.entry_values();
        let mut copies = Vec::with_capacity(entries.len());
        for value in entries { copies.push(self.copy_keymap_item(value, depth)?); }
        let mut next = copies.into_iter();
        copy.map_entries(&mut |value| if let Some(item) = next.next() { *value = item; });
        // Characters without an entry take the default; copy it once for them.
        if self.pair(copy.default()).is_some() {
            let default = self.copy_keymap_item(copy.default(), depth)?;
            copy.set_default(default);
        }
        Ok(self.vm.allocate_object(data::Object::CharTable(Box::new(copy))))
    }

    fn copy_keymap_item(&mut self, element: Value, depth: usize) -> super::super::Out {
        let Some((first, rest)) = self.pair(element) else { return Ok(element); };
        let (head, menu_item) = (self.intern("keymap"), self.intern("menu-item"));
        if first == menu_item {
            // (menu-item NAME BINDING . PROPS): copy the three spine cells.
            let Some((name, after_name)) = self.pair(rest) else { return Ok(self.cons(first, rest)); };
            let Some((binding, props)) = self.pair(after_name) else {
                let cell = self.cons(name, after_name);
                return Ok(self.cons(first, cell));
            };
            let binding = if self.pair(binding).is_some_and(|(car, _)| car == head) { self.copy_keymap(binding, depth)? } else { binding };
            let cell = self.cons(binding, props);
            let cell = self.cons(name, cell);
            return Ok(self.cons(first, cell));
        }
        if matches!(self.kind(first), Kind::String | Kind::Bytes) {
            // Old format: (STRING [HELP] . BINDING).
            let (help, binding) = match self.pair(rest) {
                Some((help, binding)) if matches!(self.kind(help), Kind::String | Kind::Bytes) => (Some(help), binding),
                _ => (None, rest),
            };
            let binding = if self.pair(binding).is_some_and(|(car, _)| car == head) { self.copy_keymap(binding, depth)? } else { binding };
            let tail = match help { Some(help) => self.cons(help, binding), None => binding };
            return Ok(self.cons(first, tail));
        }
        if first == head { return self.copy_keymap(element, depth); }
        Ok(element)
    }
}

impl Leaf<'_> {
    /// One `lookup-key' walk; Err((item, filter, definition)) names the
    /// menu filter it still needs.
    pub(crate) fn lookup_key_walk(&mut self, map: Value, key: Value, defaults: bool, filters: Vec<(Value, Value)>)
        -> std::result::Result<std::result::Result<Value, (Value, Value, Value)>, Fault> {
        self.vm.lookup_key_walk(map, key, defaults, filters).map_err(|e| {
            if e == "keymap required" { Fault::Wrong("keymapp", map) } else { Fault::Error(e) }
        })
    }
}
