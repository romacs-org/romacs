//! Lisp positions are one-based scalars; retained buffer state is byte-canonical.
use crate::{coordinates::*, data::Object, text::Text, *};
fn position(v: Value) -> Result<CharOffset> {
    usize::try_from(v.as_integer()?)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .map(CharOffset)
        .ok_or_else(|| "index out of range".into())
}
#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn named_temporary_buffer_dies_on_normal_error_and_throw_exits() {
        for source in [
            "(with-temp-buffer (setq escaped-temp (current-buffer)))",
            "(condition-case nil (with-temp-buffer (setq escaped-temp (current-buffer)) (error \"unwind\")) (error nil))",
            "(catch 'escape (with-temp-buffer (setq escaped-temp (current-buffer)) (throw 'escape 7)))",
        ] {
            let mut vm = Vm::new();
            vm.eval_source_raw(source).unwrap();
            let slot = vm.slot("escaped-temp");
            let buffer = vm.slots[slot].value;
            assert!(!vm.buffer_state(buffer).unwrap().live, "{source}");
            assert!(vm.buffer_state(buffer).unwrap().name.is_none());
            assert!(vm.require_live_buffer(buffer).is_err());
            assert!(vm.named_buffer(" *temp*").is_none());
        }
    }

    #[test]
    fn killed_buffer_loses_symbol_local_bindings() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq lifecycle-buffer (get-buffer-create \"locals-cleanup\" t)) (with-current-buffer lifecycle-buffer (set (make-local-variable 'lifecycle-local) 9))").unwrap();
        let buffer_slot = vm.slot("lifecycle-buffer");
        let buffer = vm.slots[buffer_slot].value;
        let variable = vm.slot("lifecycle-local");
        let table = vm.slots[variable].local;
        assert!(vm.declaration_hash_get(table, buffer).unwrap().is_some(), "BUFFER_LIFECYCLE_LOCALS_FIXTURE");
        vm.eval_source_raw("(kill-buffer lifecycle-buffer)").unwrap();
        let buffer = vm.slots[buffer_slot].value;
        assert!(vm.declaration_hash_get(table, buffer).unwrap().is_none(), "BUFFER_LIFECYCLE_LOCALS_CLEARED");
    }

    #[test]
    fn buffer_identity_and_ticks_survive_representation_changes_and_collection() {
        let mut vm = Vm::new();
        let buffer = vm.create_named_buffer("model-test", false).unwrap();
        vm.slots[vm.buffer_slot].value = buffer;
        vm.eval_source_raw("(insert \"abc\") (put-text-property 1 2 'model-property t)").unwrap();
        let buffer = vm.slots[vm.buffer_slot].value;
        assert_eq!((vm.buffer_state(buffer).unwrap().modiff, vm.buffer_state(buffer).unwrap().chars_modiff), (4, 3));
        vm.convert_buffer_utf8(false).unwrap();
        vm.collect(&mut []);
        let buffer = vm.slots[vm.buffer_slot].value;
        let state = vm.buffer_state(buffer).unwrap();
        assert_eq!(state.name.as_deref(), Some("model-test"));
        assert_eq!((state.modiff, state.chars_modiff), (4, 3));
        assert!(state.live && state.modified());
        vm.convert_buffer_utf8(true).unwrap();
        assert_eq!(vm.named_buffer("model-test"), Some(vm.slots[vm.buffer_slot].value));
    }
}

