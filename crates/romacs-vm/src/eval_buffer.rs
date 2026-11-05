//! Ordinary buffer/startup data-model operations; no window or redisplay ownership.
use crate::{coordinates::ByteOffset, data::Object, *};
use crate::text::Newlines;
use std::path::{Path, PathBuf};

/// GNU forward-line's motion of COUNT lines from byte POINT of TEXT
/// within the bytes R: the new point and the lines short.
fn line_motion(text: &(impl Newlines + ?Sized), r: std::ops::Range<usize>, point: usize, count: i64) -> (usize, i64) {
    let mut remaining = count;
    let line_start = |end: usize| text.previous_newline(end, r.start).map_or(r.start, |i| i + 1);
    let mut pos;
    if count <= 0 {
        pos = line_start(point);
        while remaining < 0 && pos > r.start {
            pos = line_start(pos - 1);
            remaining += 1;
        }
    } else {
        pos = point;
        while remaining > 0 && pos < r.end {
            pos = text.next_newline(pos, r.end).map_or(r.end, |i| i + 1);
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
                let value = if let Some(name) = self.buffer_name_text(arg) {
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
                        let (pos, shortage) = line_motion(b.body(), range.start.0..range.end.0, b.point.0, count);
                        b.point = ByteOffset(pos);
                        shortage
                    }
                    Object::ByteBuffer(b) => {
                        let range = b.restriction.start.0..b.restriction.end.0;
                        let (pos, shortage) = line_motion(b.bytes.as_slice(), range, b.point.0, count);
                        b.point = ByteOffset(pos);
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
                // GNU's update_mode_lines: the bar is expanded again at the
                // next redisplay (`BarKey'), from freshly converted forms.
                self.display_state.mode_line_epoch = self.display_state.mode_line_epoch.wrapping_add(1);
                self.mode_line_forms.clear();
                Ok(if n == 0 { Value::NIL } else { self.stack[start] })
            }
            // emacs.c Fdaemonp: a server's name, as `--fg-daemon=NAME''s.
            Daemonp => match crate::host::daemon() {
                Some(name) => self.literal(&Form::String(name.to_owned())),
                None => Ok(Value::NIL),
            },
            // emacs.c Fdaemon_initialized: a server is initialized once it
            // listens, before any Lisp can call this.
            DaemonInitialized => Err(if crate::host::daemon().is_some() {
                "The daemon has already been initialized"
            } else {
                "This function can only be called if emacs is run as a daemon"
            }
            .into()),
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
                if crate::host::probe(crate::host::Probe::IsFile, &candidate) {
                    let absolute = if candidate.is_absolute() {
                        candidate
                    } else {
                        crate::host::current_dir()
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
