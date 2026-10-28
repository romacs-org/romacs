//! World builds, the VM's side (DESIGN-M1 §5, §6): the recipe runs with
//! `host::Mode::Build`, and the VM marks where it is in the build.
use crate::{Vm, host};

impl Vm {
    /// The VM is made: the recipe starts, and per-process host facts may no
    /// longer be read outside startup. A no-op outside build mode.
    pub fn world_begin(&mut self) {
        host::begin_recipe();
    }

    /// `romacs--startup-phase`: the startup function's run begins (ON) or
    /// ends. A no-op outside build mode.
    pub(crate) fn startup_phase(&mut self, on: bool) {
        if host::mode() == host::Mode::Build {
            host::switch_startup(on);
        }
    }
}
