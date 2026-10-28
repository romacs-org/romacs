//! World builds: the startup phase (`crate::world`).
use super::*;

impl Leaf<'_> {
    pub(crate) fn startup_phase(&mut self, on: bool) {
        self.vm.startup_phase(on);
    }
}
