//! The evaluator's counted host calls from Lisp (`crate::host_calls`).
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// `Vm::host_calls`.
    pub(crate) fn host_calls(&mut self) -> std::result::Result<Value, Fault> {
        self.vm.host_calls().map_err(Fault::Error)
    }
}
