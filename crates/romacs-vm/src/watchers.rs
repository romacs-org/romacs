//! GNU data.c variable watchers: the SYMBOL_TRAPPED_WRITE check and
//! notify_variable_watchers. Watchers run before the write they observe.
//! A native leaf subr runs in place, as GNU calls a subr directly; any other
//! watcher is funcalled, from whatever write point reached it.
use crate::*;

impl Vm {
    pub(crate) fn variable_trapped(&self, base: usize) -> bool {
        !self.trapped.is_empty() && self.trapped.contains(&base)
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
        let mut tail = self.symbol_property(symbol, property)?;
        let place = match place {
            Some(place) => place,
            None if self.local_if_set(base)? => {
                self.buffer_index()?;
                self.slots[self.buffer_slot].value
            }
            None => Value::NIL,
        };
        let newval = if newval == Value::UNBOUND { Value::NIL } else { newval };
        let operation = self.intern(operation);
        // GNU untraps the variable while its watchers run (no recursion).
        self.trapped.remove(&base);
        let mut result = Ok(());
        // The live list, its next link read after each call: a watcher
        // removed by an earlier one is not called.
        while let Ok((watcher, _)) = self.cons_pair(tail) {
            let args = [symbol, newval, operation, place];
            result = if prim::native_watcher(self, watcher).is_some() {
                prim::call_native_watcher(self, watcher, &args)
            } else {
                self.call_lisp_at_write(watcher, &args)
            };
            if result.is_err() { break; }
            tail = self.cons_pair(tail).map_or(Value::NIL, |(_, rest)| rest);
        }
        self.trapped.insert(base);
        result
    }
    /// Flocal_variable_if_set_p for the current buffer: a local binding, an
    /// automatically buffer-local variable, or a per-buffer C variable.
    fn local_if_set(&mut self, base: usize) -> Result<bool> {
        if matches!(self.forwarding(base), Some(crate::forwarded::Fwd::Buffer(_))) { return Ok(true); }
        if self.variable_binding_slot(base, false)? != base { return Ok(true); }
        Ok(self.local_table(base)?.is_some() && !self.slots[base].explicit_local)
    }
    /// calln (FUNCTION, ARGS...) at a write point (a variable watcher, a
    /// `defalias-fset-function'). The writer's Values (an engine's
    /// registers, a leaf's locals) are not rooted there, so collection is
    /// inhibited until FUNCTION returns, as GNU's is in compilation and
    /// loading here; the engine grows its code cache for functions FUNCTION
    /// compiled (`lower_function'). A non-local exit is left in
    /// `propagated_escape' for the writer; an escape already pending there
    /// survives a FUNCTION that returns.
    pub(crate) fn call_lisp_at_write(&mut self, watcher: Value, args: &[Value]) -> Result<()> {
        let pending = self.propagated_escape.take();
        let hooked = std::mem::take(&mut self.propagated_escape_hooked);
        let depth = self.propagated_eval_depth.take();
        let threshold = std::mem::replace(&mut self.gc_threshold, usize::MAX);
        let result = self.reader_call(watcher, args);
        self.gc_threshold = threshold;
        if result.is_ok() {
            self.propagated_escape = pending;
            self.propagated_escape_hooked = hooked;
            self.propagated_eval_depth = depth;
        }
        result.map(|_| ())
    }
}
