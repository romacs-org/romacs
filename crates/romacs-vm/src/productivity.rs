//! Synchronous local productivity primitives. Eval-stream is a trusted local
//! control interface, not a sandbox; subprocesses inherit the caller environment.
use crate::{Form, Result, Value, Vm};
#[path = "productivity_format.rs"]
mod numeric_format;
use numeric_format::FormatSpec;
impl Vm {
    /// Whether an owned background command completed since the last drain.
    pub fn background_processes_ready(&self) -> bool { self.process_jobs.has_ready() }

    /// Call WAKE, from another thread, whenever an owned background command
    /// completes: the embedder's wait blocks instead of polling for them.
    pub fn set_background_wake(&mut self, wake: std::sync::Arc<dyn Fn() + Send + Sync>) { self.process_jobs.set_wake(wake) }

    /// Cancel and settle background children before the native window shuts down.
    pub fn shutdown_background_processes(&mut self) -> Result<()> { self.process_jobs.shutdown() }

    pub(super) fn process_lines(&mut self, start: usize, n: usize) -> Result<Value> {
        if n == 0 {
            return Err("wrong primitive arity".into());
        }
        let args = self.stack[start..start + n]
            .iter()
            .map(|v| {
                self.str_view(*v)
                    .ok_or("string required")?
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "not-unicode-string".into())
            })
            .collect::<Result<Vec<_>>>()?;
        let output = crate::host::command(&args[0])
            .and_then(|mut command| command.args(&args[1..]).stdin(std::process::Stdio::null()).output())
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "process-lines: {} exited {}",
                args[0], output.status
            ));
        }
        let text =
            String::from_utf8(output.stdout).map_err(|_| "process-lines requires UTF-8 output")?;
        let mut lines = vec![];
        for line in text.split_terminator('\n') {
            lines.push(self.literal(&Form::String(line.to_owned()))?);
        }
        Ok(self.list_value(&lines))
    }
    pub(super) fn format_string(&mut self, start: usize, n: usize) -> Result<Value> {
        if n == 0 {
            return Err("wrong primitive arity".into());
        }
        let arguments = self.stack[start..start + n].to_vec();
        self.format_values(&arguments, None)
    }

    pub(crate) fn format_values(&mut self, arguments: &[Value], quoting_style: Option<&str>) -> Result<Value> {
        let format_value = arguments[0];
        let format = self
            .str_view(format_value)
            .ok_or("string required")?
            .to_owned();
        let byte_template = !format.is_multibyte();
        let mut out = Vec::new();
        let mut raw_high = false;
        let mut raw_ranges = Vec::new();
        let mut unicode_high = false;
        let message_multibyte = !byte_template || arguments[1..].iter()
            .any(|&value| self.str_view(value).is_some_and(|s| s.is_multibyte()));
        let mut argument = 0usize;
        let codes = format.chars().collect::<Vec<_>>();
        let mut chars = codes.iter().copied().peekable();
        // For styled_format's properties (editfns.c:4315): where each format
        // character lands in OUT, and each field's span with its argument.
        let mut landing = vec![0usize; codes.len() + 1];
        let mut fields: Vec<(std::ops::Range<usize>, Value)> = Vec::new();
        while let Some(c) = chars.next() {
            let at = codes.len() - chars.len() - 1;
            landing[at] = out.len();
            if c != b'%' as u32 {
                // Only literal template characters are requoted (GNU editfns.c).
                // Text produced by %s/%S is never passed through this branch.
                let c = match (quoting_style, c) {
                    (Some("curve"), 96) => 0x2018,
                    (Some("curve"), 39) => 0x2019,
                    (Some("straight"), 96) => 39,
                    _ => c,
                };
                if byte_template && c < 256 {
                    raw_high |= c >= 128;
                    if quoting_style.is_some() && c >= 128 {
                        raw_ranges.push(out.len()..out.len() + 1);
                    }
                    out.push(c as u8);
                } else {
                    unicode_high |= c >= 128;
                    let mut encoded = [0; 5];
                    let width = crate::lstring::char_string(c, &mut encoded);
                    out.extend_from_slice(&encoded[..width]);
                }
                continue;
            }
            let spec = FormatSpec::parse(&mut chars)?;
            let spec_end = codes.len() - chars.len();
            if let Some(index) = spec.argument { argument = index; }
            if spec.conversion == '%' {
                // The first `%' is discarded; the second is copied.
                landing[at + 1..spec_end].fill(out.len());
                out.push(b'%');
                continue;
            }
            let field_start = out.len();
            let value = *arguments.get(argument + 1).ok_or("not enough format arguments")?;
            argument += 1;
            let mut raw = false;
            let mut leading = 0;
            let piece = if spec.conversion == 'c' {
                let code = value.integer_value()
                    .ok_or("Format specifier doesn't match argument type")?;
                if !(0..=0x3fffff).contains(&code) {
                    return Err("format character out of range".into());
                }
                let code = code as u32;
                let mut encoded = [0; 5];
                let width = crate::lstring::char_string(code, &mut encoded);
                spec.character(encoded[..width].to_vec(), code)
            } else if spec.conversion == 's'
                && let Some(text) = self.str_view(value)
            {
                raw = !text.is_multibyte();
                let (piece, padding) = spec.text_field(text.bytes().to_vec(), !raw, 8);
                leading = padding;
                piece
            } else if matches!(spec.conversion, 's' | 'S') {
                // styled_format prints the argument with prin1 or princ
                // (editfns.c:3693): the printer and its variables decide,
                // but %s of a symbol is its name, `print-gensym' or not.
                let name = (spec.conversion == 's').then(|| value.index_if(7)).flatten()
                    .and_then(|i| self.symbol_names.get(i.checked_sub(1)?)).cloned();
                let printed = match name { Some(name) => name, None => self.print_render(value, spec.conversion == 'S', Value::NIL, true)? };
                let printed = crate::symkey::to_internal(&printed).unwrap_or_else(|| printed.into_bytes());
                spec.text(printed, true, 8)
            } else {
                let numeric = if matches!(spec.conversion, 'e' | 'f' | 'g') {
                    if let Some(number) = self.bigint_value(value)
                        && (i64::try_from(&number).is_ok() || u64::try_from(&number).is_ok()) {
                        spec.integer_float(&number)
                    } else {
                        spec.float(self.number(value).map_err(|_| "Format specifier doesn't match argument type")?)
                    }
                } else {
                    let float = self.bigint_value(value).is_none();
                    let number = if let Some(number) = self.bigint_value(value) { number }
                        else {
                            use num_bigint::ToBigInt;
                            let number = self.number(value).map_err(|_| "Format specifier doesn't match argument type")?;
                            if !number.is_finite() && matches!(spec.conversion, 'd' | 'i') {
                                out.extend_from_slice(spec.float(number).as_bytes());
                                landing[at + 1..spec_end].fill(out.len());
                                continue;
                            }
                            number.to_bigint().ok_or("floating-point overflow")?
                        };
                    if float { spec.truncated_float(&number) } else { spec.integer(&number) }
                };
                numeric.into_bytes()
            };
            if raw {
                raw_high |= !piece.is_ascii();
                if !piece.is_ascii() {
                    raw_ranges.push(out.len()..out.len() + piece.len());
                }
            } else {
                unicode_high |= !piece.is_ascii();
            }
            out.extend_from_slice(&piece);
            landing[at + 1..spec_end].fill(out.len());
            if spec.conversion == 's' && self.string_properties(value).is_some_and(|p| !p.is_empty()) {
                // styled_format (editfns.c:3786-3797): with a propertied
                // format string the field's left padding takes the
                // argument's properties too; without one it does not.
                let fmt_props = self.string_properties(format_value).is_some_and(|p| !p.is_empty());
                let start = if fmt_props { field_start } else { field_start + leading };
                fields.push((start..out.len(), value));
            }
        }
        landing[codes.len()] = out.len();
        if unicode_high && raw_high {
            // GNU promotes each unibyte octet separately, never as UTF-8.
            // Preserve source spans so literal Unicode and byte8 characters
            // already in a multibyte argument are not encoded twice.
            let mut promoted = Vec::with_capacity(out.len());
            let mut copied = 0;
            for range in raw_ranges {
                promoted.extend_from_slice(&out[copied..range.start]);
                for &byte in &out[range.clone()] {
                    let code = if byte >= 128 {
                        crate::lstring::BYTE8_OFFSET + u32::from(byte)
                    } else {
                        u32::from(byte)
                    };
                    let mut encoded = [0; 5];
                    let width = crate::lstring::char_string(code, &mut encoded);
                    promoted.extend_from_slice(&encoded[..width]);
                }
                copied = range.end;
            }
            promoted.extend_from_slice(&out[copied..]);
            return Ok(self.make_lisp_string(promoted, true));
        }
        if raw_high {
            let properties = self.format_properties(format_value, &landing, &fields, &out, false);
            return Ok(self.propertied_string(out, false, properties));
        }
        let multibyte = quoting_style.is_none() || message_multibyte || unicode_high;
        let properties = self.format_properties(format_value, &landing, &fields, &out, multibyte);
        Ok(self.propertied_string(out, multibyte, properties))
    }

    fn string_properties(&self, value: Value) -> Option<&crate::properties::Properties> {
        match self.object_if(value)? {
            crate::data::Object::String(s) | crate::data::Object::RawString(s) => Some(&s.properties),
            _ => None,
        }
    }

    /// The result's text properties, as styled_format sets them up
    /// (editfns.c:4315-4410): the format string's, each moved to where its
    /// characters landed (a run that reaches into a field covers the whole
    /// field), then each `%s' argument's at its field's start, with runs
    /// that end at the argument's end extended to the field's end
    /// (textprop.c extend_property_ranges).  OUT is the result's bytes.
    fn format_properties(&self, format: Value, landing: &[usize], fields: &[(std::ops::Range<usize>, Value)],
                         out: &[u8], multibyte: bool) -> crate::properties::Properties {
        use crate::coordinates::ByteOffset;
        let mut properties = crate::properties::Properties::new();
        let starts = |bytes: &[u8], multibyte: bool| -> Vec<usize> {
            if !multibyte { return (0..=bytes.len()).collect(); }
            let mut starts = Vec::new();
            let mut at = 0;
            while at < bytes.len() { starts.push(at); at += crate::lstring::string_char(&bytes[at..]).1; }
            starts.push(bytes.len());
            starts
        };
        if let (Some(own), Some(view)) = (self.string_properties(format).filter(|p| !p.is_empty()), self.str_view(format)) {
            let format_starts = starts(view.bytes(), view.is_multibyte());
            let char_of = |byte: ByteOffset| format_starts.partition_point(|&b| b < byte.0);
            for (range, values) in own.runs(ByteOffset(0)..ByteOffset(view.bytes().len())) {
                let span = landing[char_of(range.start)]..landing[char_of(range.end)];
                if span.is_empty() { continue; }
                for (key, value) in values { properties.put(ByteOffset(span.start)..ByteOffset(span.end), &key, value); }
            }
        }
        if fields.is_empty() { return properties; }
        let out_starts = starts(out, multibyte);
        for (field, argument) in fields {
            let (Some(own), Some(view)) = (self.string_properties(*argument), self.str_view(*argument)) else { continue };
            let argument_starts = starts(view.bytes(), view.is_multibyte());
            let char_of = |byte: ByteOffset| argument_starts.partition_point(|&b| b < byte.0);
            let old_end = argument_starts.len() - 1;
            let first = out_starts.partition_point(|&b| b < field.start);
            let new_end = out_starts.partition_point(|&b| b < field.end) - first;
            for (range, values) in own.runs(ByteOffset(0)..ByteOffset(view.bytes().len())) {
                let (begin, end) = (char_of(range.start), char_of(range.end));
                if begin >= new_end || values.is_empty() { continue; }
                let end = if (end == old_end && end != new_end) || end > new_end { new_end } else { end };
                let span = ByteOffset(out_starts[first + begin])..ByteOffset(out_starts[first + end]);
                for (key, value) in values { properties.put(span.clone(), &key, value); }
            }
        }
        properties
    }

    fn propertied_string(&mut self, bytes: Vec<u8>, multibyte: bool, properties: crate::properties::Properties) -> Value {
        if properties.is_empty() { return self.make_lisp_string(bytes, multibyte); }
        let mut string = crate::data::LispString::from_bytes(bytes, multibyte);
        string.properties = properties;
        let object = if string.is_utf8() { crate::data::Object::String(Box::new(string)) }
            else { crate::data::Object::RawString(Box::new(string)) };
        self.allocate_object(object)
    }
}
