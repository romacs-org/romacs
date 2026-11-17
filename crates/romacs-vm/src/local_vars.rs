//! A buffer's own Lisp local variables: GNU's `local_var_alist`, kept in
//! the buffer (`PerBuffer`), so making, reading, setting and killing a
//! buffer's locals, and killing the buffer, write that buffer alone.
//!
//! The entries are (symbol, value) pairs sorted by the symbol's slot, so a
//! lookup is a binary search and an iteration runs in slot order. A value
//! of UNBOUND is a local binding that is void. `version` counts the changes
//! of which symbols have an entry, never of a value: a cache of where a
//! variable's value is (`Places`) holds while it stands.
use crate::Value;

#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct LocalVars {
    entries: Vec<(Value, Value)>,
    version: u32,
}

impl LocalVars {
    fn find(&self, symbol: Value) -> std::result::Result<usize, usize> {
        self.entries.binary_search_by_key(&symbol.0, |&(s, _)| s.0)
    }
    /// SYMBOL's local value here, if it has one.
    #[inline]
    pub(crate) fn get(&self, symbol: Value) -> Option<Value> {
        self.find(symbol).ok().map(|i| self.entries[i].1)
    }
    /// SYMBOL's local value here, to store into, if it has one.
    #[inline]
    pub(crate) fn get_mut(&mut self, symbol: Value) -> Option<&mut Value> {
        self.find(symbol).ok().map(|i| &mut self.entries[i].1)
    }
    pub(crate) fn contains(&self, symbol: Value) -> bool {
        self.find(symbol).is_ok()
    }
    /// Give SYMBOL a local VALUE here, unless it has one: whether it is new.
    pub(crate) fn insert(&mut self, symbol: Value, value: Value) -> bool {
        match self.find(symbol) {
            Ok(_) => false,
            Err(at) => {
                self.entries.insert(at, (symbol, value));
                self.version = self.version.wrapping_add(1);
                true
            }
        }
    }
    /// Drop SYMBOL's local here: its value, if it had one.
    pub(crate) fn remove(&mut self, symbol: Value) -> Option<Value> {
        let at = self.find(symbol).ok()?;
        self.version = self.version.wrapping_add(1);
        Some(self.entries.remove(at).1)
    }
    /// Drop every local.
    pub(crate) fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries = Vec::new();
            self.version = self.version.wrapping_add(1);
        }
    }
    /// The changes of which symbols have a local here.
    pub(crate) fn version(&self) -> u32 {
        self.version
    }
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    /// Each (symbol, value), in slot order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Value, Value)> + '_ {
        self.entries.iter().copied()
    }
    /// Both halves of every entry: the symbol keeps an obarray's symbol
    /// alive while a buffer has a local of it, the value is the buffer's.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for (symbol, value) in &mut self.entries {
            visit(symbol);
            visit(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(slot: usize) -> Value {
        Value::reference(slot + 1, 7)
    }

    #[test]
    fn entries_stay_sorted_by_slot_and_unique() {
        let mut locals = LocalVars::default();
        for slot in [9, 3, 7, 1, 5] {
            assert!(locals.insert(symbol(slot), Value::integer(slot as i64).unwrap()));
        }
        assert!(!locals.insert(symbol(7), Value::NIL), "an existing local keeps its value");
        let slots: Vec<usize> = locals.iter().map(|(s, _)| s.index(7).unwrap() - 1).collect();
        assert_eq!(slots, [1, 3, 5, 7, 9]);
        assert_eq!(locals.get(symbol(7)), Some(Value::integer(7).unwrap()));
        assert_eq!(locals.get(symbol(4)), None);
        *locals.get_mut(symbol(3)).unwrap() = Value::TRUE;
        assert_eq!(locals.get(symbol(3)), Some(Value::TRUE));
        assert_eq!(locals.remove(symbol(5)), Some(Value::integer(5).unwrap()));
        assert_eq!(locals.remove(symbol(5)), None);
        assert!(!locals.contains(symbol(5)) && locals.contains(symbol(9)));
        assert_eq!(locals.len(), 4);
    }

    #[test]
    fn the_version_counts_which_symbols_have_locals_not_their_values() {
        let mut locals = LocalVars::default();
        let start = locals.version();
        locals.insert(symbol(2), Value::NIL);
        let made = locals.version();
        assert_ne!(made, start);
        *locals.get_mut(symbol(2)).unwrap() = Value::TRUE;
        locals.insert(symbol(2), Value::NIL);
        assert_eq!(locals.version(), made, "a store or a second insert changes nothing");
        locals.remove(symbol(2));
        assert_ne!(locals.version(), made);
        let removed = locals.version();
        locals.clear();
        assert_eq!(locals.version(), removed, "clearing nothing changes nothing");
        locals.insert(symbol(2), Value::NIL);
        locals.clear();
        assert!(locals.len() == 0 && locals.version() != removed);
    }

    #[test]
    fn an_unbound_value_is_a_void_local() {
        let mut locals = LocalVars::default();
        locals.insert(symbol(4), Value::UNBOUND);
        assert!(locals.contains(symbol(4)));
        assert_eq!(locals.get(symbol(4)), Some(Value::UNBOUND));
    }

    #[test]
    fn a_visit_reaches_symbol_and_value() {
        let mut locals = LocalVars::default();
        locals.insert(symbol(1), Value::integer(10).unwrap());
        locals.insert(symbol(2), Value::integer(20).unwrap());
        let mut seen = Vec::new();
        locals.visit_values(&mut |v| seen.push(*v));
        assert_eq!(seen, [symbol(1), Value::integer(10).unwrap(), symbol(2), Value::integer(20).unwrap()]);
    }
}