impl Vm {
    pub(crate) fn buffer_state(&self, value: Value) -> Result<&bytes::BufferState> {
        match self.object(value)? {
            Object::Buffer(b) => Ok(&b.state),
            Object::ByteBuffer(b) => Ok(&b.state),
            _ => Err("buffer required".into()),
        }
    }
    pub(crate) fn buffer_state_mut(&mut self, value: Value) -> Result<&mut bytes::BufferState> {
        match self.objects.get_mut(value.index(0)?.checked_sub(1).ok_or("buffer required")?).ok_or("buffer required")? {
            Object::Buffer(b) => Ok(&mut b.state),
            Object::ByteBuffer(b) => Ok(&mut b.state),
            _ => Err("buffer required".into()),
        }
    }
    pub(crate) fn require_live_buffer(&self, value: Value) -> Result<()> {
        if self.buffer_state(value)?.live { Ok(()) }
        else { Err("Selecting deleted buffer".into()) }
    }
    pub(crate) fn named_buffer(&mut self, name: &str) -> Option<Value> {
        let slot = self.slot(&format!("#:vm2-buffer:{name}"));
        let value = self.slots[slot].value;
        self.buffer_state(value).ok().filter(|b| b.live && b.name.as_deref() == Some(name)).map(|_| value)
    }
    pub(crate) fn create_named_buffer(&mut self, name: &str, unique: bool) -> Result<Value> {
        if name.is_empty() { return Err("empty buffer name".into()); }
        let mut actual = name.to_owned();
        let mut suffix = 2;
        while let Some(value) = self.named_buffer(&actual) {
            if !unique { return Ok(value); }
            actual = format!("{name}<{suffix}>");
            suffix += 1;
        }
        let mut buffer: crate::annotations::AnnotatedBuffer = Text::new(b"")?.into();
        buffer.state.name = Some(actual.clone());
        buffer.case_tables = self.buffer_standard_case_roots()?;
        let value = self.allocate_object(Object::Buffer(Box::new(buffer)));
        let slot = self.slot(&format!("#:vm2-buffer:{actual}"));
        self.slots[slot].value = value;
        Ok(value)
    }
    pub(crate) fn kill_buffer_model(&mut self, value: Value) -> Result<()> {
        if !self.buffer_state(value)?.live || self.objects.iter().any(|object| {
            matches!(object, Object::Window(w) if w.mini && w.buffer == value)
        }) { return Ok(()); }
        for (name, slot) in &self.names {
            if name.starts_with("#:vm2-buffer:") && self.slots[*slot].value == value {
                self.slots[*slot].value = Value::UNBOUND;
            }
        }
        let windows: Vec<Value> = self.objects.iter().enumerate().filter_map(|(i, object)| {
            match object {
                Object::Window(w) if w.buffer == value => Some(Value::reference(i + 1, 0)),
                _ => None,
            }
        }).collect();
        if self.slots[self.buffer_slot].value == value || !windows.is_empty() {
            let replacement = self.names.iter().filter_map(|(name, slot)| {
                let candidate = self.slots[*slot].value;
                if name.starts_with("#:vm2-buffer:")
                    && candidate != value
                    && (windows.is_empty() || matches!(self.object_if(candidate), Some(Object::Buffer(_))))
                    && self.buffer_state(candidate).is_ok_and(|state| {
                        state.live && state.name.as_deref().is_some_and(|n| !n.starts_with(' '))
                    })
                    && !self.objects.iter().any(|object| {
                        matches!(object, Object::Window(w) if w.mini && w.buffer == candidate)
                    })
                {
                    Some((*slot, candidate))
                } else {
                    None
                }
            }).min_by_key(|(slot, _)| *slot).map(|(_, candidate)| candidate)
                .map(Ok).unwrap_or_else(|| self.create_named_buffer("*scratch*", true))?;
            let point = match self.object(replacement)? {
                Object::Buffer(b) => b.char_of_byte(b.point)?.0,
                Object::ByteBuffer(b) => b.point.0,
                _ => unreachable!(),
            };
            let point = Value::integer((point + 1) as i64)?;
            for window in windows {
                let (pt, start) = (self.window(window)?.point, self.window(window)?.start);
                self.set_marker_object(pt, point, replacement)?;
                self.set_marker_object(start, Value::integer(1)?, replacement)?;
                let w = self.window_mut(window)?;
                w.buffer = replacement;
                w.end_distance = 0;
                w.end_valid = false;
                w.vscroll_px = 0;
            }
            if self.slots[self.buffer_slot].value == value {
                self.slots[self.buffer_slot].value = replacement;
            }
        }
        let tables: Vec<Value> = self.slots.iter().map(|s| s.local)
            .filter(|&v| v != Value::NIL).collect();
        for table in tables { self.declaration_hash_remove(table, value)?; }
        for object in &mut self.objects {
            match object {
                Object::Marker(m) if m.buffer == value => { m.buffer = Value::NIL; m.anchor = None; }
                Object::Overlay(o) if o.buffer == value => {
                    o.buffer = Value::NIL; o.start = None; o.end = None;
                }
                _ => {}
            }
        }
        // GNU kill-buffer resets ordinary locals but retains the visited filename.
        // Reuse the native per-buffer storage without retaining other Lisp locals.
        let fields = match &self.objects[value.index(0)? - 1] {
            Object::Buffer(b) => &b.per_buffer,
            Object::ByteBuffer(b) => &b.per_buffer,
            _ => unreachable!(),
        };
        let has_filename = ["buffer-file-name", "buffer-file-truename"].into_iter().any(|name| match self.forwarded.lookup(name) {
            Some(crate::forwarded::Fwd::Buffer(i)) => fields.get(i).is_some_and(|v| v != Value::NIL),
            _ => false,
        });
        let mut retained = if has_filename { fields.clone() } else { Default::default() };
        for name in crate::forwarded::Forwarded::names() {
            if name != "buffer-file-name" && name != "buffer-file-truename" {
                if let Some(crate::forwarded::Fwd::Buffer(i)) = self.forwarded.lookup(name) {
                    retained.clear_local(i);
                }
            }
        }
        let mut state = self.buffer_state(value)?.clone();
        state.kill();
        let index = value.index(0)? - 1;
        self.objects[index] = match &self.objects[index] {
            Object::ByteBuffer(_) => Object::ByteBuffer(Box::new(bytes::ByteBuffer {
                state, bytes: vec![], syntax_table: Value::NIL, category_table: Value::NIL, case_tables: [Value::NIL; 4],
                point: ByteOffset(0), restriction: ByteOffset(0)..ByteOffset(0),
                properties: Default::default(), per_buffer: retained,
            })),
            _ => {
                let mut empty: crate::annotations::AnnotatedBuffer = Text::new(b"")?.into();
                empty.state = state;
                empty.per_buffer = retained;
                Object::Buffer(Box::new(empty))
            }
        };
        self.display_state.version += 1;
        Ok(())
    }
    fn string_property_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        object: Value,
    ) -> Result<Value> {
        let key_value = self.stack[start + if p == Primitive::PutProperty { 2 } else { 1 }];
        let slot = key_value
            .index(7)?
            .checked_sub(1)
            .ok_or("symbol required")?;
        let key = self
            .names
            .iter()
            .find(|(_, v)| **v == slot)
            .ok_or("symbol required")?
            .0
            .clone();
        let at =
            usize::try_from(self.stack[start].as_integer()?).map_err(|_| "index out of range")?;
        if Self::image_value(object) {
            let text = self.string_value(object)?;
            if p == Primitive::GetProperty && at <= text.chars().count() {
                return Ok(Value::NIL);
            }
            return Err("immutable image string or index out of range".into());
        }
        let end = if p == Primitive::PutProperty {
            Some(
                usize::try_from(self.stack[start + 1].as_integer()?)
                    .map_err(|_| "index out of range")?,
            )
        } else {
            None
        };
        let value = if p == Primitive::PutProperty {
            self.stack[start + 3]
        } else {
            Value::NIL
        };
        let (Object::String(s) | Object::RawString(s)) = &mut self.objects[object.index(0)? - 1] else {
            return Err("string required".into());
        };
        let b = s.byte_offset(CharOffset(at)).ok_or("index out of range")?;
        if let Some(end) = end {
            let e = s.byte_offset(CharOffset(end)).ok_or("index out of range")?;
            s.properties
                .put(ByteOffset(b.min(e))..ByteOffset(b.max(e)), &key, value);
            Ok(Value::NIL)
        } else {
            if at == s.len() {
                return Ok(Value::NIL);
            }
            Ok(s.properties
                .get_property(ByteOffset(b), &key)
                .unwrap_or(Value::NIL))
        }
    }
    pub(super) fn string_bytes(&self, v: Value) -> Result<Vec<u8>> {
        if Self::image_value(v) {
            return Ok(self.string_value(v)?.as_bytes().to_vec());
        }
        match self.object(v)? {
            Object::String(s) => Ok(s.as_str().as_bytes().to_vec()),
            Object::Bytes(b) if b.is_ascii() => Ok(b.clone()),
            Object::RawString(_) | Object::Bytes(_) => Err("not-unicode-string".into()),
            _ => Err("string required".into()),
        }
    }
    pub(super) fn buffer_text_p(&self) -> bool {
        let current = self.slots[self.buffer_slot].value;
        current == Value::NIL || matches!(self.object_if(current), Some(Object::Buffer(_)))
    }
    pub(super) fn buffer_index(&mut self) -> Result<usize> {
        if self.slots[self.buffer_slot].value == Value::NIL {
            self.slots[self.buffer_slot].value = self.create_named_buffer("*scratch*", false)?;
        }
        let v = self.slots[self.buffer_slot].value;
        self.require_live_buffer(v)?;
        Ok(v.index(0)? - 1)
    }
    pub(super) fn ensure_buffer_writable(&mut self) -> Result<()> {
        self.buffer_index()?;
        let read_only = self.slot("buffer-read-only");
        let inhibited = self.slot("inhibit-read-only");
        if self.variable_value(read_only)? != Value::NIL
            && self.variable_value(inhibited)? == Value::NIL
        {
            return Err("buffer mutation refused: read-only".into());
        }
        Ok(())
    }
    pub(super) fn buffer_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        let result = self.buffer_primitive_inner(p, start, n);
        if result.is_ok()
            && matches!(
                p,
                Primitive::Insert
                    | Primitive::DeleteRegion
                    | Primitive::PutProperty
                    | Primitive::GotoChar
                    | Primitive::Narrow
                    | Primitive::Widen
            )
        {
            self.display_state.version += 1;
        }
        result
    }
    fn buffer_primitive_inner(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if (p as usize) >= MakeMarker as usize {
            return self.annotation_primitive(p, start, n);
        }
        if (p as usize) >= SetMultibyte as usize {
            return self.edge_primitive(p, start, n);
        }
        let args = self.buffer_primitive_args(p, start, n)?;
        let valid = match p {
            Remainder | DeleteRegion | Substring | Narrow => n == 2,
            SetBuffer | GotoChar | CharToByte | ByteToChar => n == 1,
            CurrentBuffer | Point | PointMin | PointMax | BufferSize | Widen => {
                n == 0
            }
            CharAfter => n <= 1,
            Insert => true,
            PutProperty => (4..=5).contains(&n),
            GetProperty => (2..=3).contains(&n),
            _ => false,
        };
        if !valid {
            return Err("wrong primitive arity".into());
        }
        if p == Remainder {
            return Value::integer(
                args[0]
                    .as_integer()?
                    .checked_rem(args[1].as_integer()?)
                    .ok_or("arithmetic error")?,
            );
        }
        if p == SetBuffer {
            let mut v = args[0];
            if let Some(name) = self.eval_buffer_name(v)? {
                v = if name == "*scratch*" { self.create_named_buffer(&name, false)? }
                    else { self.named_buffer(&name).ok_or_else(|| format!("no buffer named {name}"))? };
            }
            self.require_live_buffer(v)?;
            self.slots[self.buffer_slot].value = v;
            return Ok(v);
        }
        let object = if (p == PutProperty && n == 5) || (p == GetProperty && n == 3) {
            args[n - 1]
        } else {
            Value::NIL
        };
        if object != Value::NIL
            && (Self::image_value(object) || matches!(self.object(object), Ok(Object::String(_) | Object::RawString(_))))
        {
            return self.string_property_primitive(p, start, object);
        }
        let index = if object != Value::NIL {
            if !matches!(
                self.object(object)?,
                Object::Buffer(_) | Object::ByteBuffer(_)
            ) {
                return Err("buffer or string required".into());
            }
            object.index(0)? - 1
        } else {
            self.buffer_index()?
        };
        if p == CurrentBuffer {
            return Ok(self.slots[self.buffer_slot].value);
        }
        if matches!(p, Insert | DeleteRegion) {
            self.ensure_buffer_writable()?;
        }
        if matches!(self.objects[index], Object::ByteBuffer(_)) {
            return self.byte_buffer_primitive(p, start, n, index, &args);
        }
        let mut inserted_properties = properties::Properties::new();
        let inserted = if p == Insert {
            let mut bytes = vec![];
            for v in &self.stack[start..start + n] {
                if let Some(code) = v.integer_value() {
                    let ch = u32::try_from(code)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or("invalid character")?;
                    bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                } else {
                    if !Self::image_value(*v)
                        && let Ok(Object::String(s)) = self.object(*v)
                    {
                        inserted_properties.copy_into(ByteOffset(bytes.len()), &s.properties);
                    }
                    bytes.extend(self.string_bytes(*v)?);
                }
            }
            bytes
        } else {
            vec![]
        };
        let key = if matches!(p, PutProperty | GetProperty) {
            let value = args[if p == PutProperty { 2 } else { 1 }];
            let slot = value.index(7)?.checked_sub(1).ok_or("symbol required")?;
            Some(
                self.names
                    .iter()
                    .find(|(_, v)| **v == slot)
                    .ok_or("symbol required")?
                    .0
                    .clone(),
            )
        } else {
            None
        };
        // Fchar_after: a marker's byte position, from the marker's own
        // buffer, is read against this buffer's accessible region.
        let char_after_marker = if p == CharAfter && n > 0 {
            match self.object_if(args[0]) {
                Some(Object::Marker(marker)) => {
                    let anchor = marker.anchor.ok_or("Marker does not point anywhere")?;
                    let Some(Object::Buffer(owner)) = self.object_if(marker.buffer) else {
                        return Err("Marker does not point anywhere".into());
                    };
                    Some(ByteOffset(owner.anchors.resolve(anchor).ok_or("Marker does not point anywhere")? as usize))
                }
                Some(Object::Bignum(_)) => return Ok(Value::NIL),
                _ => None,
            }
        } else {
            None
        };
        let Object::Buffer(t) = &mut self.objects[index] else {
            unreachable!()
        };
        let accessible = t.accessible();
        let region = |a: Value, b: Value| -> Result<std::ops::Range<ByteOffset>> {
            let a = t.byte_of_char(position(a)?)?;
            let b = t.byte_of_char(position(b)?)?;
            let r = a.min(b)..a.max(b);
            if r.start < accessible.start || r.end > accessible.end {
                return Err("index out of range".into());
            }
            Ok(r)
        };
        match p {
            Point => Value::integer((t.char_of_byte(t.point)?.0 + 1) as i64),
            PointMin => Value::integer((t.char_of_byte(accessible.start)?.0 + 1) as i64),
            PointMax => Value::integer((t.char_of_byte(accessible.end)?.0 + 1) as i64),
            BufferSize => Value::integer(t.len_chars() as i64),
            GotoChar => {
                let c = args[0].as_integer()?.saturating_sub(1).max(0) as usize;
                t.point = t
                    .byte_of_char(CharOffset(c.min(t.len_chars())))?
                    .clamp(accessible.start, accessible.end);
                Ok(self.stack[start])
            }
            CharAfter => {
                let b = if let Some(b) = char_after_marker {
                    b
                } else if n == 0 || args[0] == Value::NIL {
                    t.point
                } else {
                    let c = args[0].as_integer()?;
                    if c < 1 {
                        return Ok(Value::NIL);
                    }
                    let Ok(b) = t.byte_of_char(CharOffset((c - 1) as usize)) else {
                        return Ok(Value::NIL);
                    };
                    b
                };
                if !accessible.contains(&b) {
                    return Ok(Value::NIL);
                }
                t.character_at(b)?
                    .map(Value::integer)
                    .unwrap_or(Ok(Value::NIL))
            }
            Insert => {
                let at = t.point;
                t.replace(at..at, &inserted)?;
                t.properties.copy_into(at, &inserted_properties);
                Ok(Value::NIL)
            }
            DeleteRegion => {
                let r = region(args[0], args[1])?;
                t.replace(r, b"")?;
                self.finish_annotation_edit(self.slots[self.buffer_slot].value)?;
                Ok(Value::NIL)
            }
            Substring => {
                let r = region(args[0], args[1])?;
                let string = Box::new(data::LispString::new(t.slice(r)?));
                Ok(self.allocate_object(Object::String(string)))
            }
            Narrow => {
                let a = t.byte_of_char(position(args[0])?)?;
                let b = t.byte_of_char(position(args[1])?)?;
                t.narrow(a.min(b)..a.max(b))?;
                Ok(Value::NIL)
            }
            Widen => {
                t.widen();
                Ok(Value::NIL)
            }
            CharToByte | ByteToChar => {
                let value = args[0].as_integer()?;
                if value < 1 {
                    return Ok(Value::NIL);
                }
                let at = (value - 1) as usize;
                let result = if p == CharToByte {
                    t.byte_of_char(CharOffset(at)).map(|b| b.0)
                } else {
                    let mut b = at;
                    if b <= t.len_bytes() {
                        while b > 0 && b < t.len_bytes() && t.bytes()[b] & 0xc0 == 0x80 {
                            b -= 1;
                        }
                    }
                    t.char_of_byte(ByteOffset(b)).map(|c| c.0)
                };
                result
                    .map(|p| Value::integer((p + 1) as i64))
                    .unwrap_or(Ok(Value::NIL))
            }
            PutProperty => {
                let r = region(args[0], args[1])?;
                t.put_property(r, key.as_ref().unwrap(), args[3]);
                Ok(Value::NIL)
            }
            GetProperty => {
                let at = t.byte_of_char(position(args[0])?)?;
                if at < accessible.start || at > accessible.end {
                    return Err("index out of range".into());
                }
                Ok(t.properties
                    .get_property(at, key.as_ref().unwrap())
                    .unwrap_or(Value::NIL))
            }
            _ => unreachable!(),
        }
    }
}

