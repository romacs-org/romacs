use super::*;
impl Leaf<'_> {
    fn unicode_property_table(&self, table: Value) -> Option<&crate::char_table::CharTable> {
        let data::Object::CharTable(table) = self.vm.object_if(table)? else {
            return None;
        };
        self.vm.is_uniprop_table(table).then_some(table)
    }
    pub(crate) fn is_uniprop(&self, table: Value) -> bool {
        self.unicode_property_table(table).is_some()
    }
    /// chartab.c Foptimize_char_table: collapse every sub-table whose slots
    /// are all values equal (or `eq' when EQ) to the first, children first,
    /// then reset the ASCII cache.
    pub(crate) fn optimize_char_table(&mut self, table: Value, eq: bool) -> std::result::Result<(), Fault> {
        let root = self.map_table_root(table);
        let slots = root.lock().unwrap().slots.clone();
        for (i, slot) in slots.into_iter().enumerate() {
            if let char_table::CharTableSlot::Node(child) = slot {
                let optimized = self.optimize_node(&child, eq)?;
                root.lock().unwrap().slots[i] = optimized;
            }
        }
        self.uniprop_reset_ascii(table);
        Ok(())
    }
    /// optimize_sub_char_table: NODE's slot in its parent after optimizing.
    fn optimize_node(&self, node: &char_table::CharTableNodeHandle, eq: bool) -> std::result::Result<char_table::CharTableSlot, Fault> {
        let mut slots = node.lock().unwrap().slots.clone();
        for slot in &mut slots {
            if let char_table::CharTableSlot::Node(child) = slot {
                *slot = self.optimize_node(&child.clone(), eq)?;
            }
        }
        node.lock().unwrap().slots = slots.clone();
        let same = |a: Value, b: Value| -> std::result::Result<bool, Fault> {
            if eq { return Ok(a == b); }
            self.equal(a, b).map_err(|e| match e {
                crate::hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
                crate::hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
                crate::hash::HashFault::Error(m, _) => Fault::Error(m.into()),
                _ => Fault::Error("equal failed".into()),
            })
        };
        let char_table::CharTableSlot::Value(first) = slots[0] else {
            return Ok(char_table::CharTableSlot::Node(node.clone()));
        };
        for slot in &slots[1..] {
            let char_table::CharTableSlot::Value(this) = *slot else {
                return Ok(char_table::CharTableSlot::Node(node.clone()));
            };
            if !same(this, first)? {
                return Ok(char_table::CharTableSlot::Node(node.clone()));
            }
        }
        Ok(char_table::CharTableSlot::Value(first))
    }
    /// set_char_table_ascii (table, char_table_ascii (table)).
    pub(crate) fn uniprop_reset_ascii(&mut self, table: Value) {
        let index = table.index_if(0).unwrap() - 1;
        if let data::Object::CharTable(t) = &mut self.vm.objects[index] {
            t.ascii = char_table::CharTable::ascii_from_root(&t.root);
        }
    }
    pub(crate) fn map_table_has_decoder(&self, table: Value) -> bool {
        self.unicode_property_table(table)
            .is_some_and(|table| table.extras[1].integer_value() == Some(0))
    }
    pub(crate) fn map_table_decode(&self, table: Value, value: Value) -> super::super::Out {
        let Some(data::Object::CharTable(table)) = self.vm.object_if(table) else {
            unreachable!()
        };
        let Some(data::Object::Vector(values)) =
            table.extras.get(4).and_then(|&v| self.vm.object_if(v))
        else {
            return Ok(value);
        };
        let index = value
            .integer_value()
            .ok_or(Fault::Wrong("integerp", value))?;
        Ok(usize::try_from(index)
            .ok()
            .and_then(|index| values.get(index))
            .copied()
            .unwrap_or(value))
    }
    pub(crate) fn map_table_slot(
        &mut self,
        table: Value,
        node: &char_table::CharTableNodeHandle,
        index: usize,
    ) -> std::result::Result<char_table::CharTableSlot, Fault> {
        let slot = node.lock().unwrap().slots[index].clone();
        let char_table::CharTableSlot::Value(value) = slot else {
            return Ok(slot);
        };
        if self.unicode_property_table(table).is_none() {
            return Ok(slot);
        }
        let Some(codes) = self.vm.uniprop_compressed(value) else {
            return Ok(slot);
        };
        let (depth, from) = {
            let node = node.lock().unwrap();
            (
                node.depth,
                node.min_char + index * char_table::CharTableNode::WIDTHS[node.depth],
            )
        };
        if depth != 2 {
            return Err(Fault::Error(
                "compressed Unicode property requires a 128-character block".into(),
            ));
        }
        let slots = char_table::uniprop_block(&codes)
            .map_err(Fault::Error)?
            .into_iter()
            .map(char_table::CharTableSlot::Value)
            .collect();
        let expanded = char_table::CharTableSlot::Node(std::sync::Arc::new(std::sync::Mutex::new(
            char_table::CharTableNode {
                depth: 3,
                min_char: from,
                slots,
            },
        )));
        node.lock().unwrap().slots[index] = expanded.clone();
        let table = table.index_if(0).unwrap() - 1;
        let data::Object::CharTable(table) = &mut self.vm.objects[table] else {
            unreachable!()
        };
        table.ascii = char_table::CharTable::ascii_from_root(&table.root);
        Ok(expanded)
    }
    /// GNU sub_char_table_set's uniprop_table_uncompress: expand C's
    /// compressed block in place before a write.
    pub(crate) fn uniprop_expand(&mut self, table: Value, c: usize) -> std::result::Result<(), Fault> {
        let mut node = self.map_table_root(table);
        loop {
            let (depth, at) = {
                let current = node.lock().unwrap();
                (current.depth, (c - current.min_char) / char_table::CharTableNode::WIDTHS[current.depth])
            };
            if depth == 2 {
                return self.map_table_slot(table, &node, at).map(|_| ());
            }
            let slot = node.lock().unwrap().slots[at].clone();
            match slot {
                char_table::CharTableSlot::Node(child) => node = child,
                char_table::CharTableSlot::Value(_) => return Ok(()),
            }
        }
    }
    /// GNU get_keymap: non-loading predicates recognize keymap autoloads;
    /// loading callers return the descriptor for a traced T2 call.
    pub(crate) fn map_resolve(
        &self,
        original: Value,
        load: bool,
    ) -> std::result::Result<Option<(Value, bool)>, Fault> {
        if original == Value::NIL {
            return Ok(None);
        }
        let mut value = original;
        let mut seen = HashSet::default();
        while self.kind(value) == Kind::Symbol && value != Value::NIL {
            if !seen.insert(value.0) {
                return Err(Fault::CyclicFunctionIndirection(original));
            }
            value = self.symbol_function(value)?;
        }
        if let Some((head, _)) = self.pair(value) {
            if self.map_is_head(head) {
                return Ok(Some((value, false)));
            }
            if self.kind(original) == Kind::Symbol && self.obarray_lookup("autoload") == Some(head)
            {
                let mut tail = value;
                for _ in 0..4 {
                    tail = self.pair(tail).map_or(Value::NIL, |p| p.1);
                }
                if self.pair(tail).is_some_and(|p| self.map_is_head(p.0)) {
                    return Ok(Some((if load { value } else { original }, load)));
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn map_is_head(&self, value: Value) -> bool {
        self.vm
            .names
            .get("keymap")
            .is_some_and(|&n| value == Value::reference(n + 1, 7))
    }
    pub(crate) fn map_table_root(&self, value: Value) -> char_table::CharTableNodeHandle {
        let data::Object::CharTable(t) = self.vm.object_if(value).expect("rooted chartab") else {
            unreachable!()
        };
        t.root.clone()
    }
    pub(crate) fn map_table_default(&self, value: Value) -> Value {
        let data::Object::CharTable(t) = self.vm.object_if(value).expect("rooted chartab") else {
            unreachable!()
        };
        t.default
    }
    pub(crate) fn map_table_parent(&self, value: Value) -> Value {
        let data::Object::CharTable(t) = self.vm.object_if(value).expect("rooted chartab") else {
            unreachable!()
        };
        if matches!(
            self.vm.object_if(t.parent),
            Some(data::Object::CharTable(_))
        ) {
            t.parent
        } else {
            Value::NIL
        }
    }
    pub(crate) fn map_table_validate(&self, value: Value) -> std::result::Result<(), Fault> {
        let Some(data::Object::CharTable(t)) = self.vm.object_if(value) else {
            return Err(Fault::Wrong("char-table-p", value));
        };
        t.validate_mapping().map_err(Fault::Error)
    }
    fn map_table_entry(&mut self, table: Value, index: usize) -> super::super::Out {
        let mut node = self.map_table_root(table);
        loop {
            let at = {
                let current = node.lock().unwrap();
                (index - current.min_char) / char_table::CharTableNode::WIDTHS[current.depth]
            };
            match self.map_table_slot(table, &node, at)? {
                char_table::CharTableSlot::Value(value) => return Ok(value),
                char_table::CharTableSlot::Node(child) => node = child,
            }
        }
    }
    pub(crate) fn map_table_first(&mut self, table: Value) -> super::super::Out {
        self.map_table_entry(table, 0)
    }
    pub(crate) fn map_table_raw_default(&mut self, table: Value, index: usize) -> super::super::Out {
        let value = self.map_table_entry(table, index)?;
        Ok(if value == Value::NIL { self.map_table_default(table) } else { value })
    }
    pub(crate) fn map_cursor(&mut self, map: Value, internal: bool) -> Value {
        self.vm
            .allocate_object(data::Object::MapKeymapCursor(Box::new(
                crate::map_keymap::Cursor::new(map, internal),
            )))
    }
    pub(crate) fn char_table_cursor(&mut self, table: Value) -> super::super::Out {
        let cursor = crate::map_keymap::Cursor::char_table(self, table)?;
        Ok(self
            .vm
            .allocate_object(data::Object::MapKeymapCursor(Box::new(cursor))))
    }
    pub(crate) fn map_cursor_next(
        &mut self,
        value: Value,
    ) -> std::result::Result<crate::map_keymap::Advance, Fault> {
        let index = value.index(0).expect("cursor") - 1;
        // Leaf calls cannot collect. Moving the cursor out avoids exposing Vm
        // to its state machine or retaining an arena borrow during allocation.
        let data::Object::MapKeymapCursor(cursor) = &self.vm.objects[index] else {
            unreachable!()
        };
        let mut cursor = cursor.clone();
        let next = cursor.next(self)?;
        self.vm.objects[index] = data::Object::MapKeymapCursor(cursor);
        Ok(next)
    }
}
