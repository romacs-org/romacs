//! Ordinary buffer/startup data-model operations; no window or redisplay ownership.
use crate::{coordinates::ByteOffset, data::Object, *};
use std::path::{Path, PathBuf};

fn line_motion(bytes: &[u8], point: usize, count: i64) -> (usize, i64) {
    let mut pos = point;
    let mut remaining = count;
    if count <= 0 {
        pos = bytes[..pos]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        while remaining < 0 && pos > 0 {
            pos = bytes[..pos - 1]
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |i| i + 1);
            remaining += 1;
        }
    } else {
        while remaining > 0 && pos < bytes.len() {
            pos = bytes[pos..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| pos + i + 1);
            remaining -= 1;
        }
    }
    (pos, remaining)
}

impl Vm {
    pub(super) fn eval_buffer_name(&self, value: Value) -> Result<Option<String>> {
        match self.str_view(value) {
            Some(view) => view.as_str().map(|s| Some(s.to_owned()))
                .ok_or_else(|| "not-unicode-string".into()),
            None => Ok(None),
        }
    }
    pub(super) fn eval_buffer_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        let valid = match p {
            GetBuffer => n == 1,
            TempBuffer => n <= 1,
            SwitchToBuffer => (1..=3).contains(&n),
            ForwardLine | MenuBarMode | ForceModeLineUpdate => n <= 1,
            LocateLibrary => (1..=4).contains(&n),
            EmacsLispMode | Daemonp | DaemonInitialized => n == 0,
            _ => false,
        };
        if !valid {
            return Err("wrong primitive arity".into());
        }
        match p {
            TempBuffer => {
                if n == 0 {
                    let buffer = self.create_named_buffer(" *temp*", true)?;
                    self.buffer_state_mut(buffer)?.inhibit_buffer_hooks = true;
                    Ok(buffer)
                }
                else {
                    self.kill_buffer_model(self.stack[start])?;
                    Ok(Value::NIL)
                }
            }
            GetBuffer | SwitchToBuffer => {
                let arg = self.stack[start];
                let value = if let Some(name) = self.eval_buffer_name(arg)? {
                    if p == SwitchToBuffer && name.is_empty() {
                        return Err("empty buffer name".into());
                    }
                    if p == SwitchToBuffer || (name == "*scratch*" && self.scratch_unmade()) {
                        self.create_named_buffer(&name, false)?
                    } else { self.named_buffer(&name).unwrap_or(Value::NIL) }
                } else if matches!(
                    self.object_if(arg),
                    Some(Object::Buffer(_) | Object::ByteBuffer(_))
                ) {
                    arg
                } else {
                    return Err("buffer or string required".into());
                };
                if p == SwitchToBuffer {
                    self.require_live_buffer(value)?;
                    self.slots[self.buffer_slot].value = value;
                }
                Ok(value)
            }
            ForwardLine => {
                let count = if n == 0 || self.stack[start] == Value::NIL {
                    1
                } else {
                    self.stack[start].as_integer()?
                };
                let index = self.buffer_index()?;
                let shortage = match &mut self.objects[index] {
                    Object::Buffer(b) => {
                        let range = b.accessible();
                        let (pos, shortage) = line_motion(
                            &b.as_str().as_bytes()[range.start.0..range.end.0],
                            b.point.0 - range.start.0,
                            count,
                        );
                        b.point = ByteOffset(pos + range.start.0);
                        shortage
                    }
                    Object::ByteBuffer(b) => {
                        let range = b.restriction.clone();
                        let (pos, shortage) = line_motion(
                            &b.bytes[range.start.0..range.end.0],
                            b.point.0 - range.start.0,
                            count,
                        );
                        b.point = ByteOffset(pos + range.start.0);
                        shortage
                    }
                    _ => unreachable!(),
                };
                Value::integer(shortage)
            }
            EmacsLispMode => {
                self.buffer_index()?;
                let slot = self.slot("major-mode");
                self.make_local_variable(slot)?;
                let mode = self.intern("emacs-lisp-mode");
                self.set_variable_value(slot, mode)?;
                Ok(Value::NIL)
            }
            MenuBarMode => {
                let slot = self.slot("menu-bar-mode");
                let toggle = self.intern("toggle");
                let enabled = if n > 0 && self.stack[start] == toggle {
                    self.variable_value(slot)? == Value::NIL
                } else if n > 0 && (self.stack[start].integer_value().is_some() || matches!(self.object_if(self.stack[start]), Some(Object::Float(_)))) {
                    !(self.number(self.stack[start])? < 1.0)
                } else {
                    true
                };
                let value = if enabled { Value::TRUE } else { Value::NIL };
                self.set_variable_value(slot, value)?;
                Ok(value)
            }
            ForceModeLineUpdate => {
                self.update_mode_lines();
                Ok(if n == 0 { Value::NIL } else { self.stack[start] })
            }
            Daemonp => Ok(Value::NIL),
            DaemonInitialized => {
                Err("This function can only be called if emacs is run as a daemon".into())
            }
            LocateLibrary => self.eval_locate_library(start, n),
            _ => unreachable!(),
        }
    }

    fn eval_locate_library(&mut self, start: usize, n: usize) -> Result<Value> {
        let name = self.source_string(self.stack[start])?.to_owned();
        let nosuffix = n > 1 && self.stack[start + 1] != Value::NIL;
        let path_slot = self.slot("load-path");
        let paths = if n > 2 && self.stack[start + 2] != Value::NIL {
            self.stack[start + 2]
        } else {
            self.variable_value(path_slot)?
        };
        let roots = if Path::new(&name).is_absolute() {
            vec![PathBuf::new()]
        } else {
            self.list_values(paths)?
                .into_iter()
                .map(|v| {
                    if v == Value::NIL {
                        Ok(PathBuf::from("."))
                    } else {
                        Ok(PathBuf::from(self.source_string(v)?))
                    }
                })
                .collect::<Result<Vec<_>>>()?
        };
        for root in roots {
            let base = root.join(&name);
            let candidates = if nosuffix {
                vec![base]
            } else {
                vec![PathBuf::from(format!("{}.el", base.display())), base]
            };
            for candidate in candidates {
                if candidate.is_file() {
                    let absolute = if candidate.is_absolute() {
                        candidate
                    } else {
                        std::env::current_dir()
                            .map_err(|e| e.to_string())?
                            .join(candidate)
                    };
                    return self.literal(&Form::String(
                        absolute.to_str().ok_or("not-unicode-string")?.to_owned(),
                    ));
                }
            }
        }
        Ok(Value::NIL)
    }
}
