use super::*;
use crate::prim::Out;
use crate::terminal_host::TerminalBinding;

impl Leaf<'_> {
    pub(crate) fn terminal_create(&mut self, shell: Value, directory: Value, columns: i64, rows: i64) -> Out {
        let shell = self.string_text(shell)?;
        let directory = self.string_text(directory)?;
        let columns = u16::try_from(columns).ok().filter(|n| *n > 0).ok_or_else(|| Fault::Error("Invalid terminal columns".into()))?;
        let rows = u16::try_from(rows).ok().filter(|n| *n > 0).ok_or_else(|| Fault::Error("Invalid terminal rows".into()))?;
        let buffer = self.current_buffer()?;
        if self.vm.terminal_bindings.iter().any(|b| b.buffer == buffer) {
            return Err(Fault::Error("Buffer already owns a terminal".into()));
        }
        let host = self.vm.terminal_host.as_mut().ok_or_else(|| Fault::Error("No terminal host attached".into()))?;
        let session = host.create(&shell, &directory, columns, rows).map_err(Fault::Error)?;
        let value = i64::try_from(session).ok().and_then(|id| Value::integer(id).ok());
        let Some(value) = value else {
            let _ = host.close(session);
            return Err(Fault::Error("Terminal session identity exhausted".into()));
        };
        self.vm.terminal_bindings.push(TerminalBinding { buffer, session, revision: None, follow: true, columns, rows, styles: std::sync::Arc::from([]), default_colors: [None; 2] });
        Ok(value)
    }

    pub(crate) fn terminal_send(&mut self, session: i64, bytes: Value) -> Out {
        let bytes = self.string_view(bytes)?.bytes;
        let id = u64::try_from(session).map_err(|_| Fault::Error("Invalid terminal session".into()))?;
        let snapshot = self.vm.terminal_input_state(id).map_err(Fault::Error)?;
        if !snapshot.running {
            let condition = self.intern("romacs-terminal-not-running");
            let data = self.cons(Value::integer(session).unwrap(), Value::NIL);
            return Err(Fault::Signal(condition, data));
        }
        self.vm.terminal_send(id, &bytes).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    pub(crate) fn terminal_follow(&mut self, session: i64, follow: Value) -> Out {
        let binding = self.vm.terminal_bindings.iter_mut().find(|b| b.session == session as u64)
            .ok_or_else(|| Fault::Error("Unknown terminal session".into()))?;
        binding.follow = follow != Value::NIL;
        binding.revision = None;
        Ok(follow)
    }

    pub(crate) fn terminal_close(&mut self, session: i64) -> Out {
        let id = u64::try_from(session).map_err(|_| Fault::Error("Invalid terminal session".into()))?;
        if self.vm.terminal_bindings.iter().any(|b| b.session == id) {
            self.vm.terminal_host.as_mut().ok_or_else(|| Fault::Error("No terminal host attached".into()))?
                .close(id).map_err(Fault::Error)?;
            self.vm.terminal_bindings.retain(|b| b.session != id);
        }
        Ok(Value::NIL)
    }

    pub(crate) fn terminal_running(&mut self, session: i64) -> Out {
        let running = self.vm.terminal_input_state(session as u64).map_err(Fault::Error)?.running;
        Ok(if running { Value::TRUE } else { Value::NIL })
    }
}
