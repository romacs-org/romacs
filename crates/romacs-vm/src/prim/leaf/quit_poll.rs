//! A primitive's side of the native quit poll (`Vm::native_poll`): which
//! primitives' loops may quit, and the poll their loops call.
use crate::prim::{Fault, Leaf};

impl Leaf<'_> {
    /// BODY, a primitive's work Lisp called, whose native loops stop
    /// consistently at their polls: a C-g there quits, leaving what the loop
    /// did standing, as GNU's `maybe_quit` in a C loop does. The same loops
    /// reached from redisplay, outside this scope, answer inspections only.
    /// GNU's display iterator quits nowhere; `vertical-motion` over a long
    /// buffer should.
    pub(crate) fn quittable<T>(&mut self, body: impl FnOnce(&mut Self) -> std::result::Result<T, Fault>) -> std::result::Result<T, Fault> {
        let outer = std::mem::replace(&mut self.vm.native_quit, true);
        let result = body(self);
        self.vm.native_quit = outer;
        result
    }

    /// `Vm::native_poll`.
    pub(crate) fn native_poll(&mut self) -> std::result::Result<(), Fault> {
        self.vm.native_poll()
    }

    /// `Vm::native_stop`.
    pub(crate) fn native_stop(&self) -> crate::quit::NativeStop {
        self.vm.native_stop()
    }
}

pub(crate) use crate::quit::{POLL_STEPS, Steps};
