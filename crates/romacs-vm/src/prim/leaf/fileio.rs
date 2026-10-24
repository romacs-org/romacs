//! The current buffer's text, for `prims/fileio.rs` (`insert-file-contents',
//! `write-region'). A child of `prim::leaf`, so it may read `Leaf`'s private
//! `vm`; nothing here can reach a collection point.
//!
//! Every accessor copies data out (owned bytes, plain integers), so no borrowed
//! heap reference crosses an allocation. Buffer TEXT is not a Lisp object: the
//! inserted characters live in the buffer's rope, which the collector does not
//! move. The only Lisp objects the file primitives keep are the strings they
//! store in per-buffer variables (`buffer-file-name`), which the buffer's own
//! `visit_values` traces (forwarded.rs COLLECTOR CONTRACT).
use super::*;
use crate::coordinates::{ByteOffset, CharOffset};

/// What GNU's insert/write code reads about `current_buffer` before it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BufferShape {
    /// Unibyte (`enable-multibyte-characters` nil): a VM2 `ByteBuffer`.
    pub unibyte: bool,
    /// `BEG == Z`: the whole buffer, ignoring narrowing, is empty.
    pub empty: bool,
    /// BEGV and ZV as 1-based character positions.
    pub begv: i64,
    pub zv: i64,
    /// Z, 1-based: one past the last character of the widened buffer.
    pub z: i64,
}

