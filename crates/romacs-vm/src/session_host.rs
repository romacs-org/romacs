//! The embedder's end of the session: what `kill-emacs' (prims/emacs.rs)
//! asks for once `kill-emacs-hook' has run. GNU's Fkill_emacs exits the
//! process; a host that owns its process does the same. A VM with no
//! session host attached is embedded in a process it does not own (a
//! test harness, a library caller), so `kill-emacs' refuses there.
use crate::Vm;

/// Ends the session with an exit code; never returns.
pub type SessionHost = Box<dyn FnMut(i32) -> std::convert::Infallible>;

impl Vm {
    /// Attach (Some) or detach (None) the session's end.
    pub fn set_session_host(&mut self, host: Option<SessionHost>) {
        self.session_host = host;
    }
    pub(crate) fn has_session_host(&self) -> bool { self.session_host.is_some() }
    /// End the session with CODE through the host, if one is attached.
    pub(crate) fn end_session(&mut self, code: i32) -> crate::Result<std::convert::Infallible> {
        let host = self.session_host.as_mut().ok_or("no session host")?;
        Ok(host(code))
    }
}