impl Vm {
    /// I/O embeddings can retain arbitrary octets without passing through text.
    /// The arguments of a legacy buffer primitive, a marker in a position
    /// slot replaced by its position (CHECK_FIXNUM_COERCE_MARKER); other
    /// arguments, such as a property value, are left alone.
    fn buffer_primitive_args(&self, p: Primitive, start: usize, n: usize) -> Result<Vec<Value>> {
        use Primitive::*;
        let positions: &[usize] = match p {
            GotoChar | CharToByte | GetProperty => &[0],
            DeleteRegion | Substring | Narrow | PutProperty => &[0, 1],
            _ => &[],
        };
        let mut args = self.stack[start..start + n].to_vec();
        for &slot in positions {
            if let Some(&value) = args.get(slot)
                && matches!(self.object_if(value), Some(Object::Marker(_)))
            {
                let position = self.marker_position_value(value)?;
                if position == Value::NIL {
                    return Err("Marker does not point anywhere".into());
                }
                args[slot] = position;
            }
        }
        Ok(args)
    }
    pub(crate) fn byte_string(&mut self, bytes: &[u8]) -> Value {
        self.allocate_object(Object::Bytes(bytes.to_vec()))
    }
    pub(crate) fn bytes_value(&self, v: Value) -> Result<&[u8]> {
        match self.object(v)? {
            Object::Bytes(b) => Ok(b),
            _ => Err("bytes required".into()),
        }
    }
    fn edge_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if matches!(p, EncodeBufferUtf8 | DecodeBufferUtf8) {
            if n != 0 {
                return Err("wrong primitive arity".into());
            }
            return self.convert_buffer_utf8(p == DecodeBufferUtf8);
        }
        if p == ByteString {
            let b = self.stack[start..start + n]
                .iter()
                .map(|v| u8::try_from(v.as_integer()?).map_err(|_| "invalid octet".into()))
                .collect::<Result<Vec<_>>>()?;
            return Ok(self.allocate_object(Object::Bytes(b)));
        }
        if n != if p == BufferString { 0 } else { 1 } {
            return Err("wrong primitive arity".into());
        }
        if matches!(p, EncodeUtf8 | DecodeUtf8 | StringToBytes | IsMultibyte) {
            let v = self.stack[start];
            if p == IsMultibyte {
                return Ok(if self.str_view(v).is_some_and(|view|view.is_multibyte()) { Value::TRUE } else { Value::NIL });
            }
            let view = self.str_view(v).ok_or("string required")?.to_owned();
            if p == DecodeUtf8 {
                if view.is_multibyte() { return Ok(v); }
                let mut bytes = Vec::new();
                let mut tail = view.bytes();
                while !tail.is_empty() {
                    if let Some((_, width)) = lstring::decode_char(tail).filter(|_|tail[0] >= 0xc2 || tail[0] < 0x80) {
                        bytes.extend_from_slice(&tail[..width]); tail = &tail[width..];
                    } else {
                        let code = if tail[0] >= 128 { tail[0] as u32 + lstring::BYTE8_OFFSET } else { tail[0] as u32 };
                        let mut encoded = [0;5]; let n = lstring::char_string(code, &mut encoded);
                        bytes.extend_from_slice(&encoded[..n]); tail = &tail[1..];
                    }
                }
                return Ok(self.make_lisp_string(bytes, true));
            }
            if !view.is_multibyte() { return Ok(v); }
            let b = if p == StringToBytes {
                let mut bytes = Vec::with_capacity(view.len());
                for (index, code) in view.chars().enumerate() {
                    let byte = if code > lstring::MAX_5_BYTE_CHAR { code - lstring::BYTE8_OFFSET } else { code };
                    if code >= 128 && code <= lstring::MAX_5_BYTE_CHAR { return Err(format!("Cannot convert character at index {index} to unibyte")); }
                    bytes.push(byte as u8);
                }
                bytes
            } else {
                let mut bytes = Vec::new();
                for code in view.chars() {
                    if code > lstring::MAX_5_BYTE_CHAR { bytes.push((code-lstring::BYTE8_OFFSET) as u8); }
                    else { let mut encoded=[0;5]; let n=lstring::char_string(code,&mut encoded); bytes.extend_from_slice(&encoded[..n]); }
                }
                bytes
            };
            return Ok(self.allocate_object(Object::Bytes(b)));
        }
        let index = self.buffer_index()?;
        if p == BufferString {
            let o = match &self.objects[index] {
                Object::Buffer(t) => {
                    let accessible = t.accessible();
                    let mut s = data::LispString::new(t.slice(accessible.clone())?);
                    s.properties = t.properties.slice(accessible);
                    Object::String(Box::new(s))
                }
                Object::ByteBuffer(b) => {
                    Object::Bytes(b.bytes[b.restriction.start.0..b.restriction.end.0].to_vec())
                }
                _ => unreachable!(),
            };
            return Ok(self.allocate_object(o));
        }
        if p == InsertFileBytes {
            self.ensure_buffer_writable()?;
            let replacement = match &self.objects[index] {
                Object::Buffer(t) => {
                    if !t.anchors.is_empty() {
                        return Err("cannot encode buffer with registered anchors".into());
                    }
                    {
                    // Per-buffer variables belong to the buffer, not its representation.
                    let mut b = t.binary();
                    b.state = t.state.clone();
                    b.per_buffer = t.per_buffer.clone();
                    b.case_tables = t.case_tables;
                    Some(Object::ByteBuffer(Box::new(b)))
                }
                }
                _ => None,
            };
            if let Some(o) = replacement {
                self.objects[index] = o;
            }
            if n != 1 {
                return Err("wrong primitive arity".into());
            }
            let args = self.stack[start..start + n].to_vec();
            let result = self.byte_buffer_primitive(InsertFileBytes, start, n, index, &args);
            if result.is_ok() {
                self.display_state.version += 1;
            }
            return result;
        }
        self.convert_buffer_utf8(self.stack[start] != Value::NIL)
    }
    fn convert_buffer_utf8(&mut self, text: bool) -> Result<Value> {
        let index = self.buffer_index()?;
        // This is an explicit text/byte boundary, not a second text encoding.
        // Conversion preserves octets and byte-canonical metadata or fails atomically.
        let replacement = match (&self.objects[index], text) {
            (Object::Buffer(t), false) => {
                if !t.anchors.is_empty() {
                    return Err("cannot encode buffer with registered anchors".into());
                }
                {
                    // Per-buffer variables belong to the buffer, not its representation.
                    let mut b = t.binary();
                    b.state = t.state.clone();
                    b.per_buffer = t.per_buffer.clone();
                    b.case_tables = t.case_tables;
                    Some(Object::ByteBuffer(Box::new(b)))
                }
            }
            (Object::ByteBuffer(b), true) => {
                {
                let mut a: crate::annotations::AnnotatedBuffer = Text::decode_binary(b)?.into();
                a.state = b.state.clone();
                a.per_buffer = b.per_buffer.clone();
                a.case_tables = b.case_tables;
                Some(Object::Buffer(Box::new(a)))
            }
            }
            _ => None,
        };
        if let Some(o) = replacement {
            self.objects[index] = o;
        }
        Ok(if text { Value::TRUE } else { Value::NIL })
    }
    fn byte_buffer_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
        index: usize,
        args: &[Value],
    ) -> Result<Value> {
        use Primitive::*;
        let inserted = if p == Insert {
            let mut out = vec![];
            for &v in &self.stack[start..start + n] {
                if let Some(i) = v.integer_value() {
                    out.push(u8::try_from(i).map_err(|_| "invalid octet")?);
                } else {
                    match self.object(v)? {
                        Object::Bytes(b) => out.extend_from_slice(b),
                        // ASCII is byte-identical; non-ASCII requires an explicit encoder.
                        Object::String(s) if s.as_str().is_ascii() => {
                            out.extend_from_slice(s.as_str().as_bytes())
                        }
                        _ => return Err("bytes required; encode text explicitly".into()),
                    }
                }
            }
            out
        } else if p == InsertFileBytes {
            let path = String::from_utf8(self.string_bytes(args[0])?)
                .map_err(|_| "invalid filename")?;
            std::fs::read(path).map_err(|e| e.to_string())?
        } else {
            vec![]
        };
        let key = if matches!(p, PutProperty | GetProperty) {
            let v = args[if p == PutProperty { 2 } else { 1 }];
            let slot = v.index(7)?.checked_sub(1).ok_or("symbol required")?;
            Some(
                self.names
                    .iter()
                    .find(|(_, v)| **v == slot)
                    .ok_or("symbol required")?
                    .0
                    .clone(),
            )
        } else {
            None
        };
        let Object::ByteBuffer(b) = &mut self.objects[index] else {
            unreachable!()
        };
        let r = b.restriction.clone();
        let pos = |v: Value| -> Result<ByteOffset> { Ok(ByteOffset(position(v)?.0)) };
        let region = |a: Value, z: Value| -> Result<std::ops::Range<ByteOffset>> {
            let a = pos(a)?;
            let z = pos(z)?;
            let range = a.min(z)..a.max(z);
            if range.start < r.start || range.end > r.end {
                return Err("index out of range".into());
            }
            Ok(range)
        };
        match p {
            Point => Value::integer((b.point.0 + 1) as i64),
            PointMin => Value::integer((r.start.0 + 1) as i64),
            PointMax => Value::integer((r.end.0 + 1) as i64),
            BufferSize => Value::integer(b.bytes.len() as i64),
            GotoChar => {
                b.point = ByteOffset(args[0].as_integer()?.saturating_sub(1).max(0) as usize)
                    .clamp(r.start, r.end);
                Ok(self.stack[start])
            }
            CharAfter => {
                let at = if n == 0 || args[0] == Value::NIL {
                    b.point
                } else {
                    let i = args[0].as_integer()?;
                    if i < 1 {
                        return Ok(Value::NIL);
                    }
                    ByteOffset((i - 1) as usize)
                };
                if r.contains(&at) {
                    Value::integer(b.bytes[at.0] as i64)
                } else {
                    Ok(Value::NIL)
                }
            }
            Insert | InsertFileBytes => {
                let at = b.point;
                b.replace(at..at, &inserted)?;
                if p == InsertFileBytes {
                    b.point = at;
                    let count = Value::integer(inserted.len() as i64)?;
                    Ok(self.list_value(&[args[0], count]))
                } else {
                    Ok(Value::NIL)
                }
            }
            DeleteRegion => {
                b.replace(region(args[0], args[1])?, b"")?;
                Ok(Value::NIL)
            }
            Substring => {
                let r = region(args[0], args[1])?;
                let bytes = b.bytes[r.start.0..r.end.0].to_vec();
                Ok(self.allocate_object(Object::Bytes(bytes)))
            }
            Narrow => {
                let a = pos(args[0])?;
                let z = pos(args[1])?;
                let r = a.min(z)..a.max(z);
                if r.end.0 > b.bytes.len() {
                    return Err("index out of range".into());
                }
                b.point = b.point.clamp(r.start, r.end);
                b.restriction = r;
                Ok(Value::NIL)
            }
            Widen => {
                b.restriction = ByteOffset(0)..ByteOffset(b.bytes.len());
                Ok(Value::NIL)
            }
            CharToByte | ByteToChar => {
                let p = args[0].as_integer()?;
                if p < 1 || p as u64 > b.bytes.len() as u64 + 1 {
                    Ok(Value::NIL)
                } else {
                    Ok(args[0])
                }
            }
            PutProperty => {
                let mut changed = false;
                for p in region(args[0], args[1])?.start.0
                    ..region(args[0], args[1])?.end.0
                {
                    changed |= b.properties
                        .entry(ByteOffset(p))
                        .or_default()
                        .insert(key.as_ref().unwrap().clone(), args[3])
                        != Some(args[3]);
                }
                if changed { b.state.record_change(false); }
                Ok(Value::NIL)
            }
            GetProperty => {
                let p = pos(args[0])?;
                if p < r.start || p > r.end {
                    return Err("index out of range".into());
                }
                Ok(b.properties
                    .get(&p)
                    .and_then(|v| v.get(key.as_ref().unwrap()))
                    .copied()
                    .unwrap_or(Value::NIL))
            }
            _ => Err("unsupported byte buffer operation".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form(v: serde_json::Value) -> Form {
        Form::from_json(v).unwrap()
    }
    #[test]
    fn multibyte_variable_reports_real_buffer_type_and_is_read_only() {
        let mut vm = Vm::new();
        let f = vm
            .compile(&form(serde_json::json!([
                "with-temp-buffer",
                [
                    "vector",
                    "enable-multibyte-characters",
                    [
                        "progn",
                        ["set-buffer-multibyte", "nil"],
                        "enable-multibyte-characters"
                    ],
                    [
                        "progn",
                        ["set-buffer-multibyte", "t"],
                        "enable-multibyte-characters"
                    ],
                    [
                        "condition-case",
                        "err",
                        ["let", [["enable-multibyte-characters", "nil"]], 0],
                        ["setting-constant", 7]
                    ],
                    [
                        "condition-case",
                        "err",
                        ["setq", "enable-multibyte-characters", "nil"],
                        ["setting-constant", 9]
                    ]
                ]
            ])))
            .unwrap();
        let result = vm.execute_raw(f);
        assert!(result.is_ok(), "buffer representation variable: {result:?}");
        assert_eq!(vm.display(result.unwrap()).unwrap(), "[t nil t 7 9]");
    }
    #[test]
    fn casetab_four_disconnected_roots_survive_conversion_and_gc() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(set-buffer-multibyte t)").unwrap();
        let index = vm.buffer_index().unwrap();
        vm.allocate_object(Object::Bytes(vec![0; 37]));
        let mut roots = [Value::NIL; 4];
        for (i, root) in roots.iter_mut().enumerate() {
            let mut table = crate::char_table::CharTable::new(Value::NIL);
            table.set(65, Value::integer((i + 100) as i64).unwrap()).unwrap();
            *root = vm.allocate_object(Object::CharTable(Box::new(table)));
        }
        let Object::Buffer(buffer) = &mut vm.objects[index] else { panic!() };
        buffer.case_tables = roots;
        vm.collect(&mut []);
        let index = vm.buffer_index().unwrap();
        let Object::Buffer(buffer) = &vm.objects[index] else { panic!() };
        for i in 0..4 {
            assert_ne!(roots[i], buffer.case_tables[i]);
            assert_eq!(vm.char_table_get(buffer.case_tables[i], 65).unwrap().integer_value(), Some((i + 100) as i64));
        }
        for multibyte in [false, true, false] {
            vm.eval_source_raw(if multibyte { "(set-buffer-multibyte t)" } else { "(set-buffer-multibyte nil)" }).unwrap();
            vm.collect(&mut []);
            let index = vm.buffer_index().unwrap();
            let roots = match &vm.objects[index] {
                Object::Buffer(b) => b.case_tables,
                Object::ByteBuffer(b) => b.case_tables,
                _ => panic!(),
            };
            for i in 0..4 { assert_eq!(vm.char_table_get(roots[i], 65).unwrap().integer_value(), Some((i + 100) as i64)); }
        }
    }
    #[test]
    fn failed_byte_decoding_is_atomic_and_byte_properties_are_gc_edges() {
        let mut vm = Vm::new();
        vm.allocate_object(Object::Bytes(vec![42]));
        let leaf = vm.allocate_object(Object::String(Box::new(data::LispString::new(
            "retained",
        ))));
        let mut b = bytes::ByteBuffer {
            state: Default::default(),
            syntax_table: Value::NIL,
            category_table: Value::NIL,
            case_tables: [Value::NIL; 4],
            bytes: vec![0xce, 0xbb],
            point: ByteOffset(1),
            restriction: ByteOffset(0)..ByteOffset(2),
            properties: Default::default(),
            per_buffer: Default::default(),
        };
        b.properties
            .entry(ByteOffset(1))
            .or_default()
            .insert("x".into(), leaf);
        let buffer = vm.allocate_object(Object::ByteBuffer(Box::new(b)));
        vm.slots[vm.buffer_slot].value = buffer;
        let f = vm
            .compile(&form(serde_json::json!(["set-buffer-multibyte", "t"])))
            .unwrap();
        assert!(
            vm.execute_raw(f).is_err(),
            "an interior byte boundary cannot become text"
        );
        let mut roots = [buffer];
        vm.collect(&mut roots);
        let Object::ByteBuffer(b) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        assert_eq!(b.bytes, [0xce, 0xbb]);
        assert_eq!(b.point, ByteOffset(1));
        let leaf = b.properties[&ByteOffset(1)]["x"];
        assert_eq!(vm.display(leaf).unwrap(), "\"retained\"");
        let Object::ByteBuffer(b) = &mut vm.objects[roots[0].index(0).unwrap() - 1] else {
            panic!()
        };
        b.point = ByteOffset(0);
        b.properties.clear();
        assert!(vm.execute_raw(f).is_ok());
        assert!(vm.buffer_text_p());
        let Object::Buffer(t) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        assert_eq!(t.as_str(), "λ");
    }
    #[test]
    fn byte_string_shims_keep_encoding_distinct_from_checked_octets() {
        // GNU: string-to-unibyte of "é" signals
        // (error "Cannot convert character at index 0 to unibyte").
        // The old successful-octet expectation was Unicode, not GNU semantics.
        let mut vm = Vm::new();
        let f = vm
            .compile(&form(serde_json::json!(["vector",
                ["string-as-unibyte", {"string":"é"}],
                ["condition-case", "e", ["string-to-unibyte", {"string":"é"}], ["error", "e"]],
                ["string-as-multibyte", ["unibyte-string", 0xce, 0xbb]],
                ["multibyte-string-p", ["unibyte-string", 255]],
                ["multibyte-string-p", {"string":"λ"}]
            ])))
            .unwrap();
        let result = vm.execute_raw(f).unwrap();
        assert_eq!(
            vm.display(result).unwrap(),
            "[#u8(195 169) (error \"Cannot convert character at index 0 to unibyte\") \"λ\" nil t]"
        );
        let bad = vm
            .compile(&form(serde_json::json!([
                "string-as-multibyte",
                ["unibyte-string", 255]
            ])))
            .unwrap();
        // GNU now: (t 1 2 4194303), not the old UTF-8 rejection.
        let raw = vm.execute_raw(bad).unwrap();
        let view = vm.str_view(raw).unwrap();
        assert!(view.is_multibyte());
        assert_eq!(view.len(), 1);
        assert_eq!(view.bytes().len(), 2);
        assert_eq!(view.chars().next(), Some(4194303));
    }
    #[test]
    fn explicit_buffer_boundaries_preserve_utf8_and_reject_invalid_octets() {
        let mut vm = Vm::new();
        let f = vm
            .compile(&form(serde_json::json!(["with-temp-buffer",
                ["insert", {"string":"λ🦀"}],
                ["vector", ["buffer-encode-utf8"], ["buffer-string"], "enable-multibyte-characters",
                    ["buffer-decode-utf8"], ["buffer-string"], "enable-multibyte-characters"]
            ])))
            .unwrap();
        let v = vm.execute_raw(f).unwrap();
        assert_eq!(
            vm.display(v).unwrap(),
            "[nil #u8(206 187 240 159 166 128) nil t \"λ🦀\" t]"
        );
        let invalid = vm
            .compile(&form(serde_json::json!([
                "with-temp-buffer",
                ["buffer-encode-utf8"],
                ["insert", 255],
                ["buffer-decode-utf8"]
            ])))
            .unwrap();
        assert!(vm.execute_raw(invalid).is_err());
    }
    #[test]
    fn byte_boundary_shim_preserves_non_utf8_through_gc() {
        let mut vm = Vm::new();
        let compiled = vm.compile(&form(serde_json::json!([
            "with-temp-buffer",
            ["set-buffer-multibyte", "nil"],
            ["insert", 255, 128, 0],
            [
                "vector",
                ["buffer-string"],
                ["char-after", 1],
                ["buffer-size"]
            ]
        ])));
        assert!(
            compiled.is_ok(),
            "byte-boundary compatibility shim: {compiled:?}"
        );
        let executed = vm.execute_raw(compiled.unwrap());
        assert!(executed.is_ok(), "byte-boundary execution: {executed:?}");
        let mut roots = [executed.unwrap()];
        vm.collect(&mut roots);
        assert_eq!(vm.display(roots[0]).unwrap(), "[#u8(255 128 0) 255 3]");
    }
    #[test]
    fn unicode_string_replacement_is_not_a_gnu_mode_operation() {
        let mut vm = Vm::new();
        let f = vm.compile(&form(serde_json::json!(["let", [["s", {"string":"aλz"}]], ["aset", "s", 0, 128578], ["aset", "s", 1, 98], ["vector", ["length", "s"], ["aref", "s", 0], ["aref", "s", 1]]]))).unwrap();
        let result = vm.execute_raw(f);
        assert!(result.is_ok(), "Unicode scalar replacement: {result:?}");
        assert_eq!(vm.display(result.unwrap()).unwrap(), "[3 128578 98]");
    }
    #[test]
    fn annotation_default_resolution_is_not_raw_storage() {
        let mut vm = Vm::new();
        vm.declare_special_raw("default-text-properties", Value::NIL);
        let f = vm.compile(&form(serde_json::json!(["let", [["default-text-properties", ["quote", ["p", 99]]]], ["with-temp-buffer", ["insert", {"string":"x"}], ["get-text-property", 1, ["quote", "p"]]]]))).unwrap();
        let value = vm.execute_raw(f).unwrap();
        assert_eq!(
            value,
            Value::integer(99).unwrap(),
            "absent key must resolve default, not raw nil"
        );
    }
    #[test]
    fn dense_property_write_uses_runs_not_character_maps() {
        let mut vm = Vm::new();
        let f = vm.compile(&form(serde_json::json!(["with-temp-buffer", ["insert", {"string": "a".repeat(4096)}], ["put-text-property", 1, 4097, ["quote", "face"], "nil"], ["current-buffer"]]))).unwrap();
        let buffer = vm.execute_raw(f).unwrap();
        let Object::Buffer(text) = vm.object(buffer).unwrap() else {
            panic!()
        };
        assert!(
            text.properties.storage_bytes() < 32768,
            "dense equal properties still allocate per character: {}",
            text.properties.storage_bytes()
        );
    }
    #[test]
    fn string_property_identity_edges_move_with_compaction_and_width_changes() {
        let mut vm = Vm::new();
        vm.allocate_object(Object::Bytes(vec![0]));
        let f = vm.compile(&form(serde_json::json!(["let", [["s", {"string":"aλz"}]], ["put-text-property",1,2,["quote","x"],["vector",7],"s"], ["aset","s",1,128578], "s"]))).unwrap();
        let string = vm.execute_raw(f).unwrap();
        let Object::String(s) = vm.object(string).unwrap() else {
            panic!()
        };
        assert_eq!(s.as_str(), "a🙂z");
        let before = s.properties.get_property(ByteOffset(1), "x").unwrap();
        assert_eq!(s.properties.get_property(ByteOffset(4), "x"), Some(before));
        assert_eq!(s.properties.get_property(ByteOffset(5), "x"), None);
        let mut roots = [string];
        vm.collect(&mut roots);
        let Object::String(s) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        let after = s.properties.get_property(ByteOffset(1), "x").unwrap();
        assert_ne!(before, after, "test must exercise moving Value indices");
        assert!(
            matches!(vm.object(after), Ok(Object::Vector(v)) if v == &vec![Value::integer(7).unwrap()])
        );
        // This form's literal string is also rooted by compiled code. Retire
        // those legitimate roots before asserting unreachable graph reclamation.
        vm.functions.clear();
        vm.execution = Default::default();
        vm.collect(&mut []);
        assert_eq!(vm.heap_objects(), 0);
    }
    #[test]
    fn equal_but_not_eq_values_are_not_interned_together() {
        let mut vm = Vm::new();
        let a = vm.allocate_object(Object::String(Box::new(data::LispString::new("same"))));
        let b = vm.allocate_object(Object::String(Box::new(data::LispString::new("same"))));
        assert_ne!(a, b);
        let mut text = Text::new(b"ab").unwrap();
        text.put_property(ByteOffset(0)..ByteOffset(1), "face", a)
            .unwrap();
        text.put_property(ByteOffset(1)..ByteOffset(2), "face", b)
            .unwrap();
        assert_eq!(text.property(ByteOffset(0), "face"), Ok(Some(a)));
        assert_eq!(text.property(ByteOffset(1), "face"), Ok(Some(b)));
        let copy = text.clone();
        assert_eq!(copy.property(ByteOffset(1), "face"), Ok(Some(b)));
    }
    #[test]
    fn buffer_properties_are_strong_rewritten_edges_and_cycles_reclaim() {
        let mut vm = Vm::new();
        vm.allocate_object(Object::String(Box::new(data::LispString::new("d"))));
        let leaf = vm.allocate_object(Object::String(Box::new(data::LispString::new("λ"))));
        let vector = vm.allocate_object(Object::Vector(vec![leaf]));
        let mut text = Text::new("aλЖ🙂z".as_bytes()).unwrap();
        text.properties
            .put(ByteOffset(1)..ByteOffset(3), "probe", vector);
        let buffer = vm.allocate_object(Object::Buffer(Box::new(text.into())));
        if let Object::Vector(v) = &mut vm.objects[2] {
            v.push(buffer);
        }
        let mut roots = [buffer];
        vm.collect(&mut roots);
        assert_eq!(vm.heap_objects(), 3);
        let Object::Buffer(t) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        assert_eq!(t.byte_of_char(CharOffset(4)), Ok(ByteOffset(9)));
        let property = t.properties.get_property(ByteOffset(1), "probe").unwrap();
        let Object::Vector(v) = vm.object(property).unwrap() else {
            panic!()
        };
        assert_eq!(v[1], roots[0]);
        assert!(matches!(vm.object(v[0]).unwrap(),Object::String(s) if s.as_str()=="λ"));
        vm.collect(&mut []);
        assert_eq!(vm.heap_objects(), 0);
        assert_eq!(vm.heap_stats().reclaimed, 4);
    }
    #[test]
    fn buffers_and_property_graphs_survive_execute_rooted_and_nested_unwind() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let f = vm.compile(&form(serde_json::json!(["with-current-buffer",["get-buffer-create",{"string":"rooted-live"},"t"],["insert",{"string":"aλЖ🙂z"}], ["put-text-property",2,4,["quote","x"],["vector",{"string":"λ"},7]], ["current-buffer"]]))).unwrap();
        let mut roots = [vm.execute_raw(f).unwrap()];
        // Keep the GC assertion about external roots, not the name registry.
        let registry = vm.slot("#:vm2-buffer:rooted-live");
        vm.slots[registry].value = Value::UNBOUND;
        let churn = vm.compile(&form(serde_json::json!(["let",[["i",0]],["while",["<","i",100000],["with-temp-buffer",["insert",{"string":"🙂"}]],["setq","i",["1+","i"]]],7]))).unwrap();
        assert_eq!(
            vm.execute_rooted(churn, &mut roots)
                .unwrap()
                .as_integer()
                .unwrap(),
            7
        );
        assert!(vm.heap_stats().collections > 0);
        vm.collect(&mut roots);
        let Object::Buffer(t) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        assert_eq!(t.slice(ByteOffset(1)..ByteOffset(9)).unwrap(), "λЖ🙂");
        let Object::Vector(v) = vm
            .object(t.properties.get_property(ByteOffset(1), "x").unwrap())
            .unwrap()
        else {
            panic!()
        };
        assert!(matches!(vm.object(v[0]).unwrap(),Object::String(s) if s.as_str()=="λ"));
    }
    #[test]
    fn lazy_suffix_property_edges_are_rewritten_and_cycles_reclaimed() {
        use crate::{
            Vm,
            data::{Object, LispString},
            text::Text,
        };
        let mut vm = Vm::new();
        vm.allocate_object(Object::Bytes(vec![0]));
        let leaf = vm.allocate_object(Object::String(Box::new(LispString::new("λ"))));
        let vector = vm.allocate_object(Object::Vector(vec![leaf]));
        let mut text = Text::new(&vec![b'a'; 8192]).unwrap();
        for at in (0..8192).step_by(8) {
            text.properties
                .put(ByteOffset(at)..ByteOffset(at + 1), "x", vector);
        }
        text.replace(ByteOffset(0)..ByteOffset(0), "λ🙂".as_bytes())
            .unwrap();
        let buffer = vm.allocate_object(Object::Buffer(Box::new(text.into())));
        if let Object::Vector(v) = &mut vm.objects[2] {
            v.push(buffer);
        }
        let mut roots = [buffer];
        vm.collect(&mut roots);
        let Object::Buffer(t) = vm.object(roots[0]).unwrap() else {
            panic!()
        };
        for at in (0..8192).step_by(8) {
            let property = t.properties.get_property(ByteOffset(at + 6), "x").unwrap();
            let Object::Vector(v) = vm.object(property).unwrap() else {
                panic!()
            };
            assert_eq!(v[1], roots[0]);
            assert!(matches!(vm.object(v[0]).unwrap(), Object::String(s) if s.as_str()=="λ"));
        }
        vm.collect(&mut []);
        assert_eq!(vm.heap_objects(), 0);
        assert_eq!(vm.heap_stats().reclaimed, 4);
    }
}

