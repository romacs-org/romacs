//! What the reverse keymap lookup (`src/where_is.rs`) reads of the VM.
use super::{Leaf, Value, data};

impl Leaf<'_> {
    /// `current-global-map`.
    pub(crate) fn global_map(&mut self) -> Value {
        self.vm.ensure_keymaps();
        self.vm.slots[self.vm.names["#:vm2-global-keymap"]].value
    }
    /// A fresh `eql` hash table of ENTRIES, whose keys are distinct.
    pub(crate) fn eql_table(&mut self, entries: &[(Value, Value)]) -> Value {
        let table = self.vm.table_from_entries(crate::hash::Test::Eql, entries);
        self.vm.allocate_object(data::Object::HashTable(Box::new(table)))
    }
}
