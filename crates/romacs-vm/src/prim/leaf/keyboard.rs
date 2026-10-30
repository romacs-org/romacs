//! The host keyboard's state for prims/keyboard.rs.
use super::*;

impl Leaf<'_> {
    /// keyboard.c readable_events over the host keyboard: whether an
    /// attached host holds input not yet read. No Lisp runs.
    pub(crate) fn host_input_pending(&mut self) -> bool {
        self.vm.host_input.as_mut().is_some_and(|host| host.input_pending())
    }
}
