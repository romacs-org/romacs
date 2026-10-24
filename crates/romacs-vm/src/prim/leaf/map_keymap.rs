use super::*;
impl Leaf<'_> {
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
        let mut seen = HashSet::new();
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
        if matches!(self.vm.object_if(t.parent), Some(data::Object::CharTable(_))) { t.parent } else { Value::NIL }
    }
    pub(crate) fn map_table_validate(&self, value: Value) -> std::result::Result<(), Fault> {
        let Some(data::Object::CharTable(t)) = self.vm.object_if(value) else {
            return Err(Fault::Wrong("char-table-p", value));
        };
        t.validate_mapping().map_err(Fault::Error)
    }
    pub(crate) fn map_table_raw_default(&self, value: Value, index: usize) -> super::super::Out {
        let data::Object::CharTable(t) = self.vm.object_if(value).expect("rooted chartab") else {
            unreachable!()
        };
        t.get(index).map_err(Fault::Error)
    }
    pub(crate) fn map_cursor(&mut self, map: Value, internal: bool) -> Value {
        self.vm
            .allocate_object(data::Object::MapKeymapCursor(Box::new(
                crate::map_keymap::Cursor::new(map, internal),
            )))
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
