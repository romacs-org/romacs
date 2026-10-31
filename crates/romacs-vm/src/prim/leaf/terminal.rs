use super::*;
use crate::prim::Out;
use crate::terminal_host::TerminalBinding;

impl Leaf<'_> {
    /// A terminal of COLUMNS x ROWS, or, when nil, of the stable body of
    /// the window that controls the current buffer (`controlling_window').
    pub(crate) fn terminal_create(&mut self, shell: Value, directory: Value, columns: Value, rows: Value) -> Out {
        let shell = self.string_text(shell)?;
        let directory = self.string_text(directory)?;
        let buffer = self.current_buffer()?;
        let (columns, rows) = if columns == Value::NIL && rows == Value::NIL {
            self.vm.ensure_windows().map_err(Fault::Error)?;
            let mut showing = vec![];
            for w in self.vm.leaf_windows().map_err(Fault::Error)? {
                if self.vm.window(w).map_err(Fault::Error)?.buffer == buffer { showing.push(w); }
            }
            let control = self.vm.controlling_window(buffer, &showing).map_err(Fault::Error)?
                .ok_or_else(|| Fault::Error("Terminal buffer is not shown in a window".into()))?;
            self.vm.stable_window_body(control).map_err(Fault::Error)?
                .ok_or_else(|| Fault::Error("Window too small for a terminal".into()))?
        } else {
            let size = |v: Value, what: &str| v.as_integer().ok().and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0)
                .ok_or_else(|| Fault::Error(format!("Invalid terminal {what}")));
            (size(columns, "columns")?, size(rows, "rows")?)
        };
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
        self.vm.terminal_bindings.push(TerminalBinding::new(buffer, session, columns, rows));
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
