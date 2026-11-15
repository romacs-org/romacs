//! Ordinary buffer/startup data-model operations; no window or redisplay ownership.
use crate::{coordinates::ByteOffset, data::Object, *};
use crate::text::Newlines;
use std::path::{Path, PathBuf};

/// GNU forward-line's motion of COUNT lines from byte POINT of TEXT
/// within the bytes R: the new point and the lines short.
fn line_motion(text: &(impl Newlines + ?Sized), r: std::ops::Range<usize>, point: usize, count: i64) -> (usize, i64) {
    if count > 0 {
        let wanted = crate::text::newline_count(count);
        let found = text.newlines_after(point, r.end, wanted);
        if let Some(last) = found.last.filter(|_| found.count == wanted) { return (last + 1, 0); }
        // Short of lines: a last line without its newline counts as one moved.
        let partial = found.last.map_or(point, |i| i + 1) < r.end;
        return (r.end, count - found.lines() - i64::from(partial));
    }
    // The start of point's line, then -COUNT lines back.
    let wanted = crate::text::newline_count(count).saturating_add(1);
    let found = text.newlines_before(point, r.start, wanted);
    match found.last.filter(|_| found.count == wanted) {
        Some(last) => (last + 1, 0),
        None => (r.start, count + found.lines()),
    }
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
                let shortage = match self.objects.entry_mut(index) {
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
                // As `set` writes: a watcher runs at the engine's drain.
                self.write_from_native(crate::watchers::NativeWrite::set(slot, mode)).map_err(crate::watchers::fault_text)?;
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
                self.write_from_native(crate::watchers::NativeWrite::set(slot, value)).map_err(crate::watchers::fault_text)?;
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

#[cfg(test)]
mod tests {
    use super::line_motion;
    use crate::prim::{line_beginning, line_end_after};
    use crate::text::Newlines;

    /// Text of LINES lines of 0-40 bytes, some with a two-byte character,
    /// the last without its newline when OPEN.
    fn lines(count: usize, open: bool) -> String {
        let mut seed = 7u64;
        let mut next = || { seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407); (seed >> 33) as usize };
        let mut text = String::new();
        for _ in 0..count {
            text.extend(std::iter::repeat_n('x', next() % 40));
            if next() % 5 == 0 { text.push('λ'); }
            text.push('\n');
        }
        if open { text.push_str("open"); }
        text
    }

    // The motions as they were, one newline at a time.
    fn motion_by_lines(text: &[u8], r: std::ops::Range<usize>, point: usize, count: i64) -> (usize, i64) {
        let line_start = |end: usize| text.previous_newline(end, r.start).map_or(r.start, |i| i + 1);
        let (mut pos, mut remaining) = (point, count);
        if count <= 0 {
            pos = line_start(point);
            while remaining < 0 && pos > r.start { pos = line_start(pos - 1); remaining += 1; }
        } else {
            while remaining > 0 && pos < r.end { pos = text.next_newline(pos, r.end).map_or(r.end, |i| i + 1); remaining -= 1; }
        }
        (pos, remaining)
    }
    fn beginning_by_lines(text: &[u8], low: usize, high: usize, point: usize, lines: i64) -> usize {
        let start_of = |at: usize| text.previous_newline(at, low).map_or(low, |i| i + 1);
        let mut pos = start_of(point);
        if lines >= 0 {
            for _ in 0..lines { match text.next_newline(pos, high) { Some(i) => pos = i + 1, None => return high } }
        } else {
            for _ in lines..0 { if pos == low { break; } pos = start_of(pos - 1); }
        }
        pos
    }
    fn end_by_lines(text: &[u8], low: usize, high: usize, point: usize, count: i64) -> usize {
        let mut pos = point;
        if count > 0 {
            for _ in 0..count { match text.next_newline(pos, high) { Some(i) => pos = i + 1, None => return high } }
            return pos - 1;
        }
        for _ in count..0 { match text.previous_newline(pos, low) { Some(i) => pos = i, None => return low } }
        pos
    }

    /// `forward-line`, `line-beginning-position` and `line-end-position`'s
    /// motions find each run of lines in one scan of each piece of text, a
    /// slice or a rope of many chunks and a tail, and land where the motion
    /// a newline at a time did, short counts and narrowing included.
    #[test]
    fn line_motions_agree_with_motion_a_newline_at_a_time() {
        for open in [false, true] {
            let source = lines(3000, open);
            let mut text = crate::text::Text::new(source.as_bytes()).unwrap();
            // A tail after the rope: appended at point, at the end.
            let end = crate::coordinates::ByteOffset(text.len_bytes());
            text.set_view(crate::coordinates::ByteOffset(0)..end, end);
            assert!(text.append_utf8("\ntail\nend"));
            let bytes = text.as_str().as_bytes().to_vec();
            let len = bytes.len();
            assert!(text.chunks_in(0..len).count() > 20, "a rope of many chunks");
            let starts: Vec<usize> = std::iter::once(0).chain(memchr::memchr_iter(b'\n', &bytes).map(|i| i + 1)).collect();
            let ranges = [0..len, starts[40]..starts[2900], starts[1500]..len, 0..starts[3]];
            for r in ranges {
                let points = [r.start, r.end, usize::midpoint(r.start, r.end), starts[starts.len() / 3].clamp(r.start, r.end), r.end.saturating_sub(1).max(r.start)];
                for point in points.map(|p| if bytes.get(p).is_some_and(|b| b & 0xc0 == 0x80) { p - 1 } else { p }) {
                    for count in [-4000, -2999, -100, -3, -2, -1, 0, 1, 2, 3, 100, 2999, 4000, i64::MAX, i64::MIN + 1] {
                        let want = motion_by_lines(&bytes, r.clone(), point, count.clamp(-5000, 5000));
                        let want = (want.0, if count.abs() > 5000 { want.1 + (count - count.clamp(-5000, 5000)) } else { want.1 });
                        assert_eq!(line_motion(&bytes[..], r.clone(), point, count), want, "slice {r:?} point {point} count {count}");
                        assert_eq!(line_motion(&*text, r.clone(), point, count), want, "rope {r:?} point {point} count {count}");
                        let lines = count.clamp(-5000, 5000);
                        let want = beginning_by_lines(&bytes, r.start, r.end, point, lines);
                        assert_eq!(line_beginning(&*text, r.start, r.end, point, lines), want, "beginning {r:?} point {point} lines {lines}");
                        let want = end_by_lines(&bytes, r.start, r.end, point, lines);
                        assert_eq!(line_end_after(&*text, r.start, r.end, point, lines), want, "end {r:?} point {point} count {lines}");
                    }
                }
            }
        }
    }
}
