//! The host keyboard's state for prims/keyboard.rs.
use super::*;

impl Leaf<'_> {
    /// keyboard.c get_input_pending over the host keyboard: a pending quit
    /// (`quit-flag', or a `C-g' the host counted) is input, since read_char
    /// returns the quit character for it; then whether an attached host
    /// holds input not yet read. No Lisp runs.
    pub(crate) fn host_input_pending(&mut self) -> bool {
        self.vm.quit_input_pending() || self.vm.host_input_pending()
    }
}
