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
    /// run_finalizers' first half: the functions of the doomed finalizers,
    /// oldest first, each finalizer emptied so that it never runs again.
    pub(crate) fn take_doomed_finalizers(&mut self) -> Value {
        let mut functions = vec![];
        for finalizer in std::mem::take(&mut self.vm.doomed_finalizers) {
            if let data::Object::Finalizer(function) = &mut self.vm.objects[finalizer.payload() as usize - 1] {
                functions.push(std::mem::replace(function, Value::NIL));
            }
        }
        let mut list = Value::NIL;
        for function in functions.into_iter().rev().filter(|f| *f != Value::NIL) { list = self.cons(function, list); }
        list
    }
    /// The usage log's read of the collections since its last.
    pub(crate) fn take_collections(&mut self) -> Value {
        self.vm.take_collections()
    }
}
