//! World builds: the startup phase (`crate::world`).
use super::*;

impl Leaf<'_> {
    pub(crate) fn startup_phase(&mut self, on: bool) {
        self.vm.startup_phase(on);
    }

    /// `symbol-plist`: a read of every property of SYMBOL, which startup
    /// makes an input.
    pub(crate) fn observed_symbol_plist(&mut self, symbol: Value) -> crate::prim::Out {
        if self.vm.fence.is_some() {
            self.vm.fence_property(symbol, None, crate::world::Access::Read, None);
        }
        self.symbol_plist(symbol)
    }

    /// The variable NAME's value read past the fence, by a caller whose
    /// result this value does not decide; nil when void.
    pub(crate) fn unfenced_variable(&mut self, name: &str) -> crate::prim::Out {
        let symbol = self.intern(name);
        let slot = self.symbol_slot(symbol)?;
        let value = self.vm.unfenced(slot, |vm| vm.variable_value(slot)).map_err(Fault::Error)?;
        Ok(if value == Value::UNBOUND { Value::NIL } else { value })
    }

    /// The environment list NAME as its Rust owners read it: one named
    /// entry, which the host records, not a read of the fenced list.
    pub(crate) fn environment_list(&mut self, name: &str) -> crate::prim::Out {
        let symbol = self.intern(name);
        let slot = self.symbol_slot(symbol)?;
        let value = self.vm.unfenced(slot, |vm| vm.variable_value(slot)).map_err(Fault::Error)?;
        if value == Value::UNBOUND { Err(Fault::VoidVariable(symbol)) } else { Ok(value) }
    }
}
