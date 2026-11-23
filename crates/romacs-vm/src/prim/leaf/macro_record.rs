//! The macro record (`crate::macro_record`): what the compiles expanded
//! through, and the queries over it.
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// `Vm::open_macro_capture` of SYMBOLS' slots, its depth; non-symbols
    /// are left out.
    pub(crate) fn open_macro_capture(&mut self, symbols: &[Value]) -> usize {
        let slots = symbols.iter().filter_map(|&symbol| self.symbol_index(symbol)).collect();
        self.vm.open_macro_capture(slots)
    }
    /// `Vm::close_macro_capture`.
    pub(crate) fn close_macro_capture(&mut self, depth: usize, record: bool) {
        self.vm.close_macro_capture(depth, record);
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
