//! Redisplay's counts from Lisp (`crate::display_stats`).
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// `Vm::display_stats`.
    pub(crate) fn display_stats(&mut self, reset: bool) -> std::result::Result<Value, Fault> {
        self.vm.display_stats(reset).map_err(Fault::Error)
    }
}
