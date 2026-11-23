//! The macro record (`crate::macro_record`): what the compiles expanded
//! through, and the queries over it.
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// Each of SYMBOLS' function cells and the alias links from it to the
    /// end of its chain, each as (SYMBOL-SLOT . VERSION) now; nothing for a
    /// non-symbol.
    pub(crate) fn macro_cells(&self, symbols: &[Value]) -> Vec<crate::macro_record::MacroUse> {
        symbols.iter().filter_map(|&symbol| self.symbol_index(symbol)).flat_map(|slot| self.vm.alias_cells(slot)).collect()
    }
    /// The compile now running expanded through each of USES.
    pub(crate) fn record_macro_cells(&mut self, uses: &[crate::macro_record::MacroUse]) {
        self.vm.record_macro_uses(uses);
    }
    /// SYMBOL's slot, for a cell `romacs--record-macro-cells` names.
    pub(crate) fn macro_cell_slot(&self, symbol: Value) -> std::result::Result<usize, Fault> {
        self.symbol_slot(symbol)
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