impl Leaf<'_> {
    /// A non-evaluating regexp search: GNU file-name handler selection must
    /// neither fold case nor overwrite the caller's match data.
    pub fn file_name_match(&mut self, pattern: Value, string: Value) -> std::result::Result<Option<usize>, Fault> {
        self.vm.file_name_match(pattern, string)
    }

    pub fn fileio_modtime(&mut self, buffer: Value) -> std::result::Result<(crate::forwarded::VisitedModtime, Option<String>), Fault> {
        let current = self.current_buffer()?;
        let target = if buffer == Value::NIL { current } else { buffer };
        let index = target.index(0).ok().and_then(|i| i.checked_sub(1))
            .filter(|&i| matches!(self.vm.objects.get(i), Some(data::Object::Buffer(_) | data::Object::ByteBuffer(_))))
            .ok_or(Fault::Wrong("bufferp", target))?;
        let state = match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.per_buffer.1,
            data::Object::ByteBuffer(b) => b.per_buffer.1,
            _ => unreachable!(),
        };
        // T1 leaf: no callbacks or collection. Restore even on variable errors.
        self.vm.slots[self.vm.buffer_slot].value = target;
        let symbol = self.intern("buffer-file-name");
        let filename = self.variable_value(symbol).and_then(|v| match v {
            Some(v) if matches!(self.kind(v), Kind::String | Kind::Bytes) => self.string_text(v).map(Some),
            _ => Ok(None),
        });
        self.vm.slots[self.vm.buffer_slot].value = current;
        Ok((state, filename?))
    }

    pub fn fileio_set_modtime(&mut self, state: crate::forwarded::VisitedModtime) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => b.per_buffer.1 = state,
            data::Object::ByteBuffer(b) => b.per_buffer.1 = state,
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn file_visits(&self) -> Vec<(Value, String, Option<(u64, u64)>, Option<String>)> {
        self.vm.file_visits()
    }

    pub fn fileio_sync_output(&self, file: &std::fs::File) -> std::io::Result<()> {
        use std::os::fd::AsRawFd as _;
        if unsafe { libc::fsync(file.as_raw_fd()) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn fileio_close_output(&self, file: std::fs::File) -> std::io::Result<()> {
        use std::os::fd::IntoRawFd as _;
        if unsafe { libc::close(file.into_raw_fd()) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn fileio_write_output(&self, file: &mut std::fs::File, mut bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;
        use std::os::fd::AsRawFd as _;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !bytes.is_empty() {
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::from_raw_os_error(libc::ETIMEDOUT));
            }
            match file.write(bytes) {
                Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero)),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let timeout = remaining.as_millis().clamp(1, 100) as i32;
                    let mut pollfd = libc::pollfd { fd: file.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                    if unsafe { libc::poll(&mut pollfd, 1, timeout) } < 0 {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::Interrupted { return Err(error) }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// GNU `Fcurrent_buffer`, creating VM2's lazy `*scratch*` like every
    /// buffer primitive does.
    pub fn current_buffer(&mut self) -> std::result::Result<Value, Fault> {
        self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(self.vm.slots[self.vm.buffer_slot].value)
    }

    pub fn buffer_shape(&mut self) -> std::result::Result<BufferShape, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &self.vm.objects[index] {
            data::Object::Buffer(t) => {
                let a = t.accessible();
                let begv = t.char_of_byte(a.start).map_err(Fault::Error)?.0 as i64 + 1;
                let zv = t.char_of_byte(a.end).map_err(Fault::Error)?.0 as i64 + 1;
                let z = t.len_chars() as i64 + 1;
                Ok(BufferShape { unibyte: false, empty: t.len_bytes() == 0, begv, zv, z })
            }
            data::Object::ByteBuffer(b) => Ok(BufferShape {
                unibyte: true,
                empty: b.bytes.is_empty(),
                begv: b.restriction.start.0 as i64 + 1,
                zv: b.restriction.end.0 as i64 + 1,
                z: b.bytes.len() as i64 + 1,
            }),
            _ => Err(Fault::Error("buffer required".into())),
        }
    }

    /// GNU textget: direct property, category fallback, then defaults. Values
    /// are copied out of the rope before interning can grow VM storage.
    fn fileio_text_property(&mut self, at: ByteOffset, key: &str) -> std::result::Result<Value, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(Value::NIL) };
        let direct = t.properties.get_property(at, key);
        let category = t.properties.get_property(at, "category").unwrap_or(Value::NIL);
        if let Some(value) = direct { return Ok(value) }
        let symbol = self.intern(key);
        if self.kind(category) == Kind::Symbol {
            let value = self.vm.symbol_property(category, symbol).map_err(Fault::Error)?;
            if value != Value::NIL { return Ok(value) }
        }
        let aliases = self.intern("char-property-alias-alist");
        let aliases = self.variable_value(aliases)?.unwrap_or(Value::NIL);
        let mut entries = aliases;
        let mut seen_entries = Vec::new();
        while self.kind(entries) == Kind::Cons {
            if seen_entries.contains(&entries) { break }
            seen_entries.push(entries);
            let (entry, tail) = self.vm.cons_pair(entries).map_err(Fault::Error)?;
            entries = tail;
            if self.kind(entry) != Kind::Cons { continue }
            let (property, mut alternatives) = self.vm.cons_pair(entry).map_err(Fault::Error)?;
            if property != symbol { continue }
            let mut seen_alternatives = Vec::new();
            while self.kind(alternatives) == Kind::Cons {
                if seen_alternatives.contains(&alternatives) { break }
                seen_alternatives.push(alternatives);
                let (alias, tail) = self.vm.cons_pair(alternatives).map_err(Fault::Error)?;
                alternatives = tail;
                if self.kind(alias) != Kind::Symbol { continue }
                let alias = self.symbol_name(alias)?;
                let index = self.vm.buffer_index().map_err(Fault::Error)?;
                if let data::Object::Buffer(t) = &self.vm.objects[index]
                    && let Some(value) = t.properties.get_property(at, &alias)
                    && value != Value::NIL { return Ok(value) }
            }
            break;
        }
        let defaults = self.intern("default-text-properties");
        let defaults = self.variable_value(defaults)?.unwrap_or(Value::NIL);
        Ok(self.vm.plist_lookup(defaults, symbol))
    }

    pub fn fileio_property_at_point(&mut self, key: &str) -> std::result::Result<Value, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(Value::NIL) };
        let at = t.point;
        self.fileio_text_property(at, key)
    }

    fn fileio_memq(&self, value: Value, list: Value) -> std::result::Result<bool, Fault> {
        Ok(self.vm.list_values(list).map_err(Fault::Error)?.contains(&value))
    }

    /// GNU verify_interval_modification's insertion boundary checks, including
    /// read-only supplied by a category and selective inhibit-read-only lists.
    pub fn fileio_check_insertion(&mut self, inhibit: Value) -> std::result::Result<(), Fault> {
        if inhibit != Value::NIL && self.kind(inhibit) != Kind::Cons { return Ok(()) }
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(()) };
        let at = t.point;
        let left = (at > t.accessible().start).then(|| ByteOffset(at.0 - 1));
        let right = (at < t.accessible().end).then_some(at);
        let ro = self.intern("read-only");
        let cat = self.intern("category");
        for (pos, front) in [(left, false), (right, true)] {
            let Some(pos) = pos else { continue };
            let value = self.fileio_text_property(pos, "read-only")?;
            if value == Value::NIL || self.fileio_memq(value, inhibit)? { continue }
            let sticky = self.fileio_text_property(pos, if front { "front-sticky" } else { "rear-nonsticky" })?;
            let member = |this: &Self, key| -> std::result::Result<bool, Fault> {
                if sticky == Value::NIL { Ok(false) }
                else if this.kind(sticky) != Kind::Cons { Ok(true) }
                else { this.fileio_memq(key, sticky) }
            };
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            let data::Object::Buffer(t) = &self.vm.objects[index] else { unreachable!() };
            let direct = t.properties.get_property(pos, "read-only").unwrap_or(Value::NIL);
            let sticks = member(self, ro)? || (direct == Value::NIL && member(self, cat)?);
            if front == sticks {
                let condition = self.intern("text-read-only");
                let data = if matches!(self.kind(value), Kind::String | Kind::Bytes) { self.cons(value, Value::NIL) } else { Value::NIL };
                return Err(Fault::Signal(condition, data));
            }
        }
        Ok(())
    }

    pub fn fileio_has_insertion_hooks(&mut self) -> std::result::Result<bool, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(false) };
        let at = t.point;
        let left = (at > t.accessible().start).then(|| ByteOffset(at.0 - 1));
        let right = (at < t.accessible().end).then_some(at);
        for (pos, insertion_hook) in [(left, "insert-behind-hooks"), (right, "insert-in-front-hooks")] {
            if let Some(pos) = pos {
                for key in ["modification-hooks", insertion_hook] {
                    if self.fileio_text_property(pos, key)? != Value::NIL { return Ok(true) }
                }
            }
        }
        Ok(false)
    }

    /// Insert TEXT at point of the current multibyte buffer and leave point
    /// BEFORE it, as `insert-file-contents' does (it inserts into the gap at
    /// PT and never moves PT). Markers follow their insertion type. Returns
    /// the number of characters inserted.
    pub fn insert_before_point(&mut self, text: &str) -> std::result::Result<usize, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &mut self.vm.objects[index] else {
            return Err(Fault::Error("VM2 insert-file-contents into a unibyte buffer is unsupported".into()));
        };
        let at = t.point;
        let before = t.len_chars();
        t.replace(at..at, text.as_bytes()).map_err(Fault::Error)?;
        t.point = at;
        let inserted = t.len_chars() - before;
        if inserted > 0 {
            self.vm.display_state.version += 1;
        }
        Ok(inserted)
    }

    /// The UTF-8 bytes of characters FROM..TO (1-based, FROM <= TO, already
    /// validated against the widened buffer) of the current multibyte buffer.
    pub fn buffer_text_bytes(&mut self, from: i64, to: i64) -> std::result::Result<Vec<u8>, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else {
            return Err(Fault::Error("VM2 write-region from a unibyte buffer is unsupported".into()));
        };
        let char_of = |p: i64| CharOffset((p - 1).max(0) as usize);
        let a: ByteOffset = t.byte_of_char(char_of(from)).map_err(Fault::Error)?;
        let b: ByteOffset = t.byte_of_char(char_of(to)).map_err(Fault::Error)?;
        Ok(t.bytes()[a.0..b.0].to_vec())
    }

    /// GNU `Ffboundp`: the function cell is not void.
    pub fn fboundp(&self, symbol: Value) -> bool {
        self.symbol_function(symbol).is_ok_and(|f| f != Value::NIL)
    }

    /// GNU `bset_<field> (current_buffer, VALUE)` for a per-buffer C variable.
    pub fn bset(&mut self, name: &str, value: Value) -> std::result::Result<(), Fault> {
        self.vm.bset_current(name, value)
    }
}
