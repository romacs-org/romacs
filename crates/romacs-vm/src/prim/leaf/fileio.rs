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
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(()) };
        let at = t.point;
        self.check_insertion_read_only(at, inhibit)
    }

    /// verify_interval_modification for an insertion at AT: a `read-only'
    /// character on either side whose property would stick to the new text.
    pub fn check_insertion_read_only(&mut self, at: ByteOffset, inhibit: Value) -> std::result::Result<(), Fault> {
        if inhibit != Value::NIL && self.kind(inhibit) != Kind::Cons { return Ok(()) }
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &self.vm.objects[index] else { return Ok(()) };
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

    /// Insert decoded text at point of the current buffer and leave point
    /// BEFORE it, as `insert-file-contents' does (it inserts into the gap at
    /// PT and never moves PT). BYTES are internal multibyte text when
    /// MULTIBYTE, else octets; a multibyte buffer takes octets above 127 as
    /// raw-byte characters, a unibyte buffer takes octets only. Markers follow
    /// their insertion type. The insertion is recorded for undo
    /// (insert_from_gap's record_insert). Returns the number of characters
    /// inserted.
    pub fn insert_before_point(&mut self, bytes: &[u8], multibyte: bool) -> std::result::Result<usize, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let unmodified = !self.vm.buffer_state(Value::reference(index + 1, 0)).map_err(Fault::Error)?.modified();
        let (inserted, at) = match &mut self.vm.objects[index] {
            data::Object::Buffer(t) => {
                let internal;
                let bytes = if multibyte { bytes } else {
                    internal = octets_to_multibyte(bytes);
                    &internal
                };
                // VM2 buffer text is Unicode: no raw-byte characters.
                if std::str::from_utf8(bytes).is_err() {
                    return Err(fileio_refusal("insert-file-contents of raw bytes into a multibyte buffer"));
                }
                let at = t.point;
                let before = t.len_chars();
                t.replace(at..at, bytes).map_err(Fault::Error)?;
                t.point = at;
                (t.len_chars() - before, t.char_of_byte(at).map_err(Fault::Error)?.0 as i64 + 1)
            }
            data::Object::ByteBuffer(b) => {
                if multibyte { return Err(Fault::Error("multibyte text for a unibyte buffer".into())); }
                let at = b.point;
                b.replace(at..at, bytes).map_err(Fault::Error)?;
                b.point = at;
                (bytes.len(), at.0 as i64 + 1)
            }
            _ => return Err(Fault::Error("buffer required".into())),
        };
        if inserted > 0 {
            self.vm.display_state.version += 1;
            self.vm.record_insert(index, at, inserted as i64, unmodified).map_err(Fault::Error)?;
        }
        Ok(inserted)
    }

    /// The text properties of decoded STRING (the `charset' runs of a
    /// charset decoder, coding.c produce_charset) onto the text just
    /// inserted at point of the current multibyte buffer, byte for byte.
    pub fn fileio_copy_string_properties(&mut self, string: Value) -> std::result::Result<(), Fault> {
        let Some(data::Object::String(s) | data::Object::RawString(s)) = self.vm.object_if(string) else { return Ok(()) };
        let map = s.properties.to_map();
        if map.is_empty() { return Ok(()) }
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::Buffer(t) = &mut self.vm.objects[index] else { return Ok(()) };
        let at = t.point.0;
        // to_map gives each byte's properties; put them back as runs.
        let mut runs: Vec<(usize, usize, &std::collections::BTreeMap<String, Value>)> = vec![];
        for (offset, values) in &map {
            match runs.last_mut() {
                Some((_, end, run)) if *end == offset.0 && *run == values => *end += 1,
                _ => runs.push((offset.0, offset.0 + 1, values)),
            }
        }
        for (start, end, values) in runs {
            for (key, value) in values {
                t.properties.put(ByteOffset(at + start)..ByteOffset(at + end), key, *value);
            }
        }
        Ok(())
    }

    /// GNU `bset_enable_multibyte_characters' on the current buffer while it
    /// holds no text: only the representation changes. A unibyte buffer holds
    /// no markers, so the empty multibyte buffer, whose markers and overlays
    /// all stay at BEG, is held aside until the toggle back.
    pub fn fileio_set_multibyte_flag(&mut self, multibyte: bool) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let replacement = match &self.vm.objects[index] {
            data::Object::Buffer(t) if !multibyte => {
                if t.len_bytes() != 0 {
                    return Err(fileio_refusal("insert-file-contents unibyte toggle of a buffer with text"));
                }
                let mut b = t.binary();
                b.state = t.state.clone();
                b.per_buffer = t.per_buffer.clone();
                b.case_tables = t.case_tables;
                // Its markers, all at its start, live in the unibyte buffer
                // meanwhile; the held buffer keeps its overlays.
                let mut held = t.clone();
                b.anchors = std::mem::replace(&mut held.anchors, crate::anchors::AnchorTree::new(0));
                b.held = Some(held);
                data::Object::ByteBuffer(Box::new(b))
            }
            data::Object::ByteBuffer(b) if multibyte => {
                if !b.bytes.is_empty() {
                    return Err(fileio_refusal("insert-file-contents multibyte toggle of a buffer with text"));
                }
                let mut a: crate::annotations::AnnotatedBuffer = match &b.held {
                    Some(held) => (**held).clone(),
                    None => crate::text::Text::decode_binary(b).map_err(Fault::Error)?.into(),
                };
                a.anchors = b.anchors.clone();
                a.state = b.state.clone();
                a.per_buffer = b.per_buffer.clone();
                a.case_tables = b.case_tables;
                a.syntax_table = b.syntax_table;
                a.category_table = b.category_table;
                data::Object::Buffer(Box::new(a))
            }
            _ => return Ok(()),
        };
        self.vm.objects[index] = replacement;
        Ok(())
    }

    /// GNU insert_from_gap_1 into the empty current unibyte buffer: BYTES
    /// become its text with point at BEG, recording neither a modification
    /// nor undo (decide_coding_unwind takes them back).
    pub fn fileio_fill_raw(&mut self, bytes: &[u8]) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let data::Object::ByteBuffer(b) = &mut self.vm.objects[index] else {
            return Err(Fault::Error("unibyte buffer required".into()));
        };
        debug_assert!(b.bytes.is_empty());
        // Its markers stay at BEG, as insert_from_gap_1 leaves them.
        if !b.anchors.grow(u32::try_from(bytes.len()).map_err(|_| Fault::Error("buffer too large".into()))?) {
            return Err(Fault::Error("buffer too large".into()));
        }
        b.bytes = bytes.to_vec();
        b.point = ByteOffset(0);
        b.restriction = ByteOffset(0)..ByteOffset(bytes.len());
        Ok(())
    }

    /// decide_coding_unwind's text half: move every byte of the current
    /// buffer back out (markers and point return to BEG, properties go)
    /// without recording a modification, and return them. A buffer Lisp made
    /// multibyte gives back its internal bytes, as GNU's gap does.
    pub fn fileio_take_raw(&mut self) -> std::result::Result<Vec<u8>, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match &mut self.vm.objects[index] {
            data::Object::ByteBuffer(b) => {
                let (state, bytes) = (b.state.clone(), b.bytes.clone());
                b.restriction = ByteOffset(0)..ByteOffset(bytes.len());
                b.replace(ByteOffset(0)..ByteOffset(bytes.len()), &[]).map_err(Fault::Error)?;
                b.state = state;
                Ok(bytes)
            }
            data::Object::Buffer(t) => {
                let (state, bytes) = (t.state.clone(), t.bytes().to_vec());
                t.widen();
                t.replace(ByteOffset(0)..ByteOffset(bytes.len()), &[]).map_err(Fault::Error)?;
                t.state = state;
                Ok(bytes)
            }
            _ => Err(Fault::Error("buffer required".into())),
        }
    }

    /// The bytes of characters FROM..TO (1-based, FROM <= TO, already
    /// validated against the widened buffer) of the current buffer: the
    /// internal UTF-8 of a multibyte buffer, the octets of a unibyte one.
    pub fn buffer_text_bytes(&mut self, from: i64, to: i64) -> std::result::Result<Vec<u8>, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let char_of = |p: i64| CharOffset((p - 1).max(0) as usize);
        match &self.vm.objects[index] {
            data::Object::Buffer(t) => {
                let a: ByteOffset = t.byte_of_char(char_of(from)).map_err(Fault::Error)?;
                let b: ByteOffset = t.byte_of_char(char_of(to)).map_err(Fault::Error)?;
                Ok(t.bytes()[a.0..b.0].to_vec())
            }
            data::Object::ByteBuffer(b) => Ok(b.bytes[char_of(from).0..char_of(to).0].to_vec()),
            _ => Err(Fault::Error("buffer required".into())),
        }
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

fn fileio_refusal(what: &str) -> Fault {
    Fault::Error(format!("VM2 {what} is unsupported"))
}

/// Octets as multibyte internal text: those above 127 become raw-byte
/// characters (GNU str_to_multibyte).
fn octets_to_multibyte(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte < 128 { out.push(byte); continue; }
        let mut encoded = [0; 5];
        let width = crate::lstring::char_string(crate::lstring::BYTE8_OFFSET + u32::from(byte), &mut encoded);
        out.extend_from_slice(&encoded[..width]);
    }
    out
}
