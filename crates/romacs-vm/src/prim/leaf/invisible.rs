//! Native access to the same annotation substrate as get-char-property.
use super::*;

impl Leaf<'_> {
    /// SANE_TAB_WIDTH of the current buffer.
    pub(crate) fn tab_width(&mut self) -> std::result::Result<usize, Fault> {
        let variable = self.intern("tab-width");
        Ok(self.variable_value(variable)?.and_then(|v| v.integer_value())
            .filter(|n| (1..=1000).contains(n)).unwrap_or(8) as usize)
    }
    /// Point's line within the accessible text, point's byte offset in it,
    /// the line's absolute byte start, and how a character of it advances
    /// in the buffer: by its own bytes, or in a unibyte buffer by one.
    fn line_text(&mut self) -> std::result::Result<(String, usize, usize, fn(char) -> usize), Fault> {
        use crate::text::Newlines as _;
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let (text, start, point, raw) = match &self.vm.objects[index] {
            crate::data::Object::Buffer(b) => {
                let (r, point, body) = (b.accessible(), b.point.0, b.body());
                let start = body.previous_newline(point, r.start.0).map_or(r.start.0, |at| at + 1);
                let end = body.next_newline(point, r.end.0).unwrap_or(r.end.0);
                let text = b.slice(crate::coordinates::ByteOffset(start)..crate::coordinates::ByteOffset(end)).map_err(Fault::Error)?;
                (text, start, point, b.raw_in(start..end).to_vec())
            }
            crate::data::Object::ByteBuffer(b) => {
                let (r, point) = (b.restriction.start.0..b.restriction.end.0, b.point.0);
                let start = b.bytes.previous_newline(point, r.start).map_or(r.start, |at| at + 1);
                let end = b.bytes.next_newline(point, r.end).unwrap_or(r.end);
                // Each byte a character; above 127, as a raw byte shows.
                let line: String = b.bytes[start..end].iter().map(|&byte| if byte < 128 { char::from(byte) } else { '\u{80}' }).collect();
                let at = line.char_indices().nth(point - start).map_or(line.len(), |(at, _)| at);
                return Ok((line, at, start, |_| 1));
            }
            _ => unreachable!(),
        };
        if !text.is_char_boundary(point - start) {
            return Err(Fault::Error("VM2 columns cannot display non-UTF-8 bytes".into()));
        }
        // A raw byte shows as `\NNN': four columns, like a C1 control.
        let line: String = text.char_indices()
            .map(|(at, c)| if raw.binary_search(&((start + at) as u32)).is_ok() { '\u{80}' } else { c })
            .collect();
        Ok((line, point - start, start, char::len_utf8))
    }
    pub fn set_point_byte(&mut self, byte: usize) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match self.vm.objects.entry_mut(index) {
            crate::data::Object::Buffer(b) => b.point = crate::coordinates::ByteOffset(byte),
            crate::data::Object::ByteBuffer(b) => b.point = crate::coordinates::ByteOffset(byte),
            _ => unreachable!(),
        }
        Ok(())
    }
    /// Point to character position POSITION (1-based), already in range.
    pub fn goto_char_position(&mut self, position: i64) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match self.vm.objects.entry_mut(index) {
            crate::data::Object::Buffer(b) => b.point = b.byte_of_char(crate::coordinates::CharOffset(position as usize - 1)).map_err(Fault::Error)?,
            crate::data::Object::ByteBuffer(b) => b.point = crate::coordinates::ByteOffset(position as usize - 1),
            _ => unreachable!(),
        }
        Ok(())
    }
    /// indent.c position_indentation from the start of point's line:
    /// spaces and tabs, and characters of category `\s'.
    pub fn current_indentation(&mut self) -> super::super::Out {
        let tab = self.tab_width()?;
        let (line, ..) = self.line_text()?;
        let table = self.current_category_table()?;
        let mut column = 0;
        for c in line.chars() {
            match c {
                ' ' => column += 1,
                '\t' => column += tab - column % tab,
                c if !c.is_ascii() && self.char_has_category(table, c as usize, b' ')? => column += 1,
                _ => break,
            }
        }
        Value::integer(column as i64).map_err(Fault::Error)
    }
    /// indent.c scan_for_column toward GOAL on point's line, moving point
    /// after the character that reaches it (or to the line end). Returns the
    /// column reached and the absolute byte position and column before the
    /// last character, with that character.
    pub fn scan_to_column(&mut self, goal: usize) -> std::result::Result<(usize, usize, usize, Option<char>), Fault> {
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        let (line, _, low, width) = self.line_text()?;
        let (mut column, mut at) = (0, 0);
        let (mut prev_at, mut prev_column, mut prev_char) = (0, 0, None);
        for c in line.chars() {
            if column >= goal { break; }
            (prev_at, prev_column, prev_char) = (at, column, Some(c));
            column += rule.columns(c as u32, column);
            at += width(c);
        }
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match self.vm.objects.entry_mut(index) {
            crate::data::Object::Buffer(b) => b.point = crate::coordinates::ByteOffset(low + at),
            crate::data::Object::ByteBuffer(b) => b.point = crate::coordinates::ByteOffset(low + at),
            _ => unreachable!(),
        }
        Ok((column, low + prev_at, prev_column, prev_char))
    }
    pub fn native_current_column(&mut self) -> super::super::Out {
        let rule = self.vm.width_rule().map_err(Fault::Error)?;
        let (line, point, ..) = self.line_text()?;
        let column = line[..point].chars().fold(0, |column, c| column + rule.columns(c as u32, column));
        Value::integer(column as i64).map_err(Fault::Error)
    }

    fn character_property(&mut self, pos: Value, key: Value) -> super::super::Out {
        // A non-evaluating query, on the kernel file: a running engine's
        // registers stay where they are.
        self.vm
            .with_kernel_stack(&[pos, key], |vm, n| vm.annotation_primitive(Primitive::GetCharProperty, 0, n))
            .map_err(Fault::Error)
    }

    /// TEXT_PROP_MEANS_INVISIBLE (xdisp.c invisible_prop) over the current
    /// buffer's `buffer-invisibility-spec': 0 visible, 1 invisible, 2
    /// invisible with an ellipsis. The whole PROPERTY is matched first,
    /// then each element of a list, both with eq; the first match wins.
    pub(crate) fn invisible_kind(&mut self, property: Value) -> std::result::Result<u8, Fault> {
        let symbol = self.intern("buffer-invisibility-spec");
        let spec = self.variable_value(symbol)?.unwrap_or(Value::NIL);
        if spec == Value::TRUE {
            return Ok(u8::from(property != Value::NIL));
        }
        let matched = self.match_invisibility_spec(property, spec);
        if matched != 0 { return Ok(matched); }
        let mut rest = property;
        for _ in 0..self.list_limit() {
            let Some((entry, tail)) = self.pair(rest) else { break };
            let matched = self.match_invisibility_spec(entry, spec);
            if matched != 0 { return Ok(matched); }
            rest = tail;
        }
        Ok(0)
    }
    fn match_invisibility_spec(&self, property: Value, mut spec: Value) -> u8 {
        for _ in 0..self.list_limit() {
            let Some((entry, tail)) = self.pair(spec) else { break };
            if property == entry { return 1; }
            if let Some((key, ellipsis)) = self.pair(entry) && property == key {
                return if ellipsis == Value::NIL { 1 } else { 2 };
            }
            spec = tail;
        }
        0
    }

    /// `invisible_kind` of the `invisible` char property at POS of BUFFER,
    /// the current buffer, as WINDOW shows it.
    pub(crate) fn shown_invisible_kind(&mut self, buffer: Value, window: Value, pos: i64) -> std::result::Result<u8, Fault> {
        let (byte, key) = (self.buffer_byte(buffer, pos)?, self.intern("invisible"));
        let property = self.vm.window_char_property(buffer, byte, key, window).map_err(Fault::Error)?.0;
        self.invisible_kind(property)
    }
    /// `handle_invisible_prop`'s loop over BUFFER, the current buffer, as
    /// WINDOW shows it, from POS in hidden text: the first shown position
    /// after it, at most END, adjacent stretches merged, and whether one
    /// passed asks for an ellipsis (or ELLIPSIS already did). It steps only
    /// where `invisible` or an overlay may change (`Vm::next_keys_change`).
    pub(crate) fn skip_hidden(&mut self, buffer: Value, window: Value, (mut pos, end): (i64, i64), mut ellipsis: bool)
        -> std::result::Result<(i64, bool), Fault> {
        let key = self.intern("invisible");
        let keys = self.vm.deciding_keys(key).map_err(Fault::Error)?;
        for steps in 1u32.. {
            if steps.is_multiple_of(1024) { self.vm.native_poll()?; }
            pos = self.vm.next_keys_change(buffer, (pos, end), &keys).map_err(Fault::Error)?;
            if pos >= end { break; }
            match self.shown_invisible_kind(buffer, window, pos)? {
                0 => break,
                2 => ellipsis = true,
                _ => {}
            }
        }
        Ok((pos, ellipsis))
    }

    pub fn invisible_property(&mut self, pos: Value) -> super::super::Out {
        // GNU deliberately treats negative fixnums, floats and bignums as
        // property values, not positions (FIXNATP, rather than INTEGERP).
        let marker = self.marker_position(pos);
        if !pos.integer_value().is_some_and(|n| n >= 0) && marker.is_none() {
            return Ok(pos);
        }
        let n = match marker {
            Some(Some(n)) => n,
            Some(None) => return Err(Fault::Error("Marker does not point anywhere".into())),
            None => pos.integer_value().unwrap(),
        };
        let shape = self.buffer_shape()?;
        if n < shape.begv || n > shape.zv {
            return Err(Fault::Range1(Value::integer(n).map_err(Fault::Error)?));
        }
        let key = self.intern("invisible");
        self.character_property(Value::integer(n).map_err(Fault::Error)?, key)
    }
}
