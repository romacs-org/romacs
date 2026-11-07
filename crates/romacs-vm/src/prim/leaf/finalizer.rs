//! alloc.c finalizers: objects whose function runs once, after a
//! collection finds them unreachable (gc.rs dooms them).
use super::*;

impl Leaf<'_> {
    /// Fmake_finalizer.
    pub(crate) fn make_finalizer(&mut self, function: Value) -> Value {
        let finalizer = self.vm.allocate_object(data::Object::Finalizer(function));
        self.vm.finalizers.push(finalizer);
        finalizer
    }
    /// Whether a collection ended since the last call: the caller, not a
    /// later poll, now owns running `post-gc-hook' for it.
    pub(crate) fn take_post_gc(&mut self) -> bool {
        std::mem::take(&mut self.vm.post_gc_due)
    }
    /// alloc.c inhibit_garbage_collection, and its unwinding when not ON.
    pub(crate) fn hold_collection(&mut self, on: bool) {
        if on { self.vm.gc_holds += 1; } else { self.vm.gc_holds -= 1; }
    }
    /// run_finalizers' step: the function of the oldest doomed finalizer
    /// that has one, the finalizer emptied so that it never runs again, or
    /// nil. One at a time, so a `throw' out of one leaves the rest doomed.
    pub(crate) fn take_doomed_finalizer(&mut self) -> Value {
        while let Some(finalizer) = self.vm.doomed_finalizers.pop_front() {
            if let data::Object::Finalizer(function) = &mut self.vm.objects[finalizer.payload() as usize - 1] {
                let function = std::mem::replace(function, Value::NIL);
                if function != Value::NIL { return function; }
            }
        }
        Value::NIL
    }
    /// `Vm::gc_phases'.
    pub(crate) fn gc_phases(&mut self) -> Value {
        self.vm.gc_phases()
    }
    /// The usage log's read of the collections since its last.
    pub(crate) fn take_collections(&mut self) -> Value {
        self.vm.take_collections()
    }
}
