//! xdisp.c window-text-pixel-size and buffer-text-pixel-size on the
//! initial terminal frame batch has: the display iterator over the
//! window's buffer (display_elements.rs) and its motion
//! (display_motion.rs), measured in the frame's characters.
use super::*;
use super::display_elements::{Source, View};
use super::display_motion::{self as motion, Ctx, Measure, Wrap};
use crate::data::Object;
use crate::window::TextArea;

const INT_MAX: i64 = i32::MAX as i64;

fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} is unsupported")) }

/// The arguments window_text_pixel_size takes after WINDOW.
pub(crate) struct TextSizeArgs { pub from: Value, pub to: Value, pub x_limit: Value, pub y_limit: Value, pub mode_lines: Value, pub ignore_line_at_end: Value }

impl Leaf<'_> {
    /// Fwindow_text_pixel_size (xdisp.c:12078): measure with WINDOW's
    /// buffer current.
    pub(crate) fn window_text_pixel_size(&mut self, window: Value, args: TextSizeArgs) -> super::super::Out {
        let area = self.windows().text_area(window, "window-text-pixel-size")?;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| leaf.text_pixel_size(&area, buffer, &args))
    }

    /// Fbuffer_text_pixel_size (xdisp.c:12158): BUFFER-OR-NAME's text as
    /// WINDOW would show it, without the mode lines.
    pub(crate) fn buffer_text_pixel_size(&mut self, buffer_or_name: Value, window: Value, x_limit: Value, y_limit: Value, from: Value, to: Value) -> super::super::Out {
        let area = self.windows().text_area(window, "buffer-text-pixel-size")?;
        let buffer = if buffer_or_name == Value::NIL { self.current_buffer_value()? } else { self.live_buffer_named(buffer_or_name)? };
        let args = TextSizeArgs { from, to, x_limit, y_limit, mode_lines: Value::NIL, ignore_line_at_end: Value::NIL };
        self.with_buffer(buffer, |leaf| leaf.text_pixel_size(&area, buffer, &args))
    }

    fn with_buffer<T>(&mut self, buffer: Value, body: impl FnOnce(&mut Self) -> std::result::Result<T, Fault>) -> std::result::Result<T, Fault> {
        let slot = self.vm.buffer_slot;
        let saved = self.vm.slots[slot].value;
        self.vm.slots[slot].value = buffer;
        let outcome = body(self);
        self.vm.slots[slot].value = saved;
        outcome
    }
    fn current_buffer_value(&mut self) -> std::result::Result<Value, Fault> {
        self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(self.vm.slots[self.vm.buffer_slot].value)
    }
    /// Fget_buffer, which must find a live buffer.
    fn live_buffer_named(&mut self, v: Value) -> std::result::Result<Value, Fault> {
        if matches!(self.vm.object_if(v), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            self.vm.require_live_buffer(v).map_err(Fault::Error)?;
            return Ok(v);
        }
        let name = self.string_text(v).map_err(|_| Fault::Wrong("stringp", v))?;
        self.vm.named_buffer(&name).ok_or(Fault::Wrong("bufferp", Value::NIL))
    }

    fn text_pixel_size(&mut self, area: &TextArea, buffer: Value, args: &TextSizeArgs) -> super::super::Out {
        let (begv, zv) = match self.vm.object_if(buffer) {
            Some(Object::Buffer(b)) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        let (start, vertical_offset) = self.measure_from(args.from, begv, zv)?;
        let end = self.measure_to(args.to, start, zv)?;
        let max_x = if args.x_limit.integer_value().is_some_and(|n| (0..=INT_MAX).contains(&n)) { args.x_limit.integer_value() }
            else if args.x_limit != Value::NIL { Some(INT_MAX) } else { None };
        let max_y = if args.y_limit == Value::NIL { INT_MAX }
            else { args.y_limit.integer_value().filter(|n| (0..=INT_MAX).contains(n)).unwrap_or(0) };
        let cx = self.iterator_context(area, zv)?;
        let view = View { window: area.window, buffer, begv, zv, column: area.column,
            box_width: area.columns - area.left_margin - area.right_margin, left_margin: area.left_margin, right_margin: area.right_margin };
        let (column, line) = (area.column, area.line);
        let mut source = Source::new(self, view, begv)?;
        let (it, start) = start_line(&mut source, &cx, start, vertical_offset, line)?;
        let start_on_newline = source.char_after(start)? == Some(0x0a);
        let measure = Measure { start, end, begv, start_on_newline, x_limit: max_x.map(|x| x / column), max_y: max_y / line,
            ignore_line_at_end: args.ignore_line_at_end != Value::NIL };
        let (x, start_x, y) = motion::text_size(&mut source, &cx, it, &measure)?;
        drop(source);
        let x = max_x.map_or(x * column, |m| (x * column).min(m)) - start_x * column;
        let y = (y * line).min(max_y) + self.mode_line_heights(area, args.mode_lines)?;
        let (x, y) = (Value::integer(x).map_err(Fault::Error)?, Value::integer(y).map_err(Fault::Error)?);
        if vertical_offset == 0 { return Ok(self.cons(x, y)); }
        let start = Value::integer(start).map_err(Fault::Error)?;
        let tail = self.cons(start, Value::NIL);
        let tail = self.cons(y, tail);
        Ok(self.cons(x, tail))
    }

    /// FROM: nil, t (the first non-blank line), a position, or
    /// (POSITION . VERTICAL-OFFSET).
    fn measure_from(&mut self, from: Value, begv: i64, zv: i64) -> std::result::Result<(i64, i64), Fault> {
        if from == Value::NIL { return Ok((begv, 0)); }
        if from == Value::TRUE {
            return self.accessible_scan(|text| {
                let mut at = text.iter().position(|b| !b" \t\n\r".contains(b)).unwrap_or(text.len());
                while at > 0 {
                    at -= 1;
                    if !b" \t".contains(&text[at]) { break; }
                }
                // dec_both stops on the character's first byte.
                while at > 0 && text[at] & 0xc0 == 0x80 { at -= 1; }
                at
            }).map(|start| (start, 0));
        }
        if let Some((position, offset)) = self.pair(from) {
            let start = self.fix_position(position)?.clamp(begv, zv);
            let offset = offset.integer_value().ok_or(Fault::Wrong("fixnump", offset))?;
            return Ok((start, offset));
        }
        Ok((self.fix_position(from)?.clamp(begv, zv), 0))
    }

    /// TO: nil, t (the end of the last non-blank line), or a position.
    fn measure_to(&mut self, to: Value, start: i64, zv: i64) -> std::result::Result<i64, Fault> {
        if to == Value::NIL { return Ok(zv); }
        if to != Value::TRUE { return Ok(self.fix_position(to)?.clamp(start, zv)); }
        self.accessible_scan(|text| {
            let mut at = text.iter().rposition(|b| !b" \t\n\r".contains(b)).map_or(0, |i| i + 1);
            while at < text.len() {
                let blank = b" \t".contains(&text[at]);
                at += 1;
                if !blank { break; }
            }
            at
        })
    }

    /// The position SCAN finds in the current buffer's accessible bytes.
    /// It tests only ASCII blanks; a byte inside a character it passed
    /// becomes the end of that character, as fetch_char_advance leaves it.
    fn accessible_scan(&mut self, scan: impl FnOnce(&[u8]) -> usize) -> std::result::Result<i64, Fault> {
        let Some(Object::Buffer(b)) = self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) else {
            return Err(unsupported("in a unibyte buffer"));
        };
        let r = b.accessible();
        let text = &b.as_str()[r.start.0..r.end.0];
        let mut at = scan(text.as_bytes());
        while !text.is_char_boundary(at) { at += 1; }
        Ok(b.char_of_byte(crate::coordinates::ByteOffset(r.start.0 + at)).map_err(Fault::Error)?.0 as i64 + 1)
    }

    /// init_iterator's line wrap and last visible x on a terminal frame.
    fn iterator_context(&mut self, area: &TextArea, zv: i64) -> std::result::Result<Ctx, Fault> {
        let var = |leaf: &mut Self, name: &str| -> std::result::Result<Value, Fault> {
            let symbol = leaf.intern(name);
            Ok(leaf.variable_value(symbol)?.unwrap_or(Value::NIL))
        };
        if var(self, "selective-display")? != Value::NIL { return Err(unsupported("with `selective-display'")); }
        if var(self, "display-line-numbers")? != Value::NIL { return Err(unsupported("with line numbers")); }
        if area.hscroll != 0 { return Err(unsupported("in a horizontally scrolled window")); }
        let partial = var(self, "truncate-partial-width-windows")?;
        let continued = (area.full_width || partial == Value::NIL || partial.integer_value().is_some_and(|n| n <= area.columns))
            && var(self, "truncate-lines")? == Value::NIL;
        let wrap = if !continued { Wrap::Truncate } else if var(self, "word-wrap")? == Value::NIL { Wrap::Window } else { Wrap::Word };
        if wrap == Wrap::Word && var(self, "word-wrap-by-category")? != Value::NIL {
            return Err(unsupported("with `word-wrap-by-category'"));
        }
        let glyph = self.special_glyph_width(if wrap == Wrap::Truncate { 0 } else { 1 })?;
        let border = i64::from(!area.rightmost);
        let zv_after_newline = match self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) {
            Some(Object::Buffer(b)) => b.as_str()[..b.accessible().end.0].ends_with('\n'),
            _ => false,
        };
        Ok(Ctx {
            last_visible_x: area.columns - area.left_margin - area.right_margin - glyph - border,
            wrap,
            tab_width: self.tab_width()? as i64,
            zv,
            zv_after_newline,
        })
    }

    /// The truncation (SLOT 0) or continuation (SLOT 1) glyph's columns:
    /// the display table's glyph, else `$' or `\'.
    fn special_glyph_width(&mut self, slot: usize) -> std::result::Result<i64, Fault> {
        let glyph = match self.display_table()? {
            Some(table) => match self.vm.object_if(table) {
                Some(Object::CharTable(t)) => t.extras.get(slot).copied().unwrap_or(Value::NIL),
                _ => Value::NIL,
            },
            None => Value::NIL,
        };
        let c = match self.pair(glyph) {
            Some((c, _)) => c.integer_value(),
            None => glyph.integer_value().filter(|&g| g >= 0).map(|g| g & 0x3fffff),
        };
        match c {
            Some(c) if (0x20..0x7f).contains(&c) || c < 0 => Ok(1),
            Some(c) => self.character_width(c as u32),
            None => Ok(1),
        }
    }

    /// The tab, header and mode lines MODE-LINES asks for. VM2 draws no
    /// tab or header line (DIVERGENCES.md VM2-WIN3).
    fn mode_line_heights(&mut self, area: &TextArea, mode_lines: Value) -> std::result::Result<i64, Fault> {
        let mode_line = self.intern("mode-line");
        Ok(if mode_lines == Value::TRUE || mode_lines == mode_line { area.mode_line } else { 0 })
    }
}

