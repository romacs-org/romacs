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

#[cfg(test)]
thread_local! {
    /// The entries this thread's collections have visited (`visit_values`).
    static VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// The entries this thread's collections have visited.
#[cfg(test)]
pub(crate) fn visited() -> usize {
    VISITED.with(std::cell::Cell::get)
}

#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct LocalVars {
    entries: Vec<(Value, Value)>,
    version: u32,
}

impl LocalVars {
    /// Where SYMBOL's entry is, Ok, or would go, Err: a position good until
    /// the store next changes which symbols it holds.
    #[inline]
    pub(crate) fn find(&self, symbol: Value) -> std::result::Result<usize, usize> {
        self.entries.binary_search_by_key(&symbol.0, |&(s, _)| s.0)
    }
    /// SYMBOL's local value here, if it has one.
    #[inline]
    pub(crate) fn get(&self, symbol: Value) -> Option<Value> {
        self.find(symbol).ok().map(|i| self.entries[i].1)
    }
    pub(crate) fn contains(&self, symbol: Value) -> bool {
        self.find(symbol).is_ok()
    }
    /// The value of the entry at AT, which `find` gave.
    #[inline]
    pub(crate) fn value_at(&self, at: usize) -> Value {
        self.entries[at].1
    }
    /// Store VALUE in the entry at AT, which `find` gave.
    pub(crate) fn set_at(&mut self, at: usize, value: Value) {
        self.entries[at].1 = value;
    }
    /// Give SYMBOL a local VALUE at AT, where `find` said it would go.
    pub(crate) fn insert_at(&mut self, at: usize, symbol: Value, value: Value) {
        debug_assert_eq!(self.find(symbol), Err(at), "a fresh position");
        self.entries.insert(at, (symbol, value));
        self.version = self.version.wrapping_add(1);
    }
    /// Drop the entry at AT, which `find` gave.
    pub(crate) fn remove_at(&mut self, at: usize) {
        self.entries.remove(at);
        self.version = self.version.wrapping_add(1);
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
    /// Each (symbol, value), in slot order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Value, Value)> + '_ {
        self.entries.iter().copied()
    }
    /// Both halves of every entry: the symbol keeps an obarray's symbol
    /// alive while a buffer has a local of it, the value is the buffer's.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        #[cfg(test)]
        VISITED.with(|visited| visited.set(visited.get() + self.entries.len()));
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
    fn insert(locals: &mut LocalVars, slot: usize, value: Value) {
        let at = locals.find(symbol(slot)).unwrap_err();
        locals.insert_at(at, symbol(slot), value);
    }

    #[test]
    fn entries_stay_sorted_by_slot_and_unique() {
        let mut locals = LocalVars::default();
        for slot in [9, 3, 7, 1, 5] {
            insert(&mut locals, slot, Value::integer(slot as i64).unwrap());
        }
        assert!(locals.find(symbol(7)).is_ok(), "an existing local has a position");
        let slots: Vec<usize> = locals.iter().map(|(s, _)| s.index(7).unwrap() - 1).collect();
        assert_eq!(slots, [1, 3, 5, 7, 9]);
        assert_eq!(locals.get(symbol(7)), Some(Value::integer(7).unwrap()));
        assert_eq!(locals.get(symbol(4)), None);
        locals.set_at(locals.find(symbol(3)).unwrap(), Value::TRUE);
        assert_eq!(locals.get(symbol(3)), Some(Value::TRUE));
        locals.remove_at(locals.find(symbol(5)).unwrap());
        assert_eq!(locals.find(symbol(5)), Err(2));
        assert!(!locals.contains(symbol(5)) && locals.contains(symbol(9)));
        assert_eq!(locals.iter().count(), 4);
    }

    #[test]
    fn the_version_counts_which_symbols_have_locals_not_their_values() {
        let mut locals = LocalVars::default();
        let start = locals.version();
        insert(&mut locals, 2, Value::NIL);
        let made = locals.version();
        assert_ne!(made, start);
        locals.set_at(locals.find(symbol(2)).unwrap(), Value::TRUE);
        assert_eq!(locals.version(), made, "a store changes nothing");
        locals.remove_at(locals.find(symbol(2)).unwrap());
        assert_ne!(locals.version(), made);
        let removed = locals.version();
        locals.clear();
        assert_eq!(locals.version(), removed, "clearing nothing changes nothing");
        insert(&mut locals, 2, Value::NIL);
        locals.clear();
        assert!(locals.iter().next().is_none() && locals.version() != removed);
    }

    #[test]
    fn an_unbound_value_is_a_void_local() {
        let mut locals = LocalVars::default();
        insert(&mut locals, 4, Value::UNBOUND);
        assert!(locals.contains(symbol(4)));
        assert_eq!(locals.get(symbol(4)), Some(Value::UNBOUND));
    }

    #[test]
    fn a_visit_reaches_symbol_and_value() {
        let mut locals = LocalVars::default();
        insert(&mut locals, 1, Value::integer(10).unwrap());
        insert(&mut locals, 2, Value::integer(20).unwrap());
        let mut seen = Vec::new();
        locals.visit_values(&mut |v| seen.push(*v));
        assert_eq!(seen, [symbol(1), Value::integer(10).unwrap(), symbol(2), Value::integer(20).unwrap()]);
    }
}
