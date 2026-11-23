//! The macro record (`crate::macro_record`): what the compiles expanded
//! through, and the queries over it.
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// The compile now running expanded through SYMBOL's function cell;
    /// nothing for a non-symbol.
    pub(crate) fn record_macro_use(&mut self, symbol: Value) {
        if let Some(slot) = self.symbol_index(symbol) {
            self.vm.record_macro_use(slot);
        }
    }
    /// `Vm::who_macroexpands` of SYMBOL.
    pub(crate) fn who_macroexpands(&self, symbol: Value, transitive: bool) -> std::result::Result<Vec<usize>, Fault> {
        Ok(self.vm.who_macroexpands(self.symbol_slot(symbol)?, transitive))
    }
    /// `Vm::macro_invalidations`.
    pub(crate) fn macro_invalidations(&self) -> (u64, Vec<crate::macro_record::Invalidation>) {
        self.vm.macro_invalidations()
    }
}
