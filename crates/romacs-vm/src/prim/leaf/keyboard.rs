//! The host keyboard's state for prims/keyboard.rs.
use super::*;

impl Leaf<'_> {
    /// Whether `debug-on-event' names the user signal EVENT (quit.rs).
    pub(crate) fn debugs_on_event(&mut self, event: &str) -> bool {
        self.vm.debugs_on(event)
    }
    /// handle_user_signal's debugger entry for `debug-on-event' (quit.rs).
    pub(crate) fn debug_event(&mut self) {
        self.vm.debug_event()
    }
    /// keyboard.c get_input_pending over the host keyboard: a pending quit
    /// (`quit-flag', or a `C-g' the host counted) is input, since read_char
    /// returns the quit character for it; then whether an attached host
    /// holds input not yet read. No Lisp runs.
    pub(crate) fn host_input_pending(&mut self) -> bool {
        self.vm.quit_input_pending() || self.vm.host_input_pending()
    }
}

impl Leaf<'_> {
    /// `romacs--call-deadline': arm GENERATION's deadline MILLISECONDS from
    /// now, or with GENERATION nil disarm the current one (quit.rs
    /// `deadline').
    pub(crate) fn call_deadline(&mut self, generation: Value, milliseconds: Value) -> crate::prim::Out {
        if generation == Value::NIL {
            crate::host::call_deadline::disarm(&self.vm.quit);
            return Ok(Value::NIL);
        }
        let generation = generation.integer_value().filter(|&g| g > 0)
            .ok_or(crate::prim::Fault::Wrong("natnump", generation))?;
        let milliseconds = milliseconds.integer_value().filter(|&m| m >= 0)
            .ok_or(crate::prim::Fault::Wrong("natnump", milliseconds))?;
        crate::host::call_deadline::arm(self.vm.quit.clone(), generation as u64,
                                   std::time::Duration::from_millis(milliseconds as u64));
        Ok(Value::TRUE)
    }
}
