//! GNU data.c variable watchers: the SYMBOL_TRAPPED_WRITE check and
//! notify_variable_watchers. Watchers run before the write they observe.
//! A native leaf subr runs in place, as GNU calls a subr directly; any other
//! watcher is funcalled, from whatever write point reached it.
//!
//! A native call keeps Values in Rust locals, so it cannot run Lisp that may
//! collect. Its writes of a trapped variable wait in `deferred_writes', the
//! store with its notifications, until the engine that called it has published
//! the call's outcome (`threaded::drained'). So those watchers run after the
//! rest of the call, before the next instruction, and may collect and throw.
use crate::*;
use crate::gc::RootFrame;
use crate::prim::Fault;

/// What a native write does: the watchers are told, then its store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteKind {
    /// GNU `Fset' of VALUE.
    Set,
    /// GNU `set_internal' of VALUE with SET_INTERNAL_BIND: `let' reported.
    Bound,
    /// GNU `Fset_default' of VALUE.
    SetDefault,
    /// `defvaralias': the variable becomes an alias of the symbol VALUE.
    Alias,
    /// A kept local: `makunbound' reported with VALUE, and no store.
    Report,
    /// `makunbound' reported with nil, then the current buffer's binding goes.
    ClearLocal,
    /// A permanent hook's filter: `makunbound' reported with VALUE, then stored.
    StoreLocal,
}
romacs_snapshot::snapshot_enum!(WriteKind {
    WriteKind::Set => (),
    WriteKind::Bound => (),
    WriteKind::SetDefault => (),
    WriteKind::Alias => (),
    WriteKind::Report => (),
    WriteKind::ClearLocal => (),
    WriteKind::StoreLocal => (),
});
const WRITE_KINDS: [WriteKind; 7] = [WriteKind::Set, WriteKind::Bound, WriteKind::SetDefault, WriteKind::Alias,
    WriteKind::Report, WriteKind::ClearLocal, WriteKind::StoreLocal];
/// A write of the variable at SLOT that its watchers hear.
#[derive(Clone, Copy)]
pub(crate) struct NativeWrite {
    pub(crate) kind: WriteKind,
    pub(crate) slot: usize,
    pub(crate) value: Value,
}
romacs_snapshot::snapshot_struct!(NativeWrite { kind, slot, value });
impl NativeWrite {
    pub(crate) fn set(slot: usize, value: Value) -> Self {
        Self { kind: WriteKind::Set, slot, value }
    }
}