/// The start of the screen line window_text_pixel_size measures from,
/// and the position measuring starts at: START's own screen line, or the
/// one VERTICAL-OFFSET pixels of LINE-high rows above or below it.
fn start_line(source: &mut Source<'_, '_>, cx: &Ctx, start: i64, vertical_offset: i64, line: i64) -> std::result::Result<(motion::It, i64), Fault> {
    let mut origin = source.visible_line_start(start)?;
    let rows_up = if vertical_offset < 0 { (-vertical_offset + line - 1) / line } else { 0 };
    for _ in 0..rows_up {
        if origin <= source.begv() { break; }
        origin = source.visible_line_start(origin)?;
    }
    source.reset(origin)?;
    let mut it = motion::screen_line_start(source, cx, start)?;
    if vertical_offset == 0 { return Ok((it, start)); }
    if vertical_offset < 0 {
        let row = motion::screen_row(source, cx, start)?;
        it = motion::row_start(source, cx, (row - rows_up).max(0))?;
    } else {
        let rows = vertical_offset / line;
        motion::move_to(source, cx, &mut it, cx.zv, -1, rows, -1, motion::Op { pos: true, y: true, ..Default::default() })?;
        if motion::charpos(source, &it)? == cx.zv && cx.zv > source.begv() && !source.after_newline(cx.zv)? {
            it = motion::screen_line_start(source, cx, cx.zv)?;
        }
    }
    it.current_y = 0;
    let at = motion::charpos(source, &it)?.clamp(source.begv(), cx.zv);
    Ok((it, at))
}
