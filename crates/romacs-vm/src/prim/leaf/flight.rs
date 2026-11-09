//! The flight recorder's steps from Lisp (`crate::flight`).
use super::{Fault, Leaf};
use crate::Value;

impl Leaf<'_> {
    /// A step ends, LABEL's (a command): logged, or checked against the
    /// log, with the VM's fingerprint after it. Nothing when the session
    /// neither records nor replays.
    pub(crate) fn flight_step(&self, label: Value) -> std::result::Result<(), Fault> {
        if !crate::flight::active() {
            return Ok(());
        }
        let label = self.vm.display(label).map_err(Fault::Error)?;
        crate::flight::step(|| label, || self.vm.flight_step_fingerprint());
        Ok(())
    }

    /// The VM's fingerprint, as a flight step logs it.
    pub(crate) fn flight_fingerprint(&mut self) -> std::result::Result<Value, Fault> {
        let fingerprint = self.vm.flight_fingerprint().map_err(Fault::Error)?;
        Ok(self.make_string(fingerprint.into_bytes(), false))
    }
}
