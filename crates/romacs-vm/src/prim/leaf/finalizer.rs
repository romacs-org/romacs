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
    /// Whether `post-gc-hook' is due (`Vm::post_gc_hook_due'): the caller,
    /// not a later poll, now owns running it.
    pub(crate) fn take_post_gc(&mut self) -> bool {
        let due = self.vm.post_gc_hook_due();
        self.vm.post_gc_due = false;
        due
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
