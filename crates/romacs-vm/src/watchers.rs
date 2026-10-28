//! GNU data.c variable watchers: the SYMBOL_TRAPPED_WRITE check and
//! notify_variable_watchers. Watchers run before the write they observe.
//! Only native leaf subrs are accepted as watchers so far; see the
//! daily-driver campaign's design/variable-watchers.md.
use crate::*;

impl Vm {
    pub(crate) fn variable_trapped(&self, base: usize) -> bool {
        self.trapped.contains(&base)
    }
    pub(crate) fn set_variable_trapped(&mut self, base: usize, trapped: bool) {
        if trapped { self.trapped.insert(base); } else { self.trapped.remove(&base); }
    }
    /// Call each watcher of SLOT's base variable with (SYMBOL NEWVAL
    /// OPERATION WHERE), WHERE being the buffer of a buffer-local binding.
    /// PLACE overrides WHERE (set-default and makunbound report nil,
    /// kill-local-variable the current buffer).
    pub(crate) fn notify_watchers(&mut self, slot: usize, newval: Value, operation: &str) -> Result<()> {
        self.notify_watchers_at(slot, newval, operation, None)
    }
    pub(crate) fn notify_watchers_at(&mut self, slot: usize, newval: Value, operation: &str, place: Option<Value>) -> Result<()> {
        let base = self.indirect_variable(slot);
        if !self.trapped.contains(&base) { return Ok(()); }
        let symbol = Value::reference(base + 1, 7);
        let property = self.intern("watchers");
        let mut watchers = Vec::new();
        let mut tail = self.symbol_property(symbol, property)?;
        while let Ok((watcher, rest)) = self.cons_pair(tail) {
            watchers.push(watcher);
            tail = rest;
        }
        let place = match place {
            Some(place) => place,
            None => {
                let local = self.variable_binding_slot(base, false)?;
                if local != base { self.slots[self.buffer_slot].value } else { Value::NIL }
            }
        };
        let newval = if newval == Value::UNBOUND { Value::NIL } else { newval };
        let operation = self.intern(operation);
        // GNU untraps the variable while its watchers run (no recursion).
        self.trapped.remove(&base);
        let mut result = Ok(());
        for watcher in watchers {
            result = prim::call_native_watcher(self, watcher, &[symbol, newval, operation, place]);
            if result.is_err() { break; }
        }
        self.trapped.insert(base);
        result
    }
}