impl Vm {
    /// WRITE from native code: at once when nothing can hear it or no engine
    /// will complete this call; else queued, as is whatever follows a queued
    /// write, so the writes keep their order.
    pub(crate) fn write_from_native(&mut self, write: NativeWrite) -> std::result::Result<(), Fault> {
        let heard = self.variable_trapped(self.indirect_variable(write.slot));
        if !self.deferred_writes.is_empty() || (heard && self.running_engines > self.suspended_code.len()) {
            let buffer = self.slots[self.buffer_slot].value;
            self.deferred_writes.push((write, buffer));
            return Ok(());
        }
        if !heard && write.kind == WriteKind::Set {
            return self.assign_variable(write.slot, write.value).map_err(Fault::Error);
        }
        self.perform_write(write)
    }
    /// The queue, for a drain whose watchers may collect: a frame holding each
    /// write's symbol, value, buffer and kind, and how many writes.
    pub(crate) fn take_deferred(&mut self) -> Option<(RootFrame, usize)> {
        if self.deferred_writes.is_empty() { return None; }
        let writes = std::mem::take(&mut self.deferred_writes);
        let values: Vec<Value> = writes.iter().flat_map(|(write, buffer)| {
            let kind = WRITE_KINDS.iter().position(|&kind| kind == write.kind).expect("every kind is listed");
            [Value::reference(write.slot + 1, 7), write.value, *buffer, Value::integer(kind as i64).expect("a small index")]
        }).collect();
        Some((self.root_frame(&values), writes.len()))
    }
    /// Write number I of a frame `take_deferred' made, in the buffer it was
    /// made in: the buffer current now is current again after, or a watcher's
    /// own `set-buffer' stands when that was the same one. If a watcher has
    /// killed that buffer the write runs in the current one, as GNU's C goes on
    /// with `current_buffer', but what it meant for a binding of the killed
    /// buffer has nothing left to do.
    pub(crate) fn perform_deferred(&mut self, frame: &RootFrame, i: usize) -> std::result::Result<(), Fault> {
        let slot = self.rooted(frame, 4 * i).payload() as usize - 1;
        let kind = WRITE_KINDS[self.rooted(frame, 4 * i + 3).integer_value().expect("a kind's index") as usize];
        let write = NativeWrite { kind, slot, value: self.rooted(frame, 4 * i + 1) };
        let buffer = self.rooted(frame, 4 * i + 2);
        if !self.live_buffer_value(buffer) {
            let kind = if matches!(kind, WriteKind::ClearLocal | WriteKind::StoreLocal) { WriteKind::Report } else { kind };
            return self.perform_write(NativeWrite { kind, ..write });
        }
        let current = self.slots[self.buffer_slot].value;
        if buffer == current {
            return self.perform_write(write);
        }
        let saved = self.root_frame(&[current]);
        self.slots[self.buffer_slot].value = buffer;
        let outcome = self.perform_write(write);
        let [current] = self.release_frame(saved);
        if self.live_buffer_value(current) { self.slots[self.buffer_slot].value = current; }
        outcome
    }
    /// Tell the watchers, then store. A watcher's escape waits in
    /// `propagated_escape' behind `Fault::Error'; a store's fault is itself.
    fn perform_write(&mut self, write: NativeWrite) -> std::result::Result<(), Fault> {
        let NativeWrite { kind, slot, value } = write;
        if kind == WriteKind::Set {
            return self.set_variable_value(slot, value).map_err(Fault::Error);
        }
        // The value and the buffer are rooted across the watchers, read back.
        let frame = self.root_frame(&[value, self.slots[self.buffer_slot].value]);
        let outcome = self.perform_rooted(kind, slot, &frame);
        self.drop_frame(frame);
        outcome
    }
    fn perform_rooted(&mut self, kind: WriteKind, slot: usize, frame: &RootFrame) -> std::result::Result<(), Fault> {
        let reported = |vm: &mut Vm, value: Value, operation: &str, place: Value| {
            vm.notify_watchers_at(slot, value, operation, Some(place)).map_err(Fault::Error)
        };
        match kind {
            WriteKind::Set => unreachable!("`perform_write' makes it"),
            WriteKind::Bound => {
                self.notify_watchers(slot, self.rooted(frame, 0), "let").map_err(Fault::Error)?;
                self.assign_variable(slot, self.rooted(frame, 0)).map_err(Fault::Error)
            }
            WriteKind::SetDefault => {
                reported(self, self.rooted(frame, 0), "set", Value::NIL)?;
                // GNU set_default_internal, SYMBOL_FORWARDED arm (data.c:2085).
                let Some(fwd) = self.native_forwarding(slot) else {
                    self.slots[slot].value = self.rooted(frame, 0);
                    return Ok(());
                };
                // A global variable's set_internal reports again.
                if self.forwarding(slot).is_some_and(|fwd| !matches!(fwd, crate::forwarded::Fwd::Buffer(_) | crate::forwarded::Fwd::Fence(_))) {
                    reported(self, self.rooted(frame, 0), "set", Value::NIL)?;
                }
                self.set_forwarded_default(slot, fwd, self.rooted(frame, 0), crate::forwarded::SetFlag::Set)
            }
            WriteKind::Alias => {
                // eval.c:713: NEW's watchers see the alias before it is made.
                reported(self, self.rooted(frame, 0), "defvaralias", Value::NIL)?;
                prim::alias_now(self, slot, self.rooted(frame, 0))
            }
            WriteKind::Report => reported(self, self.rooted(frame, 0), "makunbound", self.rooted(frame, 1)),
            WriteKind::ClearLocal => {
                reported(self, Value::NIL, "makunbound", self.rooted(frame, 1))?;
                prim::clear_local_slot(self, slot, self.rooted(frame, 1))
            }
            WriteKind::StoreLocal => {
                reported(self, self.rooted(frame, 0), "makunbound", self.rooted(frame, 1))?;
                self.assign_variable(slot, self.rooted(frame, 0)).map_err(Fault::Error)
            }
        }
    }
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
        let tail = self.symbol_property(symbol, property)?;
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
        // A watcher may collect only for a writer that rooted its own frame
        // (an engine write, `threaded::watched_write'); any other native
        // writer may keep Values in locals, so it holds collection.
        let _unwind = self.unwind_mark();
        // GNU untraps the variable while its watchers run (no recursion),
        // and traps it again as they end; a panic out of them, at the mark.
        self.trapped.remove(&base);
        self.retrap.push(base);
        let held = usize::from(!self.rooted_write);
        self.gc_holds += held;
        // The list, the value and the place are rooted and read back after
        // each watcher. The live list's next link is read after each call:
        // a watcher removed by an earlier one is not called.
        let frame = self.root_frame(&[tail, newval, place]);
        let mut result = Ok(());
        while let Ok((watcher, _)) = self.cons_pair(self.rooted(&frame, 0)) {
            let args = [symbol, self.rooted(&frame, 1), operation, self.rooted(&frame, 2)];
            result = if prim::native_watcher(self, watcher).is_some() {
                prim::call_native_watcher(self, watcher, &args)
            } else {
                self.call_lisp_at_write(watcher, &args)
            };
            if result.is_err() { break; }
            let rest = self.cons_pair(self.rooted(&frame, 0)).map_or(Value::NIL, |(_, rest)| rest);
            self.set_rooted(&frame, 0, rest);
        }
        self.drop_frame(frame);
        self.gc_holds -= held;
        let owed = self.retrap.pop();
        debug_assert_eq!(owed, Some(base), "the re-trap owed by this call");
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
    /// `defalias-fset-function'); the engine grows its code cache for
    /// functions FUNCTION compiled (`lower_function'). A non-local exit is
    /// left in `propagated_escape' for the writer; an escape already pending
    /// there survives a FUNCTION that returns, rooted across it.
    pub(crate) fn call_lisp_at_write(&mut self, watcher: Value, args: &[Value]) -> Result<()> {
        let mut pending = self.propagated_escape.take();
        let hooked = std::mem::take(&mut self.propagated_escape_hooked);
        let depth = self.propagated_eval_depth.take();
        let mut held = vec![];
        if let Some(escape) = &mut pending { escape.visit_values(&mut |v| held.push(*v)); }
        let frame = self.root_frame(&held);
        let result = self.reader_call(watcher, args);
        let mut i = 0;
        if let Some(escape) = &mut pending { escape.visit_values(&mut |v| { *v = self.rooted(&frame, i); i += 1; }); }
        self.drop_frame(frame);
        if result.is_ok() {
            self.propagated_escape = pending;
            self.propagated_escape_hooked = hooked;
            self.propagated_eval_depth = depth;
        }
        result.map(|_| ())
    }
    /// xdisp.c safe_calln: FUNCTION's value, or nil when it exits
    /// non-locally (the exit is dropped; an escape already pending
    /// survives). Called where `call_lisp_at_write' may be; its callers are
    /// leaves, whose unsuspended engine holds collection.
    pub(crate) fn safe_call(&mut self, function: Value, args: &[Value]) -> Value {
        let pending = (self.propagated_escape.take(), std::mem::take(&mut self.propagated_escape_hooked), self.propagated_eval_depth.take());
        let result = self.reader_call(function, args);
        (self.propagated_escape, self.propagated_escape_hooked, self.propagated_eval_depth) = pending;
        result.unwrap_or(Value::NIL)
    }
}
