//! xdisp.c window-text-pixel-size and buffer-text-pixel-size on the
//! initial terminal frame batch has: the display iterator over the
//! window's buffer (display_elements.rs) and its motion
//! (display_motion.rs), measured in the frame's characters. A window a
//! host lays out is measured by the same motion in the host's pixels
//! (display_hosted.rs), as are pos-visible-in-window-p and posn-at-point.
use super::*;
use super::display_elements::{Source, View};
use super::display_hosted::Hosted;
use super::display_motion::{self as motion, Ctx, Elements, It, Measure, Op, Wrap};
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
        let view = Self::view(area, buffer, begv, zv);
        let (column, line) = if area.hosted { (1, 1) } else { (area.column, area.line) };
        let mut source = Source::new(self, view, begv)?;
        let measure = |start, start_on_newline| Measure { start, end, begv, start_on_newline, x_limit: max_x.map(|x| x / column),
            max_y: max_y / line, ignore_line_at_end: args.ignore_line_at_end != Value::NIL };
        let (x, start_x, y, start) = if area.hosted {
            if vertical_offset != 0 { return Err(unsupported("from a vertical offset in a displayed window")); }
            let mut hosted = Hosted::new(source);
            let cx = hosted_context(&mut hosted, cx, area)?;
            let origin = hosted.source().visible_line_start(start)?;
            hosted.reset(origin)?;
            let it = motion::screen_line_start(&mut hosted, &cx, start)?;
            let on_newline = hosted.source().char_after(start)? == Some(0x0a);
            let (x, start_x, y) = motion::text_size(&mut hosted, &cx, it, &measure(start, on_newline))?;
            (x, start_x, y, start)
        } else {
            let (it, start) = start_line(&mut source, &cx, start, vertical_offset, line)?;
            let on_newline = source.char_after(start)? == Some(0x0a);
            let (x, start_x, y) = motion::text_size(&mut source, &cx, it, &measure(start, on_newline))?;
            (x, start_x, y, start)
        };
        let x = max_x.map_or(x * column, |m| (x * column).min(m)) - start_x * column;
        let y = (y * line).min(max_y) + self.mode_line_heights(area, args.mode_lines)?;
        let (x, y) = (Value::integer(x).map_err(Fault::Error)?, Value::integer(y).map_err(Fault::Error)?);
        if vertical_offset == 0 { return Ok(self.cons(x, y)); }
        let start = Value::integer(start).map_err(Fault::Error)?;
        let tail = self.cons(start, Value::NIL);
        let tail = self.cons(y, tail);
        Ok(self.cons(x, tail))
    }

    fn view(area: &TextArea, buffer: Value, begv: i64, zv: i64) -> View {
        View { window: area.window, buffer, begv, zv, column: area.column,
            box_width: area.columns - area.left_margin - area.right_margin, left_margin: area.left_margin, right_margin: area.right_margin }
    }

    /// Fvertical_motion (indent.c:2207): move point LINES screen lines (or,
    /// for (COLS . LINES), to column COLS of that line) as WINDOW (default
    /// the selected one) would show the current buffer; the screen lines
    /// moved. A window a host lays out moves over the host's layout. CUR-COL
    /// is checked, then the motion scans from the start of point's line, as
    /// GNU does without it.
    pub(crate) fn vertical_motion(&mut self, lines: Value, window: Value, cur_col: Value) -> super::super::Out {
        let number = |leaf: &Self, v: Value| -> std::result::Result<f64, Fault> {
            v.integer_value().map(|n| n as f64).or_else(|| match leaf.vm.object_if(v) {
                Some(Object::Float(f)) => Some(*f),
                _ => None,
            }).ok_or(Fault::Wrong("numberp", v))
        };
        let (cols, lines) = match self.pair(lines) {
            Some((cols, lines)) => (Some((number(self, cols)?, cols)), lines),
            None => (None, lines),
        };
        let lines = lines.integer_value().ok_or(Fault::Wrong("fixnump", lines))?;
        let area = self.windows().text_area(window, "vertical-motion")?;
        // Batch runs vmotion, which moves by lines only and reads no
        // CUR-COL (indent.c:2283).
        let noninteractive = self.intern("noninteractive");
        let interactive = self.variable_value(noninteractive)?.is_none_or(|v| v == Value::NIL);
        if interactive && cur_col != Value::NIL { number(self, cur_col)?; }
        let cols = cols.filter(|_| interactive);
        let buffer = self.current_buffer_value()?;
        let (begv, zv, pt) = match self.vm.object_if(buffer) {
            Some(Object::Buffer(b)) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?, pos(b.point)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        // window_column_x: COLS canonical columns in pixels, rounded.
        let x = match cols {
            Some((cols, value)) => {
                let x = cols * area.column as f64 + 0.5;
                if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&x) {
                    let condition = self.intern("args-out-of-range");
                    return Err(Fault::Signal(condition, self.vm.list_value(&[area.window, value])));
                }
                Some(x as i64)
            }
            None => None,
        };
        let cx = self.iterator_context(&area, zv)?;
        let view = Self::view(&area, buffer, begv, zv);
        let source = Source::new(self, view, begv)?;
        let (at, vpos) = if area.hosted {
            let mut hosted = Hosted::new(source);
            let cx = hosted_context(&mut hosted, cx, &area)?;
            motion::vertical_motion(&mut hosted, &cx, pt, lines, x)?
        } else {
            let mut source = source;
            motion::vertical_motion(&mut source, &cx, pt, lines, x.map(|x| x / area.column))?
        };
        self.goto_char_position(at)?;
        Value::integer(vpos).map_err(Fault::Error)
    }

    /// Fline_pixel_height (xdisp.c:1545): the height of the screen line
    /// showing point in the selected window's buffer, the default
    /// character's at the end of the buffer.
    pub(crate) fn line_pixel_height(&mut self) -> super::super::Out {
        let window = self.vm.selected_window_value().map_err(Fault::Error)?;
        let area = self.windows().text_area(window, "line-pixel-height")?;
        let buffer = area.buffer;
        let height = self.with_buffer(buffer, |leaf| {
            let (begv, zv, pt) = match leaf.vm.object_if(buffer) {
                Some(Object::Buffer(b)) => {
                    let r = b.accessible();
                    let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                    (pos(r.start)?, pos(r.end)?, pos(b.point)?)
                }
                _ => return Err(unsupported("in a unibyte buffer")),
            };
            let cx = leaf.iterator_context(&area, zv)?;
            let view = Self::view(&area, buffer, begv, zv);
            let source = Source::new(leaf, view, begv)?;
            if area.hosted {
                let mut hosted = Hosted::new(source);
                let cx = hosted_context(&mut hosted, cx, &area)?;
                let height = row_height(&mut hosted, &cx, pt)?;
                Ok(if height > 0 { height } else { hosted.default_advance(' ')?.1 })
            } else {
                let mut source = source;
                Ok(row_height(&mut source, &cx, pt)?.max(1) * area.line)
            }
        })?;
        Value::integer(height).map_err(Fault::Error)
    }

    /// Fpos_visible_in_window_p (window.c:1998): t, nil, or with PARTIALLY
    /// (X Y), or (X Y RTOP RBOT ROWH VPOS) for a partly visible row.
    pub(crate) fn pos_visible_in_window_p(&mut self, pos: Value, window: Value, partially: Value) -> super::super::Out {
        let caller = "pos-visible-in-window-p";
        let Some((w, charpos, top)) = self.windows().pos_visible(pos, window, caller)? else { return Ok(Value::NIL) };
        let Some(v) = self.hosted_pos_visible(w, charpos, top, caller)? else { return Ok(Value::NIL) };
        let fully = v.rtop == 0 && v.rbot == 0;
        if partially == Value::NIL { return Ok(if fully { Value::TRUE } else { Value::NIL }); }
        let mut items = vec![v.x, v.y];
        if !fully { items.extend([v.rtop, v.rbot, v.rowh, v.vpos]); }
        let items = items.into_iter().map(|n| Value::integer(n).map_err(Fault::Error)).collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(self.vm.list_value(&items))
    }

    /// Fposn_at_point (keyboard.c:13054): pos-visible-in-window-p with
    /// PARTIALLY t, then posn-at-x-y at the coordinates it reports. The
    /// initial frame shows nothing, so the answer there is nil.
    pub(crate) fn posn_at_point(&mut self, pos: Value, window: Value) -> super::super::Out {
        let caller = "posn-at-point";
        let window = if window == Value::NIL { self.vm.selected_window_value().map_err(Fault::Error)? } else { window };
        let Some((w, charpos, top)) = self.windows().pos_visible(pos, window, caller)? else { return Ok(Value::NIL) };
        let Some(v) = self.hosted_pos_visible(w, charpos, top, caller)? else { return Ok(Value::NIL) };
        let area = self.windows().text_area(w, caller)?;
        let buffer = area.buffer;
        let p = self.with_buffer(buffer, |leaf| leaf.hosted_posn(&area, top, v.x, v.y))?;
        let int = |n: i64| Value::integer(n).map_err(Fault::Error);
        let (x, y, zero, pos) = (int(v.x)?, int(v.y)?, int(0)?, int(p.pos)?);
        let xy = self.cons(x, y);
        let col_row = { let (c, r) = (int(p.col)?, int(p.row)?); self.cons(c, r) };
        let offset = { let (dx, dy) = (int(p.dx)?, int(p.dy)?); self.cons(dx, dy) };
        let size = { let (wd, ht) = (int(p.width)?, int(p.height)?); self.cons(wd, ht) };
        Ok(self.vm.list_value(&[w, pos, xy, zero, Value::NIL, pos, col_row, Value::NIL, offset, size]))
    }

    /// pos_visible_p (xdisp.c:1693) for CHARPOS in window W from its start
    /// TOP, over the host's layout. The host draws a row's glyphs from its
    /// top, so a glyph's Y is its row's, and a row is as tall as the host
    /// makes it. W's text is GNU's for the host only where no row breaks.
    fn hosted_pos_visible(&mut self, w: Value, charpos: i64, top: i64, caller: &str) -> std::result::Result<Option<Visible>, Fault> {
        let area = self.windows().text_area(w, caller)?;
        if charpos < 0 { return Err(unsupported("of the window end in a displayed window")); }
        if area.vscroll != 0 { return Err(unsupported("of a vertically scrolled displayed window")); }
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let (mut hosted, cx, mut it) = leaf.hosted_from(&area, top)?;
            let src = &mut hosted;
            let last_y = area.body_height;
            motion::move_to(src, &cx, &mut it, charpos, -1, last_y - 1, -1, Op { pos: true, y: true, ..Op::default() })?;
            if motion::charpos(src, &it)? < charpos {
                // Stopped by the window's bottom: CHARPOS may still be in
                // the row the bottom cuts.
                let row = it.vpos;
                motion::move_to(src, &cx, &mut it, charpos, -1, -1, -1, Op { pos: true, ..Op::default() })?;
                if it.vpos != row || motion::charpos(src, &it)? < charpos { return Ok(None); }
            }
            if it.current_y >= last_y { return Ok(None); }
            let mut rest = it;
            motion::move_in_line(src, &cx, &mut rest, -1, -1, Op::default())?;
            let height = if rest.max_h > 0 { rest.max_h } else { src.default_height };
            let bottom = it.current_y + height;
            Ok(Some(Visible { x: it.current_x, y: it.current_y, rtop: 0, rbot: (bottom - last_y).max(0),
                rowh: bottom.min(last_y) - it.current_y, vpos: it.vpos }))
        })
    }

    /// redisplay_window's window start over the host's layout (VM2-RD1),
    /// for WINDOW showing its buffer from START with POINT: None when POINT
    /// shows on a whole row; else, when MINIMAL, the start that just shows
    /// it on the first or last whole row, otherwise the start BACK screen
    /// lines above POINT's.
    pub(crate) fn hosted_window_start(&mut self, window: Value, start: i64, point: i64, minimal: bool, back: i64) -> std::result::Result<Option<i64>, Fault> {
        let caller = "redisplay";
        let area = self.windows().text_area(window, caller)?;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let shown = |leaf: &mut Self, top: i64| -> std::result::Result<bool, Fault> {
                Ok(point >= top && leaf.hosted_pos_visible(window, point, top, caller)?.is_some_and(|v| v.rbot == 0))
            };
            if shown(leaf, start)? { return Ok(None); }
            let above = |leaf: &mut Self, rows: i64| -> std::result::Result<i64, Fault> {
                let (mut hosted, cx, _) = leaf.hosted_from(&area, start.min(point))?;
                let it = motion::lines_up(&mut hosted, &cx, point, -rows)?;
                motion::charpos(&mut hosted, &it)
            };
            if !minimal || point < start { return Ok(Some(above(leaf, if minimal { 0 } else { back })?)); }
            // As many whole rows above POINT's as still show it.
            let mut rows = area.body_height / area.line.max(1);
            while rows > 0 {
                let top = above(leaf, rows)?;
                if shown(leaf, top)? { return Ok(Some(top)); }
                rows -= 1;
            }
            Ok(Some(above(leaf, 0)?))
        })
    }

    /// buffer_posn_from_coords (dispnew.c:6260) at text-area pixel X, Y:
    /// the glyph there, from the window start TOP.
    fn hosted_posn(&mut self, area: &TextArea, top: i64, x: i64, y: i64) -> std::result::Result<Posn, Fault> {
        let (mut hosted, cx, mut it) = self.hosted_from(area, top)?;
        motion::move_to(&mut hosted, &cx, &mut it, -1, 0, y, -1, Op { x: true, y: true, ..Op::default() })?;
        motion::move_in_line(&mut hosted, &cx, &mut it, cx.zv, x, Op { x: true, ..Op::default() })?;
        let pos = motion::charpos(&mut hosted, &it)?;
        // The glyph under X, else the space glyph GNU appends after a
        // line's end and at ZV, in the default face.
        let glyph = if it.loaded { hosted.slot(it.index)?.elem } else { None };
        let (width, height) = match glyph.map(|e| e.shape) {
            Some(motion::Shape::Pixel { c, .. }) if c != 0x0a => hosted.glyph(pos)?,
            Some(motion::Shape::PixelTab { space, .. }) => (motion::pixel_tab(&cx, &it, space), hosted.glyph(pos)?.1),
            _ => hosted.default_advance(' ')?,
        };
        Ok(Posn { pos, col: it.hpos, row: it.vpos, dx: x - it.current_x, dy: y - it.current_y, width, height })
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
        let wrap = self.vm.line_wrap(area.full_width, area.columns).map_err(Fault::Error)?;
        if wrap == Wrap::Word && var(self, "word-wrap-by-category")? != Value::NIL {
            return Err(unsupported("with `word-wrap-by-category'"));
        }
        let glyph = self.special_glyph_width(if wrap == Wrap::Truncate { 0 } else { 1 })?;
        let border = i64::from(!area.rightmost);
        let zv_after_newline = match self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) {
            Some(Object::Buffer(b)) => b.as_str()[..b.accessible().end.0].ends_with('\n'),
            _ => false,
        };
        if area.hosted && self.display_table()?.is_some() { return Err(unsupported("with a display table in a displayed window")); }
        Ok(Ctx {
            last_visible_x: area.columns - area.left_margin - area.right_margin - glyph - border,
            wrap,
            tab_width: self.tab_width()? as i64,
            zv,
            zv_after_newline,
            breaks: !area.hosted,
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

impl<'a> Leaf<'a> {
    /// The host-measured elements of AREA's window in its current buffer,
    /// their context, and the iterator at its start TOP, which may begin a
    /// continuation line (start_display).
    fn hosted_from(&mut self, area: &TextArea, top: i64) -> std::result::Result<(Hosted<'_, 'a>, Ctx, It), Fault> {
        let buffer = self.vm.slots[self.vm.buffer_slot].value;
        let (begv, zv) = match self.vm.object_if(buffer) {
            Some(Object::Buffer(b)) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        let cx = self.iterator_context(area, zv)?;
        let view = Self::view(area, buffer, begv, zv);
        let mut hosted = Hosted::new(Source::new(self, view, begv)?);
        let cx = hosted_context(&mut hosted, cx, area)?;
        let mut it = It::default();
        if top > begv && !hosted.source().after_newline(top)? {
            let origin = hosted.source().visible_line_start(top)?;
            hosted.reset(origin)?;
            it = motion::screen_line_start(&mut hosted, &cx, top)?;
            if motion::charpos(&mut hosted, &it)? != top {
                return Err(unsupported("of a displayed window that starts inside a screen line"));
            }
        } else {
            hosted.reset(top)?;
        }
        Ok((hosted, cx, it))
    }

}

/// pos_visible_p's answer: the glyph's text-area pixel position, the
/// pixels of its row above and below the text area, its visible height,
/// and the row.
struct Visible { x: i64, y: i64, rtop: i64, rbot: i64, rowh: i64, vpos: i64 }

/// buffer_posn_from_coords's answer.
struct Posn { pos: i64, col: i64, row: i64, dx: i64, dy: i64, width: i64, height: i64 }

/// init_iterator's last visible x on a window-system frame without
/// fringes, in the host's pixels: the text area less the truncation or
/// continuation glyph's advance in the default face (xdisp.c:3510-3518).
/// The host continues and truncates rows as GNU does, but wraps no words.
fn hosted_context(hosted: &mut Hosted<'_, '_>, mut cx: Ctx, area: &TextArea) -> std::result::Result<Ctx, Fault> {
    let (advance, _) = hosted.default_advance(if cx.wrap == Wrap::Truncate { '$' } else { '\\' })?;
    cx.last_visible_x = area.body_width - advance;
    cx.breaks = cx.wrap != Wrap::Word;
    Ok(cx)
}

/// line_bottom_y from the start of the screen line showing POS: its
/// height, 0 for the empty line at the end of the buffer.
fn row_height<E: motion::Lines>(src: &mut E, cx: &Ctx, pos: i64) -> std::result::Result<i64, Fault> {
    let mut it = motion::lines_up(src, cx, pos, 0)?;
    motion::move_in_line(src, cx, &mut it, -1, -1, Op::default())?;
    Ok(it.max_h)
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
