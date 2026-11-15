//! fns.c `maphash`'s walk (`DOHASH_SAFE`): a hash table's used slots in order,
//! read one at a time, so a callback between two reads may change the table.
use super::{Fault, Leaf, Value};

impl Leaf<'_> {
    /// TABLE's first used slot at or after FROM, with its key and value;
    /// None past the last.
    pub(crate) fn hash_slot_from(&self, table: Value, from: usize) -> std::result::Result<Option<(usize, Value, Value)>, Fault> {
        let Some(crate::data::Object::HashTable(t)) = self.vm.object_if(table) else { return Err(Fault::Wrong("hash-table-p", table)) };
        Ok(t.next_used(from).and_then(|slot| t.entry(slot).map(|(key, value)| (slot, key, value))))
    }
}
