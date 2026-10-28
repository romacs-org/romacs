//! World builds: the startup phase (`crate::world`).
use super::*;

impl Leaf<'_> {
    pub(crate) fn startup_phase(&mut self, on: bool) {
        self.vm.startup_phase(on);
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
