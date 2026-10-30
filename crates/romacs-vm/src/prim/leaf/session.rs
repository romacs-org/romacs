//! The session's end for `kill-emacs' (prims/emacs.rs, session_host.rs).
use super::*;

impl Leaf<'_> {
    pub(crate) fn has_session_host(&self) -> bool { self.vm.has_session_host() }
    /// Hand CODE to the session host, which ends the process.
    pub(crate) fn end_session(&mut self, code: i32) -> std::result::Result<std::convert::Infallible, Fault> {
        self.vm.end_session(code).map_err(Fault::Error)
    }
}