impl Vm {
    pub(crate) fn add_text_properties_native(
        &mut self,
        start: Value,
        end: Value,
        plist: Value,
        object: Value,
    ) -> crate::prim::Out {
        use crate::prim::Fault;
        let pairs = self.text_property_pairs(plist)?;
        if pairs.is_empty() {
            return Ok(Value::NIL);
        }
        let object = if object == Value::NIL {
            self.buffer_index().map_err(Fault::Error)?;
            self.slots[self.buffer_slot].value
        } else {
            object
        };
        let Some((range, mut properties)) = self.text_property_region(start, end, object)? else {
            return Ok(Value::NIL);
        };
        let pairs = pairs
            .into_iter()
            .map(|(key, value)| self.text_property_key(key).map(|key| (key, value)))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut changed = false;
        for (key, value) in pairs {
            let mut at = range.start;
            while at < range.end {
                changed |= properties.get_property(at, &key) != Some(value);
                at = properties.next_change(at, range.end);
            }
            properties.put(range.clone(), &key, value);
        }
        if !changed {
            return Ok(Value::NIL);
        }
        self.text_property_commit(object, range, properties)?;
        Ok(Value::TRUE)
    }

    /// Install PROPERTIES, already edited over RANGE, as OBJECT's: the
    /// read-only checks and modification count of modify_text_properties
    /// for a buffer.
    fn text_property_commit(
        &mut self,
        object: Value,
        range: std::ops::Range<ByteOffset>,
        properties: properties::Properties,
    ) -> std::result::Result<(), crate::prim::Fault> {
        use crate::prim::Fault;
        if Self::image_value(object) {
            return Err(Fault::Error("immutable image string".into()));
        }
        if matches!(self.object_if(object), Some(Object::Buffer(_))) {
            let current = self.slots[self.buffer_slot].value;
            self.slots[self.buffer_slot].value = object;
            let result = self.check_text_property_edit(object, range.clone());
            self.slots[self.buffer_slot].value = current;
            result?;
        }
        let index = object.index(0).map_err(Fault::Error)? - 1;
        match &mut self.objects[index] {
            Object::Buffer(buffer) => {
                buffer.properties = properties;
                buffer.state.record_change(false);
                self.display_state.version += 1;
            }
            Object::String(string) | Object::RawString(string) => string.properties = properties,
            Object::Bytes(bytes) => {
                let mut string = Box::new(data::LispString::from_bytes(bytes.clone(), false));
                string.properties = properties;
                self.objects[index] = if string.is_utf8() {
                    Object::String(string)
                } else {
                    Object::RawString(string)
                };
            }
            Object::ByteBuffer(_) => {
                return Err(Fault::Error(
                    "bulk text properties in unibyte buffers are not implemented".into(),
                ));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// textprop.c Fremove_text_properties / Fremove_list_of_text_properties:
    /// t when some character in START..END carried one of NAMES.
    pub(crate) fn remove_text_properties_native(
        &mut self,
        start: Value,
        end: Value,
        names: &[Value],
        object: Value,
    ) -> crate::prim::Out {
        let object = if object == Value::NIL {
            self.buffer_index().map_err(crate::prim::Fault::Error)?;
            self.slots[self.buffer_slot].value
        } else {
            object
        };
        let Some((range, mut properties)) = self.text_property_region(start, end, object)? else {
            return Ok(Value::NIL);
        };
        let keys = names.iter().map(|&name| self.text_property_key(name)).collect::<std::result::Result<Vec<_>, _>>()?;
        let mut changed = false;
        for key in &keys {
            let mut at = range.start;
            while at < range.end {
                changed |= properties.get_property(at, key).is_some();
                at = properties.next_change(at, range.end);
            }
            properties.remove(range.clone(), key);
        }
        if !changed {
            return Ok(Value::NIL);
        }
        self.text_property_commit(object, range, properties)?;
        Ok(Value::TRUE)
    }

    /// textprop.c set_text_properties: replace every property in START..END.
    /// Text with no properties and a nil PROPERTIES is left alone and nil
    /// returned; otherwise t (VM2-TEXTPROP-INTERVALS1).
    pub(crate) fn set_text_properties_native(
        &mut self,
        start: Value,
        end: Value,
        plist: Value,
        object: Value,
    ) -> crate::prim::Out {
        let pairs = self.text_property_pairs(plist)?;
        let object = if object == Value::NIL {
            self.buffer_index().map_err(crate::prim::Fault::Error)?;
            self.slots[self.buffer_slot].value
        } else {
            object
        };
        let Some((range, mut properties)) = self.text_property_region(start, end, object)? else {
            return Ok(Value::NIL);
        };
        if pairs.is_empty() && properties.is_empty() {
            return Ok(Value::NIL);
        }
        for key in properties.properties_keys() {
            properties.remove(range.clone(), &key);
        }
        // The newest key reads first; GNU keeps PROPERTIES' own order.
        for (key, value) in pairs.into_iter().rev() {
            let key = self.text_property_key(key)?;
            properties.put(range.clone(), &key, value);
        }
        self.text_property_commit(object, range, properties)?;
        Ok(Value::TRUE)
    }

    fn text_property_pairs(
        &self,
        plist: Value,
    ) -> std::result::Result<Vec<(Value, Value)>, crate::prim::Fault> {
        use crate::prim::Fault;
        if plist == Value::NIL {
            return Ok(vec![]);
        }
        if !matches!(self.cons_pair(plist), Ok(_)) {
            return Ok(vec![(plist, Value::NIL)]);
        }
        let mut pairs = Vec::new();
        let mut tail = plist;
        let mut seen = HashSet::new();
        while let Ok((key, rest)) = self.cons_pair(tail) {
            if !seen.insert(tail.0) {
                return Err(Fault::Circular(plist));
            }
            let (value, next) = self
                .cons_pair(rest)
                .map_err(|_| Fault::Error("Odd length text property list".into()))?;
            pairs.push((key, value));
            tail = next;
        }
        Ok(pairs)
    }

    fn text_property_name(&self, key: Value) -> Option<&str> {
        if key == Value::NIL {
            return Some("nil");
        }
        if key == Value::TRUE {
            return Some("t");
        }
        let slot = key.index_if(7)?.checked_sub(1)?;
        let name = self.symbol_names.get(slot)?;
        (self.names.get(name) == Some(&slot)).then_some(name.as_str())
    }

    fn text_property_key(&self, key: Value) -> std::result::Result<String, crate::prim::Fault> {
        self.text_property_name(key)
            .map(str::to_owned)
            .ok_or_else(|| {
                crate::prim::Fault::Error(
                    "text property keys other than interned symbols are not implemented".into(),
                )
            })
    }

    pub(super) fn text_property_position(
        &self,
        value: Value,
    ) -> std::result::Result<i64, crate::prim::Fault> {
        if let Some(number) = value.integer_value() {
            return Ok(number);
        }
        if let Some(Object::Bignum(number)) = self.object_if(value) {
            return Ok(if number.sign() == num_bigint::Sign::Minus {
                Value::MIN_FIXNUM
            } else {
                Value::MAX_FIXNUM
            });
        }
        if matches!(self.object_if(value), Some(Object::Marker(_))) {
            return self
                .marker_position_value(value)
                .map_err(crate::prim::Fault::Error)?
                .integer_value()
                .ok_or_else(|| crate::prim::Fault::Error("Marker does not point anywhere".into()));
        }
        Err(crate::prim::Fault::Wrong("integer-or-marker-p", value))
    }

    fn text_property_region(
        &self,
        start: Value,
        end: Value,
        object: Value,
    ) -> std::result::Result<
        Option<(std::ops::Range<ByteOffset>, properties::Properties)>,
        crate::prim::Fault,
    > {
        use crate::prim::Fault;
        let string = self.str_view(object);
        if string.is_none()
            && !matches!(
                self.object_if(object),
                Some(Object::Buffer(_) | Object::ByteBuffer(_))
            )
        {
            return Err(Fault::Wrong("buffer-or-string-p", object));
        }
        let (a, z) = (
            self.text_property_position(start)?,
            self.text_property_position(end)?,
        );
        if a == z {
            return Ok(None);
        }
        let (a, z) = (a.min(z), a.max(z));
        let range_error = || Fault::Range(start, end);
        let (range, properties) = if let Some(string) = string {
            if a < 0 || z as u64 > string.len() as u64 {
                return Err(range_error());
            }
            let offset = |index| {
                ByteOffset(if string.is_multibyte() {
                    string
                        .chars()
                        .take(index)
                        .map(|code| crate::lstring::char_string(code, &mut [0; 5]))
                        .sum()
                } else {
                    index
                })
            };
            let properties = match self.object_if(object) {
                Some(Object::String(s) | Object::RawString(s)) => s.properties.clone(),
                _ => properties::Properties::new(),
            };
            (offset(a as usize)..offset(z as usize), properties)
        } else {
            if a < 1 {
                return Err(range_error());
            }
            match self.object_if(object).unwrap() {
                Object::Buffer(buffer) => {
                    let a = buffer
                        .byte_of_char(CharOffset(a as usize - 1))
                        .map_err(|_| range_error())?;
                    let z = buffer
                        .byte_of_char(CharOffset(z as usize - 1))
                        .map_err(|_| range_error())?;
                    if a < buffer.accessible().start || z > buffer.accessible().end {
                        return Err(range_error());
                    }
                    (a..z, buffer.properties.clone())
                }
                Object::ByteBuffer(buffer) => {
                    let range = ByteOffset(a as usize - 1)..ByteOffset(z as usize - 1);
                    if range.start < buffer.restriction.start || range.end > buffer.restriction.end
                    {
                        return Err(range_error());
                    }
                    (range, properties::Properties::from_map(&buffer.properties))
                }
                _ => unreachable!(),
            }
        };
        Ok(Some((range, properties)))
    }

    fn text_property_variable(
        &mut self,
        name: &str,
    ) -> std::result::Result<Value, crate::prim::Fault> {
        let slot = self.slot(name);
        let value = self
            .variable_value(slot)
            .map_err(crate::prim::Fault::Error)?;
        Ok(if value == Value::UNBOUND {
            Value::NIL
        } else {
            value
        })
    }

    fn text_property_find(
        &self,
        list: Value,
        matches: impl Fn(Value) -> bool,
    ) -> std::result::Result<Option<Value>, crate::prim::Fault> {
        let mut tail = list;
        let mut walk = annotations::TailWalk::new(list);
        while let Ok((value, next)) = self.cons_pair(tail) {
            if matches(value) {
                return Ok(Some(value));
            }
            tail = next;
            if walk.cycle(tail) {
                return Err(crate::prim::Fault::Circular(list));
            }
        }
        if tail != Value::NIL {
            return Err(crate::prim::Fault::Wrong("listp", list));
        }
        Ok(None)
    }

    fn text_property_category(
        &mut self,
        category: Value,
        property: Value,
    ) -> std::result::Result<Value, crate::prim::Fault> {
        use crate::prim::Fault;
        let environment = self.text_property_variable("overriding-plist-environment")?;
        if let Some(entry) = self.text_property_find(environment, |entry| {
            self.cons_pair(entry).is_ok_and(|(key, _)| key == category)
        })? {
            let (_, plist) = self.cons_pair(entry).map_err(Fault::Error)?;
            let value = self.plist_lookup(plist, property);
            if value != Value::NIL {
                return Ok(value);
            }
        }
        self.symbol_property(category, property)
            .map_err(Fault::Error)
    }

    fn text_property_at(
        &mut self,
        properties: &properties::Properties,
        at: ByteOffset,
        key: &str,
    ) -> std::result::Result<Value, crate::prim::Fault> {
        use crate::prim::Fault;
        if let Some(value) = properties.get_property(at, key) {
            return Ok(value);
        }
        let symbol = Value::reference(self.slot(key) + 1, 7);
        if let Some(category) = properties.get_property(at, "category")
            && (category == Value::NIL
                || category == Value::TRUE
                || (category.tag() == 7 && category != Value::UNBOUND))
        {
            let value = self.text_property_category(category, symbol)?;
            if value != Value::NIL {
                return Ok(value);
            }
        }
        let aliases = self.text_property_variable("char-property-alias-alist")?;
        if let Some(entry) = self.text_property_find(aliases, |entry| {
            self.cons_pair(entry).is_ok_and(|(key, _)| key == symbol)
        })? {
            let (_, mut tail) = self.cons_pair(entry).map_err(Fault::Error)?;
            let mut walk = annotations::TailWalk::new(tail);
            while let Ok((alias, next)) = self.cons_pair(tail) {
                // GNU aliases consult the direct plist, never the category.
                if let Some(name) = self.text_property_name(alias)
                    && let Some(value) = properties.get_property(at, name)
                    && value != Value::NIL
                {
                    return Ok(value);
                }
                tail = next;
                if walk.cycle(tail) {
                    return Err(Fault::Circular(tail));
                }
            }
        }
        let defaults = self.text_property_variable("default-text-properties")?;
        Ok(self.plist_lookup(defaults, symbol))
    }

    fn text_property_signal(&mut self, name: &str, data: &[Value]) -> crate::prim::Fault {
        let symbol = Value::reference(self.slot(name) + 1, 7);
        crate::prim::Fault::Signal(symbol, self.list_value(data))
    }

    fn check_text_property_edit(
        &mut self,
        object: Value,
        range: std::ops::Range<ByteOffset>,
    ) -> std::result::Result<(), crate::prim::Fault> {
        use crate::prim::Fault;
        let Object::Buffer(buffer) = self.object(object).map_err(Fault::Error)? else {
            unreachable!()
        };
        let properties = buffer.properties.clone();
        let modified = buffer.state.modified();
        let inhibit = self.text_property_variable("inhibit-read-only")?;
        let read_only =
            self.text_property_variable("buffer-read-only")? != Value::NIL && inhibit == Value::NIL;
        if read_only
            && self.text_property_at(&properties, range.start, "inhibit-read-only")? == Value::NIL
        {
            return Err(self.text_property_signal("buffer-read-only", &[object]));
        }
        let selective = self.cons_pair(inhibit).is_ok();
        let ignore = inhibit != Value::NIL && !selective;
        let hooks = self.text_property_variable("inhibit-modification-hooks")? == Value::NIL;
        let mut property_hooks = false;
        let mut at = range.start;
        while at < range.end {
            let value = self.text_property_at(&properties, at, "read-only")?;
            let express = self.text_property_at(&properties, at, "inhibit-read-only")?
                != Value::NIL
                || (value != Value::NIL
                    && selective
                    && self
                        .text_property_find(inhibit, |item| item == value)?
                        .is_some());
            if value != Value::NIL && !ignore && !express {
                let data = if self.str_view(value).is_some() {
                    vec![value]
                } else {
                    vec![]
                };
                return Err(self.text_property_signal("text-read-only", &data));
            }
            if read_only && !express {
                return Err(self.text_property_signal("buffer-read-only", &[object]));
            }
            property_hooks |= hooks
                && self.text_property_at(&properties, at, "modification-hooks")? != Value::NIL;
            at = properties.next_change(at, range.end);
        }
        if hooks {
            for name in [
                "before-change-functions",
                "after-change-functions",
                "first-change-hook",
            ] {
                if (name != "first-change-hook" || !modified)
                    && self.text_property_variable(name)? != Value::NIL
                {
                    property_hooks = true;
                }
            }
        }
        if property_hooks {
            return Err(Fault::Error(
                "text property changes with modification hooks are not implemented".into(),
            ));
        }
        Ok(())
    }
}
