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

    /// Make WINDOW (the selected one when nil) follow SESSION's terminal
    /// again at once (`terminal_snap'), or stop following it
    /// (`TerminalBinding::away').
    pub(crate) fn terminal_follow(&mut self, session: i64, follow: Value, window: Value) -> Out {
        let window = if window == Value::NIL { self.vm.selected_window_value().map_err(Fault::Error)? } else { window };
        self.vm.window(window).map_err(Fault::Error)?;
        let index = self.vm.terminal_bindings.iter().position(|b| b.session == session as u64)
            .ok_or_else(|| Fault::Error("Unknown terminal session".into()))?;
        if follow == Value::NIL {
            let away = &mut self.vm.terminal_bindings[index].away;
            if !away.contains(&window) { away.push(window); }
        } else {
            self.vm.terminal_snap(index, window).map_err(Fault::Error)?;
        }
        Ok(follow)
    }

    /// Mark away the terminal windows a command moved off their cursor
    /// (`terminal_track').
    pub(crate) fn terminal_track(&mut self) -> Out {
        self.vm.terminal_track().map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    /// Paste TEXT into SESSION as Alacritty pastes: between the bracketed
    /// paste marks, less ESC and ETX (so the text can neither end the paste
    /// nor interrupt), when the program asked for them; else with each line
    /// end a carriage return, as a typed RET is.
    pub(crate) fn terminal_paste(&mut self, session: i64, text: Value) -> Out {
        let text = self.string_text(text)?;
        let id = u64::try_from(session).map_err(|_| Fault::Error("Invalid terminal session".into()))?;
        let state = self.vm.terminal_input_state(id).map_err(Fault::Error)?;
        let bytes = if state.bracketed_paste {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend(text.bytes().filter(|&b| b != 0x1b && b != 0x03));
            bytes.extend_from_slice(b"\x1b[201~");
            bytes
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
        };
        let string = self.vm.make_lisp_string(bytes, false);
        self.terminal_send(session, string)
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

    /// EVENT, a Lisp input event, as the bytes a terminal sends for it
    /// (`terminal_keys'): a unibyte string, empty for an event with no bytes.
    pub(crate) fn terminal_encode_event(&mut self, event: Value, application_cursor: bool) -> Out {
        let bytes = self.event_bytes(event, application_cursor)?;
        Ok(self.vm.make_lisp_string(bytes, false))
    }

    fn event_bytes(&mut self, event: Value, application_cursor: bool) -> std::result::Result<Vec<u8>, Fault> {
        use crate::terminal_keys::{character_event, encode, symbol_event};
        let key = if let Ok(code) = event.as_integer() {
            character_event(code).ok_or_else(|| Fault::Error("Invalid character event".into()))?
        } else if event.tag() == 7 || event == Value::NIL || event == Value::TRUE {
            let name = self.symbol_name(event)?;
            return encode(symbol_event(&name), application_cursor).map_err(Fault::Error);
        } else {
            // A mouse or other list event: no terminal bytes.
            return Ok(Vec::new());
        };
        encode(key, application_cursor).map_err(Fault::Error)
    }

    /// Send EVENT to SESSION as the terminal's bytes for it, in the
    /// application-cursor mode the terminal is in now.
    pub(crate) fn terminal_send_event(&mut self, session: i64, event: Value) -> Out {
        let id = u64::try_from(session).map_err(|_| Fault::Error("Invalid terminal session".into()))?;
        let state = self.vm.terminal_input_state(id).map_err(Fault::Error)?;
        let bytes = self.event_bytes(event, state.application_cursor)?;
        if bytes.is_empty() {
            return Ok(Value::NIL);
        }
        let string = self.vm.make_lisp_string(bytes, false);
        self.terminal_send(session, string)
    }

    /// SESSION's state, as a plist: `:running', `:alternate',
    /// `:bracketed-paste', `:application-cursor', `:following' (the
    /// selected window's, when it shows the terminal), and, once the
    /// terminal has published, `:cursor' and `:screen-start' as positions
    /// in its buffer.
    pub(crate) fn terminal_state(&mut self, session: i64) -> Out {
        let id = u64::try_from(session).map_err(|_| Fault::Error("Invalid terminal session".into()))?;
        let state = self.vm.terminal_input_state(id).map_err(Fault::Error)?;
        let selected = self.vm.selected_window_value().map_err(Fault::Error)?;
        let binding = self.vm.terminal_bindings.iter().find(|b| b.session == id)
            .ok_or_else(|| Fault::Error("Unknown terminal session".into()))?;
        let (buffer, published, cursor, screen) = (binding.buffer, binding.revision.is_some(), binding.cursor_byte, binding.screen_start_byte);
        let following = !binding.away.contains(&selected);
        let flag = |on: bool| if on { Value::TRUE } else { Value::NIL };
        let mut items = Vec::new();
        for (key, on) in [(":running", state.running), (":alternate", state.alternate_screen), (":bracketed-paste", state.bracketed_paste),
                          (":application-cursor", state.application_cursor), (":following", following)] {
            items.push(self.intern(key));
            items.push(flag(on));
        }
        let positions = match self.vm.object(buffer).map_err(Fault::Error)? {
            crate::data::Object::Buffer(text) if published => {
                let position = |byte: usize| text.char_of_byte(crate::coordinates::ByteOffset(byte)).map(|c| c.0 as i64 + 1);
                Some((position(cursor).map_err(Fault::Error)?, position(screen).map_err(Fault::Error)?))
            }
            _ => None,
        };
        if let Some((cursor, screen)) = positions {
            items.push(self.intern(":cursor"));
            items.push(Value::integer(cursor).map_err(Fault::Error)?);
            items.push(self.intern(":screen-start"));
            items.push(Value::integer(screen).map_err(Fault::Error)?);
        }
        Ok(self.vm.list_value(&items))
    }

    pub(crate) fn terminal_running(&mut self, session: i64) -> Out {
        let running = self.vm.terminal_input_state(session as u64).map_err(Fault::Error)?.running;
        Ok(if running { Value::TRUE } else { Value::NIL })
    }
}
