use super::*;
impl Leaf<'_> {
    pub(crate) fn format_message(&mut self, template: Value, arguments: &[Value]) -> std::result::Result<Value, Fault> {
        self.string_view(template)?;
        let style = self.text_quoting_style()?;
        let mut values = Vec::with_capacity(arguments.len() + 1);
        values.push(template);
        values.extend_from_slice(arguments);
        self.vm.format_values(&values, Some(style)).map_err(|error| {
            if error == "floating-point overflow" {
                Fault::Signal(self.intern("overflow-error"), Value::NIL)
            } else { Fault::Error(error) }
        })
    }

    pub(crate) fn text_quoting_style(&mut self) -> std::result::Result<&'static str, Fault> {
        let variable = self.intern("text-quoting-style");
        let value = self.variable_value(variable)?.unwrap_or(Value::NIL);
        // GNU doc.c default_to_grave_quoting_style and Ftext_quoting_style.
        // Non-nil explicit styles bypass both the capability flag and table.
        let style = if value == Value::NIL {
            let flag = self.intern("internal--text-quoting-flag");
            if self.variable_value(flag)?.unwrap_or(Value::NIL) == Value::NIL {
                "grave"
            } else {
                let variable = self.intern("standard-display-table");
                let table = self.variable_value(variable)?.unwrap_or(Value::NIL);
                // disptab.h: DISP_TABLE_P requires purpose AND all 18 slots
                // (six ordinary slots plus twelve box-drawing slots).
                let purpose = self.intern("display-table");
                let display_table = match self.vm.object_if(table) {
                    Some(data::Object::CharTable(t)) => t.extras.len() == 18
                        && t.purpose == purpose,
                    _ => false,
                };
                let grave = if display_table {
                    // U+2018 uses char_table_ref, including defaults/parents.
                    let vector = self.vm.char_table_get(table, 0x2018).map_err(Fault::Error)?;
                    matches!(self.vm.object_if(vector), Some(data::Object::Vector(v))
                        if v.len() == 1 && v[0].integer_value() == Some(96))
                } else {
                    false
                };
                if grave { "grave" } else { "curve" }
            }
        } else {
            if value == self.intern("grave") {
                "grave"
            } else if value == self.intern("straight") {
                "straight"
            } else {
                "curve"
            }
        };
        Ok(style)
    }

    /// OBJECT printed to STREAM's text, each unreadable object it meets
    /// taking the next of ANSWERS (see `Vm::print_render_answered').
    pub(crate) fn print_text(&mut self, object: Value, quoted: bool, overrides: Value, stream: Value, answers: Value) -> std::result::Result<Value, Fault> {
        let answers = self.vm.list_values(answers).map_err(Fault::Error)?;
        let text = self.printing(overrides, stream, |vm, overrides, buffer| vm.print_render_answered(object, quoted, overrides, buffer, answers))?;
        Ok(self.vm.printed_value(&text))
    }

    /// The unreadable objects OBJECT's print to STREAM meets, in order.
    pub(crate) fn print_unreadables(&mut self, object: Value, quoted: bool, overrides: Value, stream: Value) -> std::result::Result<Vec<Value>, Fault> {
        self.printing(overrides, stream, |vm, overrides, buffer| vm.print_unreadables(object, quoted, overrides, buffer))
    }

    /// `prin1-to-string''s text: printed into a multibyte buffer, as GNU's
    /// ` prin1' buffer is, each unreadable object taking the next of ANSWERS.
    pub(crate) fn print_string(&mut self, object: Value, quoted: bool, overrides: Value, answers: Value) -> std::result::Result<Value, Fault> {
        let answers = self.vm.list_values(answers).map_err(Fault::Error)?;
        let text = self.vm.print_render_answered(object, quoted, overrides, true, answers).map_err(Fault::Error)?;
        Ok(self.vm.printed_value(&text))
    }

    /// `print-unreadable-function', nil when unbound.
    pub(crate) fn print_unreadable_function(&mut self) -> std::result::Result<Value, Fault> {
        self.vm.print_unreadable_function().map_err(Fault::Error)
    }

    /// PRINT run with the print's settings for STREAM: its overrides (a
    /// unibyte buffer escapes multibyte text) and whether it is a multibyte
    /// buffer, the buffer current meanwhile.
    fn printing<T>(&mut self, mut overrides: Value, stream: Value,
                   print: impl FnOnce(&mut Vm, Value, bool) -> crate::Result<T>) -> std::result::Result<T, Fault> {
        let buffer = self.print_buffer(stream)?;
        if let Some(buffer) = buffer {
            if self.kind(buffer) == Kind::ByteBuffer {
                let mut settings = if overrides == Value::TRUE { vec![overrides] }
                    else { self.vm.list_values(overrides).map_err(Fault::Error)? };
                let key = self.intern("escape-multibyte");
                settings.push(self.cons(key, Value::TRUE));
                overrides = self.vm.list_value(&settings);
            }
        }
        let current = if buffer.is_some() { Some(self.current_buffer()?) } else { None };
        if let Some(buffer) = buffer { self.vm.slots[self.vm.buffer_slot].value = buffer; }
        let multibyte = buffer.is_some_and(|buffer| self.kind(buffer) == Kind::Buffer);
        let result = print(self.vm, overrides, multibyte);
        if let Some(current) = current { self.vm.slots[self.vm.buffer_slot].value = current; }
        result.map_err(Fault::Error)
    }

    fn print_buffer(&self, stream: Value) -> std::result::Result<Option<Value>, Fault> {
        let buffer = match self.vm.object_if(stream) {
            Some(data::Object::Marker(marker)) => {
                if marker.buffer == Value::NIL {
                    return Err(Fault::Error("Marker does not point anywhere".into()));
                }
                marker.buffer
            }
            Some(data::Object::Buffer(_) | data::Object::ByteBuffer(_)) => stream,
            _ => return Ok(None),
        };
        self.vm.require_live_buffer(buffer).map_err(Fault::Error)?;
        Ok(Some(buffer))
    }

    /// A string prints as itself, unless an earlier print numbered it or
    /// its raw bytes take octal escapes in a multibyte buffer.
    pub(crate) fn princ_text(&mut self, object: Value, stream: Value, answers: Value) -> std::result::Result<Value, Fault> {
        let continuous = self.intern("print-continuous-numbering");
        let continuous = self.variable_value(continuous)?.is_some_and(|v| v != Value::NIL);
        if continuous || !matches!(self.kind(object), Kind::String | Kind::Bytes) {
            return self.print_text(object, false, Value::NIL, stream, answers);
        }
        if !self.string_view(object)?.is_multibyte() && self.print_buffer(stream)?.is_some_and(|buffer| self.kind(buffer) == Kind::Buffer) {
            let text = self.vm.print_plain_string(object).map_err(Fault::Error)?;
            return Ok(self.vm.printed_value(&text));
        }
        Ok(object)
    }

    /// Whether printing TEXT to the buffer or marker STREAM runs Lisp.
    /// print_finish's insert_1_both prepares a nonempty insertion as
    /// `insert' does; its signal_after_change runs even for empty TEXT.
    pub(crate) fn print_runs_lisp(&mut self, text: Value, stream: Value) -> std::result::Result<bool, Fault> {
        let buffer = self.print_buffer(stream)?.unwrap();
        let current = self.current_buffer()?;
        self.vm.slots[self.vm.buffer_slot].value = buffer;
        let runs = self.insertion_runs_lisp(text, stream);
        self.vm.slots[self.vm.buffer_slot].value = current;
        runs
    }

    fn insertion_runs_lisp(&mut self, text: Value, stream: Value) -> std::result::Result<bool, Fault> {
        let (point, begv, zv) = self.vm.point_extent()?;
        let marker = self.kind(stream) == Kind::Marker;
        let at = if marker {
            let at = self.vm.marker_position_value(stream).map_err(Fault::Error)?.as_integer().map_err(Fault::Error)?;
            if at < begv || at > zv {
                return Err(Fault::Error("Marker is outside the accessible part of the buffer".into()));
            }
            at
        } else { point };
        if self.string_view(text)?.bytes().is_empty() { return self.vm.after_change_runs(false, false); }
        // print_prepare moved point to a marker stream before any refusal.
        let prepared = self.vm.prepare_change(at, at).inspect_err(|_| if marker { let _ = self.goto_char_position(at); })?;
        let insert_hooks = prepared.behind != Value::NIL || prepared.in_front != Value::NIL;
        Ok(prepared.before || self.vm.after_change_runs(false, insert_hooks)?)
    }

    pub(crate) fn print_into_buffer(&mut self, text: Value, stream: Value) -> std::result::Result<(), Fault> {
        let buffer = self.print_buffer(stream)?.unwrap();
        let current = self.current_buffer()?;
        let marker = self.kind(stream) == Kind::Marker;
        let point = if marker { Some(self.print_marker_point(stream, buffer)?) } else { None };
        self.vm.slots[self.vm.buffer_slot].value = buffer;
        let result = self.insert_printed_text(text);
        let result = result.and_then(|()| {
            if let Some((old, at)) = point {
                let end = self.point()?;
                self.vm.set_marker_object(stream, Value::integer(end).unwrap(), buffer).map_err(Fault::Error)?;
                let data::Object::Buffer(target) = &mut self.vm.objects[buffer.index(0).unwrap() - 1] else { unreachable!() };
                target.point = if old >= at { crate::coordinates::ByteOffset(old.0 + target.point.0 - at.0) } else { old };
            }
            Ok(())
        });
        self.vm.slots[self.vm.buffer_slot].value = current;
        result
    }

    fn print_marker_point(&mut self, stream: Value, buffer: Value) -> std::result::Result<(crate::coordinates::ByteOffset, crate::coordinates::ByteOffset), Fault> {
        let position = self.vm.marker_position_value(stream).map_err(Fault::Error)?.as_integer().map_err(Fault::Error)?;
        let data::Object::Buffer(target) = &mut self.vm.objects[buffer.index(0).unwrap() - 1] else {
            return Err(Fault::Error("printing to unibyte buffer markers is not supported".into()));
        };
        let at = target.byte_of_char(crate::coordinates::CharOffset(position as usize - 1)).map_err(Fault::Error)?;
        let accessible = target.accessible();
        if at < accessible.start || at > accessible.end {
            return Err(Fault::Error("Marker is outside the accessible part of the buffer".into()));
        }
        let old = target.point;
        target.point = at;
        Ok((old, at))
    }

    fn insert_printed_text(&mut self, text: Value) -> std::result::Result<(), Fault> {
        let view = self.string_view(text)?;
        if view.bytes().is_empty() { return Ok(()); }
        let inhibit = self.intern("inhibit-read-only");
        let inhibit = self.variable_value(inhibit)?.unwrap_or(Value::NIL);
        let read_only = self.intern("buffer-read-only");
        if inhibit == Value::NIL && self.variable_value(read_only)?.is_some_and(|v| v != Value::NIL)
            && self.fileio_property_at_point("inhibit-read-only")? == Value::NIL {
            let condition = self.intern("buffer-read-only");
            let buffer = self.current_buffer()?;
            return Err(Fault::Signal(condition, self.cons(buffer, Value::NIL)));
        }
        self.fileio_check_insertion(inhibit)?;
        let hooks = self.intern("inhibit-modification-hooks");
        let multibyte = self.casing_multibyte()?;
        let bytes = if !multibyte && view.is_multibyte() {
            view.chars().map(|code| u8::try_from(code).map_err(|_| Fault::NotUnicode(text))).collect::<std::result::Result<Vec<_>, _>>()?
        } else { view.bytes().to_vec() };
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let buffer = self.current_buffer()?;
        let unmodified = !self.vm.buffer_state(buffer).map_err(Fault::Error)?.modified();
        let (point, _, _) = self.vm.point_extent()?;
        match &mut self.vm.objects[index] {
            data::Object::Buffer(target) => target.replace(target.point..target.point, &bytes),
            data::Object::ByteBuffer(target) => target.replace(target.point..target.point, &bytes),
            _ => unreachable!(),
        }.map_err(Fault::Error)?;
        // insert_1_both's record_insert, and prepare's deactivate-mark.
        let (after, _, _) = self.vm.point_extent()?;
        self.vm.record_insert(index, point, after - point, unmodified).map_err(Fault::Error)?;
        if self.variable_value(hooks)?.unwrap_or(Value::NIL) == Value::NIL { self.vm.mark_deactivated()?; }
        self.vm.display_state.version += 1;
        Ok(())
    }
}
