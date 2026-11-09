//! GNU category.c: category tables are char-tables of purpose
//! `category-table` whose entries are 128-bit category sets (bool-vectors).
//! Extra slot 0 holds the 95 category docstrings (` ' to `~'), slot 1 the
//! `equal' hash table that interns category sets.
use super::*;

const STANDARD: &str = "#:vm2-standard-category-table";

impl Leaf<'_> {
    fn category_set(&mut self) -> Value {
        self.vm.allocate_object(data::Object::BoolVector(Box::new(
            data::BoolVector::new(128, false).expect("small bool-vector"))))
    }

    /// Fmake_category_table: empty sets as the default and in each top slot.
    pub(crate) fn make_category_table(&mut self) -> Value {
        let purpose = self.intern("category-table");
        // syms_of_category's (put 'category-table 'char-table-extra-slots 2),
        // deferred with the first table to keep VM startup heap-free.
        let property = self.intern("char-table-extra-slots");
        if self.vm.symbol_property(purpose, property).unwrap_or(Value::NIL) == Value::NIL {
            let _ = self.vm.put_symbol_property(purpose, property, Value::integer(2).unwrap());
        }
        let mut table = crate::char_table::CharTable::new(purpose);
        table.initialize(2, Value::NIL).expect("two extra slots");
        table.default = self.category_set();
        for slot in 0..crate::char_table::CharTable::LEN / 65536 {
            let set = self.category_set();
            table.set_range(slot * 65536, slot * 65536 + 65535, set).expect("top-level slot");
        }
        table.extras[0] = self.vector(&[Value::NIL; 95]);
        self.vm.allocate_object(data::Object::CharTable(Box::new(table)))
    }

    pub(crate) fn standard_category_table(&mut self) -> Value {
        let slot = self.vm.slot(STANDARD);
        if self.vm.slots[slot].value == Value::UNBOUND || self.vm.slots[slot].value == Value::NIL {
            let table = self.make_category_table();
            self.vm.slots[slot].value = table;
        }
        self.vm.slots[slot].value
    }

    pub(crate) fn is_category_table(&mut self, value: Value) -> bool {
        let purpose = self.intern("category-table");
        matches!(self.vm.object_if(value), Some(data::Object::CharTable(t)) if t.purpose == purpose)
    }

    /// The current buffer's table (BVAR category_table).
    pub(crate) fn current_category_table(&mut self) -> super::super::Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let table = match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.category_table,
            data::Object::ByteBuffer(b) => b.category_table,
            _ => unreachable!("buffer_index validates"),
        };
        Ok(if table == Value::NIL { self.standard_category_table() } else { table })
    }

    pub(crate) fn set_current_category_table(&mut self, table: Value) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match self.vm.objects.entry_mut(index) {
            data::Object::Buffer(b) => b.category_table = table,
            data::Object::ByteBuffer(b) => b.category_table = table,
            _ => unreachable!("buffer_index validates"),
        }
        Ok(())
    }

    /// CHAR_HAS_CATEGORY of C in TABLE.
    pub(crate) fn char_has_category(&mut self, table: Value, c: usize, category: u8) -> std::result::Result<bool, Fault> {
        let set = self.char_table_element(table, c)?;
        Ok(self.bool_vector(set).ok().and_then(|bits| bits.get(category as usize)).unwrap_or(false))
    }

    /// check_category_table: nil is the current buffer's table.
    pub(crate) fn check_category_table(&mut self, table: Value) -> super::super::Out {
        if table == Value::NIL { return self.current_category_table(); }
        if self.is_category_table(table) { Ok(table) } else { Err(Fault::Wrong("category-table-p", table)) }
    }

    /// TABLE, checked, for reading: private or frozen.
    pub(crate) fn category_table_ref(&self, table: Value) -> std::result::Result<&crate::char_table::CharTable, Fault> {
        match self.vm.object(table).map_err(Fault::Error)? {
            data::Object::CharTable(t) => Ok(t),
            _ => unreachable!("checked category table"),
        }
    }

    /// TABLE, checked, for writing; a frozen table refuses (`frozen.rs`).
    pub(crate) fn category_table(&mut self, table: Value) -> std::result::Result<&mut crate::char_table::CharTable, Fault> {
        match self.object_mut(table)? {
            data::Object::CharTable(t) => Ok(t),
            _ => unreachable!("checked category table"),
        }
    }

    pub(crate) fn category_docstring(&mut self, table: Value, category: usize) -> super::super::Out {
        let docs = self.category_table_ref(table)?.extras[0];
        self.vector_get(docs, Value::integer(category as i64 - 32).unwrap())
    }

    pub(crate) fn set_category_docstring(&mut self, table: Value, category: usize, doc: Value) -> std::result::Result<(), Fault> {
        let docs = self.category_table_ref(table)?.extras[0];
        self.vector_set(docs, Value::integer(category as i64 - 32).unwrap(), doc).map(|_| ())
    }

    /// hash_get_category_set: one shared set per distinct contents.
    fn intern_category_set(&mut self, table: Value, set: Value) -> super::super::Out {
        let mut hash = self.category_table(table)?.extras[1];
        if hash == Value::NIL {
            hash = self.vm.allocate_object(data::Object::HashTable(Box::new(
                crate::hash::HashTable::new(crate::hash::Test::Equal, crate::hash::Weakness::None, 65))));
            self.category_table(table)?.extras[1] = hash;
        }
        if let Some((key, _)) = self.vm.completion_hash_find(hash, set)? { return Ok(key); }
        self.vm.declaration_hash_put(hash, set, Value::NIL).map_err(Fault::Error)?;
        Ok(set)
    }

    /// Fmodify_category_entry over [START, END], one slot range at a time.
    pub(crate) fn modify_category_entry(&mut self, table: Value, start: usize, end: usize, category: usize, add: bool) -> std::result::Result<(), Fault> {
        let mut start = start;
        while start <= end {
            let (set, _, to) = self.category_table(table)?.span(start).map_err(Fault::Error)?;
            let to = to.min(end);
            let mut bits = self.bool_vector(set)?;
            if bits.get(category) != Some(add) {
                bits.set(category, add);
                let copy = self.bool_vector_from(bits);
                let set = self.intern_category_set(table, copy)?;
                self.category_table(table)?.set_range(start, to, set).map_err(Fault::Error)?;
            }
            start = to + 1;
        }
        Ok(())
    }

    /// copy_category_table: fresh sets everywhere, a fresh docstring vector.
    pub(crate) fn copy_category_table(&mut self, table: Value) -> super::super::Out {
        let mut copy = self.category_table_ref(table)?.clone();
        let mut entries = Vec::new();
        copy.map_entries(&mut |value| entries.push(*value));
        let mut copies = Vec::with_capacity(entries.len());
        for value in entries {
            copies.push(match self.bool_vector(value) { Ok(bits) => self.bool_vector_from(bits), Err(_) => value });
        }
        let mut next = copies.into_iter();
        copy.map_entries(&mut |value| if let Some(item) = next.next() { *value = item; });
        if let Ok(bits) = self.bool_vector(copy.default) { copy.default = self.bool_vector_from(bits); }
        let docs = self.vector_values(copy.extras[0])?;
        copy.extras[0] = self.vector(&docs);
        Ok(self.vm.allocate_object(data::Object::CharTable(Box::new(copy))))
    }
}
