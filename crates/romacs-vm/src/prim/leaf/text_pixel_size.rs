//! xdisp.c window-text-pixel-size and buffer-text-pixel-size on the
//! initial terminal frame batch has: the display iterator over the
//! window's buffer (display_elements.rs) and its motion
//! (display_motion.rs), measured in the frame's characters. A window a
//! host lays out is measured by the same motion in the host's pixels
//! (display_hosted.rs), as are pos-visible-in-window-p and posn-at-point.
use super::*;
use super::display_elements::{Source, View};
use super::display_hosted::{Hosted, refused};
use super::display_motion::{self as motion, Ctx, Elements, It, Measure, Op, Wrap};
use crate::data::Object;
use crate::window::TextArea;

const INT_MAX: i64 = i32::MAX as i64;

fn unsupported(what: &str) -> Fault { Fault::Error(format!("VM2 window-text-pixel-size {what} is unsupported")) }

/// How a window labels its lines (maybe_produce_line_number): numbers of
/// at least WIDTH digits, relative to POINT_LINE (an index) or absolute,
/// point's absolute when CURRENT_ABSOLUTE, each glyph X wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineLabels { pub width: i64, pub x: i64, pub relative: bool, pub current_absolute: bool, pub point_line: i64 }

impl LineLabels {
    /// The label of the line at index LINE: its number padded to WIDTH
    /// plus a blank either side, longer when the number is.
    pub(crate) fn label(&self, line: i64) -> String {
        let mut shown = if self.relative { (line - self.point_line).abs() } else { line + 1 };
        if self.relative && shown == 0 && self.current_absolute { shown = self.point_line + 1; }
        format!("{shown:>width$} ", width = self.width as usize + 1)
    }
    /// The glyphs and the x the label of line LINE takes.
    pub(crate) fn prefix(&self, line: i64) -> (i64, i64) {
        let glyphs = self.label(line).len() as i64;
        (glyphs, glyphs * self.x)
    }
}

/// The arguments window_text_pixel_size takes after WINDOW.
pub(crate) struct TextSizeArgs { pub from: Value, pub to: Value, pub x_limit: Value, pub y_limit: Value, pub mode_lines: Value, pub ignore_line_at_end: Value }

impl Leaf<'_> {
    /// Fwindow_text_pixel_size (xdisp.c:12078): measure with WINDOW's
    /// buffer current.
    pub(crate) fn window_text_pixel_size(&mut self, window: Value, args: TextSizeArgs) -> super::super::Out {
        let area = self.windows().text_area(window, "window-text-pixel-size")?;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| leaf.approximate_text_pixel_size(&area, buffer, &args))
    }

    /// Fbuffer_text_pixel_size (xdisp.c:12158): BUFFER-OR-NAME's text as
    /// WINDOW would show it, without the mode lines.
    pub(crate) fn buffer_text_pixel_size(&mut self, buffer_or_name: Value, window: Value, x_limit: Value, y_limit: Value, from: Value, to: Value) -> super::super::Out {
        let area = self.windows().text_area(window, "buffer-text-pixel-size")?;
        let buffer = if buffer_or_name == Value::NIL { self.current_buffer_value()? } else { self.live_buffer_named(buffer_or_name)? };
        let args = TextSizeArgs { from, to, x_limit, y_limit, mode_lines: Value::NIL, ignore_line_at_end: Value::NIL };
        self.with_buffer(buffer, |leaf| leaf.approximate_text_pixel_size(&area, buffer, &args))
    }

    /// Interactive use never signals a layout refusal (VM2-WTPS1): where a
    /// displayed window's measurement refuses, the refusal is counted
    /// (`count_refusal') and APPROXIMATE answers; batch signals it.
    /// BODY, a motion Lisp called, which may quit at its polls. GNU's
    /// iterator quits nowhere; `vertical-motion' over a long buffer should.
    pub(crate) fn quittable_motion<T>(&mut self, body: impl FnOnce(&mut Self) -> std::result::Result<T, Fault>) -> std::result::Result<T, Fault> {
        let outer = std::mem::replace(&mut self.vm.motion_quit, true);
        let result = body(self);
        self.vm.motion_quit = outer;
        result
    }
    fn or_approximate<T>(&mut self, outcome: std::result::Result<T, Fault>,
                         approximate: impl FnOnce(&mut Self) -> std::result::Result<T, Fault>) -> std::result::Result<T, Fault> {
        match outcome {
            Err(fault) if refused(&fault) && self.vm.display_state.hosted => { self.count_refusal(&fault); approximate(self) }
            other => other,
        }
    }

    /// Count FAULT, a refusal, by its message and the current buffer's
    /// major mode, for the usage log (`romacs--layout-refusals').
    fn count_refusal(&mut self, fault: &Fault) {
        let Fault::Error(class) = fault else { return };
        let symbol = self.intern("major-mode");
        let mode = self.variable_value(symbol).ok().flatten().and_then(|m| self.symbol_name(m).ok()).unwrap_or_default();
        let refusals = &mut self.vm.display_state.refusals;
        match refusals.iter_mut().find(|r| r.class == *class && r.mode == mode) {
            Some(r) => r.n += 1,
            None => refusals.push(crate::display::LayoutRefusal { class: class.clone(), mode, n: 1 }),
        }
    }

    /// The refusals counted since the last call, as (CLASS MODE N) lists,
    /// and none counted after.
    pub(crate) fn take_layout_refusals(&mut self) -> super::super::Out {
        let refusals = std::mem::take(&mut self.vm.display_state.refusals);
        let mut items = Vec::with_capacity(refusals.len());
        for r in refusals {
            let class = self.string(&r.class);
            let mode = self.string(&r.mode);
            let n = Value::integer(r.n as i64).map_err(Fault::Error)?;
            items.push(self.vm.list_value(&[class, mode, n]));
        }
        Ok(self.vm.list_value(&items))
    }

    /// window-text-pixel-size over AREA, or where the host's layout
    /// refuses, the frame's character cells, else BUFFER's lines.
    fn approximate_text_pixel_size(&mut self, area: &TextArea, buffer: Value, args: &TextSizeArgs) -> super::super::Out {
        let outcome = self.text_pixel_size(area, buffer, args);
        self.or_approximate(outcome, |leaf| {
            let cells = TextArea { hosted: false, ..area.clone() };
            match leaf.text_pixel_size(&cells, buffer, args) {
                Err(fault) if refused(&fault) => leaf.line_text_size(area, buffer, args),
                other => other,
            }
        })
    }

    /// BUFFER's text from FROM to TO measured by its lines, each line a
    /// row and each character a column of AREA's frame, no wider than its
    /// text area: what any layout shows at the least.
    fn line_text_size(&mut self, area: &TextArea, buffer: Value, args: &TextSizeArgs) -> super::super::Out {
        let (begv, zv, _) = self.text_between(buffer, 0, 0)?;
        let start = self.measure_from(args.from, begv, zv)?.0;
        let end = self.measure_to(args.to, start, zv)?;
        let (_, _, text) = self.text_between(buffer, start, end)?;
        let (max_x, max_y) = limits(args);
        let widest = text.split('\n').map(|l| l.chars().count() as i64).max().unwrap_or(0);
        let x = (widest * area.column).min(area.body_width).min(max_x.unwrap_or(INT_MAX));
        let y = (text.split('\n').count() as i64 * area.line).min(max_y) + self.mode_line_heights(area, args.mode_lines)?;
        let (x, y) = (Value::integer(x).map_err(Fault::Error)?, Value::integer(y).map_err(Fault::Error)?);
        Ok(self.cons(x, y))
    }

    /// BUFFER's accessible positions and its text between positions A and
    /// B, in either order and clamped to them; a unibyte buffer's bytes are
    /// each a character.
    fn text_between(&self, buffer: Value, a: i64, b: i64) -> std::result::Result<(i64, i64, String), Fault> {
        let order = |begv: i64, zv: i64| (a.min(b).clamp(begv, zv), a.max(b).clamp(begv, zv));
        match self.vm.object_if(buffer) {
            Some(Object::Buffer(buf)) => {
                let r = buf.accessible();
                let pos = |byte| buf.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                let (begv, zv) = (pos(r.start)?, pos(r.end)?);
                let (lo, hi) = order(begv, zv);
                let byte = |p: i64| buf.byte_of_char(crate::coordinates::CharOffset((p - 1) as usize)).map(|o| o.0).map_err(Fault::Error);
                Ok((begv, zv, buf.as_str()[byte(lo)?..byte(hi)?].to_owned()))
            }
            Some(Object::ByteBuffer(buf)) => {
                let (begv, zv) = (buf.restriction.start.0 as i64 + 1, buf.restriction.end.0 as i64 + 1);
                let (lo, hi) = order(begv, zv);
                Ok((begv, zv, buf.bytes[(lo - 1) as usize..(hi - 1) as usize].iter().map(|&byte| char::from(byte)).collect()))
            }
            _ => Err(Fault::Wrong("bufferp", buffer)),
        }
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
        let (begv, zv) = match self.vm.shown_text(buffer).ok() {
            Some(b) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        let (start, vertical_offset) = self.measure_from(args.from, begv, zv)?;
        let end = self.measure_to(args.to, start, zv)?;
        let (max_x, max_y) = limits(args);
        let cx = self.iterator_context(area, zv, start)?;
        let view = Self::view(area, buffer, begv, zv, cx.lnum);
        let (column, line) = if area.hosted { (1, 1) } else { (area.column, area.line) };
        let mut source = Source::new(self, view, begv)?;
        let measure = |start, start_on_newline| Measure { start, end, begv, start_on_newline, x_limit: max_x.map(|x| x / column),
            max_y: max_y / line, ignore_line_at_end: args.ignore_line_at_end != Value::NIL };
        let (x, start_x, y, start) = if area.hosted {
            if vertical_offset != 0 { return Err(unsupported("from a vertical offset in a displayed window")); }
            let mut hosted = Hosted::new(source);
            let cx = hosted_context(&mut hosted, cx, area)?;
            // GNU starts at the previous visible line start and moves to
            // START (window-text-pixel-size, xdisp.c); a row starts at START
            // when a plain newline precedes it, so the text before cannot
            // change it.
            let origin = if hosted.source().starts_row(start)? { start } else { hosted.source().visible_line_start(start)? };
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

    /// What AREA's window shows of BUFFER: in columns on the terminal
    /// frame, in the host's pixels once it lays the window out.
    fn view(area: &TextArea, buffer: Value, begv: i64, zv: i64, lnum: Option<LineLabels>) -> View {
        let column = area.column;
        // A display specification counts the label of the estimated width.
        let lnum = lnum.map_or(0, |l| (l.width + 2) * l.x);
        if area.hosted {
            return View { window: area.window, buffer, begv, zv, column, box_width: area.body_width,
                left_margin: area.left_margin * column, right_margin: area.right_margin * column, pixels: true, lnum };
        }
        View { window: area.window, buffer, begv, zv, column,
            box_width: area.columns - area.left_margin - area.right_margin, left_margin: area.left_margin, right_margin: area.right_margin, pixels: false, lnum }
    }

    /// The stretch each of SPECS, a list of `(space ...)' display
    /// specifications in WINDOW's header or mode line, makes there, in the
    /// pixels of a window-system frame: produce_stretch_glyph's width as
    /// calc_pixel_width_or_height computes it for the window's text area,
    /// an `:align-to' target from its left edge (VM2-MODE-LINE-STRETCH);
    /// None for another display. The window's geometry is read once.
    pub(crate) fn band_stretches(&mut self, window: Value, specs: Value) -> std::result::Result<Vec<Option<romacs_layout_input::StretchWidth>>, Fault> {
        let mut area = self.windows().text_area(window, "redisplay")?;
        area.hosted = true;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let (begv, zv) = leaf.accessible_positions(buffer)?;
            let specs = leaf.vm.list_values(specs).map_err(Fault::Error)?;
            let mut source = Source::new(leaf, Self::view(&area, buffer, begv, zv, None), begv)?;
            specs.into_iter().map(|spec| source.band_stretch(spec)).collect()
        })
    }

    /// The elements WINDOW shows of its buffer from FROM, a line start,
    /// through TO, in the pixels of a window-system frame (VM2-WTPS1).
    pub(crate) fn window_shown_elements(&mut self, window: Value, from: i64, to: i64) -> std::result::Result<(Vec<super::display_hosted::Shown>, Vec<romacs_layout_input::InlineImage>), Fault> {
        let mut area = self.windows().text_area(window, "redisplay")?;
        area.hosted = true;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let (begv, zv) = leaf.accessible_positions(buffer)?;
            let top = leaf.windows().window_end(window)?.2;
            let point = leaf.vm.window_point_value(window).map_err(Fault::Error)?.as_integer().map_err(Fault::Error)?;
            let lnum = leaf.line_numbers(&area, top, point)?;
            leaf.shown_elements(Self::view(&area, buffer, begv, zv, lnum), from, to)
        })
    }

    pub(crate) fn vm_selected_window(&mut self) -> std::result::Result<Value, Fault> {
        self.vm.selected_window_value().map_err(Fault::Error)
    }
    fn position_or_nil(position: Option<i64>) -> super::super::Out {
        position.map_or(Ok(Value::NIL), |p| Value::integer(p).map_err(Fault::Error))
    }
    pub(crate) fn window_recenter_start(&mut self, window: Value, arg: Option<i64>, lines: (i64, i64)) -> super::super::Out {
        Self::position_or_nil(self.vm.recenter_start(window, arg, lines).map_err(Fault::Error)?)
    }
    pub(crate) fn window_scroll_start(&mut self, window: Value, start: i64, forward: bool, context: i64) -> super::super::Out {
        Self::position_or_nil(self.vm.scroll_start(window, start, forward, context).map_err(Fault::Error)?)
    }
    pub(crate) fn window_rows_shown_from(&mut self, window: Value, start: i64) -> super::super::Out {
        Self::position_or_nil(self.vm.rows_shown_from(window, start).map_err(Fault::Error)?)
    }

    /// The windows argument `pre-redisplay-function' takes now, if a host
    /// lays the frame out and it has anything to do.
    pub(crate) fn pre_redisplay_call(&mut self) -> std::result::Result<Option<Value>, Fault> {
        if !self.vm.has_layout_host() { return Ok(None); }
        self.vm.pre_redisplay_call().map_err(Fault::Error)
    }

    /// Settle each live window's start as the next layout will
    /// (`Vm::redisplay_point_visibility_now'), and record the end that
    /// layout will show, so `window-end' without UPDATE (isearch's
    /// `window-group-end') answers the new display too.
    pub(crate) fn redisplay_point_visibility_now(&mut self) -> std::result::Result<(), Fault> {
        if !self.vm.has_layout_host() { return Ok(()); }
        self.vm.redisplay_point_visibility_now().map_err(Fault::Error)?;
        for w in self.vm.leaf_windows().map_err(Fault::Error)? {
            let shown = matches!(self.vm.object_if(w), Some(Object::Window(x)) if x.live() && !x.mini && x.height > 0);
            if !shown { continue; }
            // A window not laid out yet ends where motion reaches; one
            // the measurement cannot follow keeps its last end.
            let laid = self.vm.window_layout(w).map_err(Fault::Error)?.map(|laid| laid.end);
            let walked = if laid.is_none() { self.window_end(w, Value::TRUE).ok().and_then(|v| v.integer_value()) } else { None };
            let buffer = self.vm.window(w).map_err(Fault::Error)?.buffer;
            let Ok(b) = self.vm.shown_text(buffer) else { continue };
            let end = match (laid, walked) {
                (Some(byte), _) => b.char_of_byte(coordinates::ByteOffset(byte)).map_err(Fault::Error)?.0 as i64 + 1,
                (None, Some(end)) => end,
                (None, None) => continue,
            };
            let distance = (b.len_chars() as i64 + 1 - end).max(0) as usize;
            let index = w.index(0).map_err(Fault::Error)? - 1;
            if let Object::Window(x) = &mut self.vm.objects[index] { x.end_distance = distance; }
            if laid.is_some() { self.vm.record_end_laid(w).map_err(Fault::Error)?; }
        }
        Ok(())
    }

    /// Whether WINDOW's buffer bytes FROM..TO show other glyphs than their
    /// characters (`text_shows_glyphs').
    pub(crate) fn window_text_shows_glyphs(&mut self, window: Value, from: usize, to: usize) -> std::result::Result<bool, Fault> {
        let buffer = self.windows().text_area(window, "redisplay")?.buffer;
        self.with_buffer(buffer, |leaf| leaf.text_shows_glyphs(from, to))
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
        let (begv, zv, pt) = match self.vm.shown_text(buffer).ok() {
            Some(b) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?, pos(b.point)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        // window_column_x: COLS canonical columns in pixels, rounded.
        let goal = |leaf: &mut Self, area: &TextArea| -> std::result::Result<Option<i64>, Fault> {
            let Some((cols, value)) = cols else { return Ok(None) };
            let x = cols * area.column as f64 + 0.5;
            if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&x) {
                let condition = leaf.intern("args-out-of-range");
                return Err(Fault::Signal(condition, leaf.vm.list_value(&[area.window, value])));
            }
            // The goal counts from after the line number, and in a
            // horizontally scrolled window adds to the scrolled
            // columns (first_x + to_x).
            let lnum = if area.hosted { leaf.line_number_widths(area)?.1 } else { 0 };
            Ok(Some(x as i64 + lnum + area.first_visible * area.column))
        };
        let motion = |leaf: &mut Self, area: &TextArea| -> std::result::Result<(i64, i64), Fault> {
            let x = goal(leaf, area)?;
            let cx = leaf.iterator_context(area, zv, pt)?;
            let view = Self::view(area, buffer, begv, zv, cx.lnum);
            let source = Source::new(leaf, view, begv)?;
            if area.hosted {
                let mut hosted = Hosted::new(source);
                let cx = hosted_context(&mut hosted, cx, area)?;
                motion::vertical_motion(&mut hosted, &cx, pt, lines, x)
            } else {
                let mut source = source;
                motion::vertical_motion(&mut source, &cx, pt, lines, x.map(|x| x / area.column))
            }
        };
        // What the host's layout cannot measure moves as a terminal frame
        // would, by the frame's character cells (GNU's line-based paths);
        // what the iterator cannot follow at all, by the buffer's lines:
        // motion, and the scrolling and recentering built on it, never
        // fails for a measurement it lacks.
        let (at, vpos) = match motion(self, &area) {
            Err(fault) if refused(&fault) && area.hosted => {
                self.count_refusal(&fault);
                match motion(self, &TextArea { hosted: false, ..area.clone() }) {
                    Err(fault) if refused(&fault) => self.buffer_line_motion(pt, lines)?,
                    other => other?,
                }
            }
            Err(fault) if refused(&fault) => self.buffer_line_motion(pt, lines)?,
            other => other?,
        };
        self.goto_char_position(at)?;
        Value::integer(vpos).map_err(Fault::Error)
    }

    /// The start of the buffer line LINES lines from PT's, within the
    /// accessible text (its end past the last line), and the lines moved:
    /// vertical motion where every buffer line is one screen line.
    fn buffer_line_motion(&mut self, pt: i64, lines: i64) -> std::result::Result<(i64, i64), Fault> {
        let Ok(b) = self.vm.shown_text(self.vm.slots[self.vm.buffer_slot].value) else {
            return Err(unsupported("in a unibyte buffer"));
        };
        let r = b.accessible();
        let text = &b.as_str()[r.start.0..r.end.0];
        let at = b.byte_of_char(crate::coordinates::CharOffset((pt - 1).max(0) as usize)).map_err(Fault::Error)?.0.clamp(r.start.0, r.end.0) - r.start.0;
        let mut start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let mut moved = 0;
        while moved < lines {
            // Past the last line break, the end of the text (vmotion
            // stops at ZV).
            let Some(i) = text[start..].find('\n') else { start = text.len(); break };
            start += i + 1;
            moved += 1;
        }
        while moved > lines && start > 0 {
            start = text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
            moved -= 1;
        }
        let pos = b.char_of_byte(crate::coordinates::ByteOffset(r.start.0 + start)).map_err(Fault::Error)?.0 as i64 + 1;
        Ok((pos, moved))
    }

    /// Fline_pixel_height (xdisp.c:1545): the height of the screen line
    /// showing point in the selected window's buffer, the default
    /// character's at the end of the buffer.
    pub(crate) fn line_pixel_height(&mut self) -> super::super::Out {
        let window = self.vm.selected_window_value().map_err(Fault::Error)?;
        let area = self.windows().text_area(window, "line-pixel-height")?;
        let buffer = area.buffer;
        let height = self.with_buffer(buffer, |leaf| {
            let (begv, zv, pt) = match leaf.vm.shown_text(buffer).ok() {
                Some(b) => {
                    let r = b.accessible();
                    let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                    (pos(r.start)?, pos(r.end)?, pos(b.point)?)
                }
                _ => return Err(unsupported("in a unibyte buffer")),
            };
            let measured = |leaf: &mut Self| -> std::result::Result<i64, Fault> {
                let cx = leaf.iterator_context(&area, zv, pt)?;
                let view = Self::view(&area, buffer, begv, zv, cx.lnum);
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
            };
            // A row the iterator or the host's layout cannot measure is a
            // character row.
            match measured(leaf) {
                Err(fault) if refused(&fault) => {
                    if area.hosted { leaf.count_refusal(&fault); }
                    Ok(area.line)
                }
                other => other,
            }
        })?;
        Value::integer(height).map_err(Fault::Error)
    }

    /// Fpos_visible_in_window_p (window.c:1998): t, nil, or with PARTIALLY
    /// (X Y), or (X Y RTOP RBOT ROWH VPOS) for a partly visible row.
    pub(crate) fn pos_visible_in_window_p(&mut self, pos: Value, window: Value, partially: Value) -> super::super::Out {
        let caller = "pos-visible-in-window-p";
        let Some((w, charpos, top)) = self.windows().pos_visible(pos, window, caller)? else { return Ok(Value::NIL) };
        let Some(v) = self.visible_or_approximate(w, charpos, top, caller)? else { return Ok(Value::NIL) };
        let fully = v.rtop == 0 && v.rbot == 0;
        if partially == Value::NIL { return Ok(if fully { Value::TRUE } else { Value::NIL }); }
        let mut items = vec![v.x, v.y];
        if !fully { items.extend([v.rtop, v.rbot, v.rowh, v.vpos]); }
        let items = items.into_iter().map(|n| Value::integer(n).map_err(Fault::Error)).collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(self.vm.list_value(&items))
    }

    /// Fwindow_end (window.c:1782). With UPDATE, a window a host lays out
    /// answers by the motion from its start, as GNU recomputes an end its
    /// display may not have caught up with; elsewhere, as GNU's batch, the
    /// end the last layout recorded.
    pub(crate) fn window_end(&mut self, window: Value, update: Value) -> super::super::Out {
        let (w, recorded, top) = self.windows().window_end(window)?;
        if update == Value::NIL || !self.vm.display_state.hosted || self.vm.window_end_current(w).map_err(Fault::Error)? {
            return Value::integer(recorded).map_err(Fault::Error);
        }
        let area = self.windows().text_area(w, "window-end")?;
        let end = self.with_buffer(area.buffer, |leaf| {
            let measured = if area.vscroll != 0 { Err(unsupported("UPDATE of a vertically scrolled window's end")) } else {
                leaf.hosted_from(&area, top).and_then(|(mut hosted, cx, it)| motion::window_end(&mut hosted, &cx, it, area.body_height))
            };
            // Where the host's layout refuses, the end the last layout
            // recorded, within the window's text now.
            leaf.or_approximate(measured, |leaf| {
                let (_, zv, _) = leaf.text_between(area.buffer, 0, 0)?;
                Ok(recorded.clamp(top.min(zv), zv))
            })
        })?;
        Value::integer(end).map_err(Fault::Error)
    }

    /// Fposn_at_point (keyboard.c:13054): pos-visible-in-window-p with
    /// PARTIALLY t, then posn-at-x-y at the coordinates it reports. The
    /// initial frame shows nothing, so the answer there is nil.
    pub(crate) fn posn_at_point(&mut self, pos: Value, window: Value) -> super::super::Out {
        let caller = "posn-at-point";
        let window = if window == Value::NIL { self.vm.selected_window_value().map_err(Fault::Error)? } else { window };
        let Some((w, charpos, top)) = self.windows().pos_visible(pos, window, caller)? else { return Ok(Value::NIL) };
        let Some(v) = self.visible_or_approximate(w, charpos, top, caller)? else { return Ok(Value::NIL) };
        // Point scrolled out on the left shows nowhere.
        if v.x < -1 { return Ok(Value::NIL); }
        let area = self.windows().text_area(w, caller)?;
        let buffer = area.buffer;
        // posn-at-x-y's Y counts from the window's top, above its header line.
        let p = self.with_buffer(buffer, |leaf| {
            let measured = leaf.hosted_posn(&area, top, v.x, v.y - area.header_line);
            // Where the host's layout refuses, the cell V's coordinates fall in.
            leaf.or_approximate(measured, |_| Ok(Posn { pos: charpos, col: v.x / area.column.max(1), row: v.vpos, dx: 0, dy: 0,
                                                       width: area.column, height: area.line, string: None, image: None }))
        })?;
        let int = |n: i64| Value::integer(n).map_err(Fault::Error);
        // At the text area's right edge, as past a truncated row's last
        // glyph, X is on the right fringe (coordinates_in_window): its
        // posn counts X from the window's left edge, column 0, DX from the
        // fringe (make_lispy_position).
        let [left_fringe, _] = self.vm.window_fringe_widths(w).map_err(Fault::Error)?.map(i64::from);
        let fringe = area.right_fringe > 0 && v.x >= area.body_width && v.x < area.body_width + area.right_fringe;
        let (area_name, x, col, dx) = if fringe {
            let wx = v.x + left_fringe + area.left_margin * area.column;
            (self.intern("right-fringe"), wx, 0, v.x - area.body_width)
        } else {
            (Value::NIL, v.x, p.col, p.dx)
        };
        // The position's (X . Y) counts from the text area's top, below
        // the header line (make_lispy_position).
        let (x, y, zero, pos) = (int(x)?, int(v.y - area.header_line)?, int(0)?, int(p.pos)?);
        let xy = self.cons(x, y);
        let col_row = { let (c, r) = (int(col)?, int(p.row)?); self.cons(c, r) };
        let offset = { let (dx, dy) = (int(dx)?, int(p.dy)?); self.cons(dx, dy) };
        let size = { let (wd, ht) = (int(p.width)?, int(p.height)?); self.cons(wd, ht) };
        // A string's glyph names the string and its position there.
        let object = match p.string { Some((string, at)) => { let at = int(at as i64)?; self.cons(string, at) } None => Value::NIL };
        let place = if area_name == Value::NIL { pos } else { area_name };
        Ok(self.vm.list_value(&[w, place, xy, zero, object, pos, col_row, p.image.unwrap_or(Value::NIL), offset, size]))
    }

    /// hosted_pos_visible, or where the host's layout refuses, CHARPOS's
    /// place counted in W's buffer lines and frame cells from TOP.
    fn visible_or_approximate(&mut self, w: Value, charpos: i64, top: i64, caller: &str) -> std::result::Result<Option<Visible>, Fault> {
        let measured = self.hosted_pos_visible(w, charpos, top, caller);
        if measured.is_ok() { return measured; }
        // Counted with W's buffer current, for its major mode.
        let area = self.windows().text_area(w, caller)?;
        self.with_buffer(area.buffer, |leaf| leaf.or_approximate(measured, |leaf| leaf.line_visible(&area, charpos, top)))
    }

    /// CHARPOS's place in AREA's window from its start TOP when every
    /// character is a cell of the frame: each buffer line its rows, as
    /// many as its cells fill when lines continue, from the window's
    /// hscroll when they are truncated, and the rows less the window's
    /// vscroll; nil before TOP, where its row is not wholly in the text
    /// area, or left or right of it on a truncated line.
    fn line_visible(&mut self, area: &TextArea, charpos: i64, top: i64) -> std::result::Result<Option<Visible>, Fault> {
        let (_, zv, _) = self.text_between(area.buffer, 0, 0)?;
        let charpos = if charpos < 0 { zv } else { charpos };
        if charpos < top { return Ok(None); }
        let (_, _, text) = self.text_between(area.buffer, top, charpos)?;
        let continued = area.hscroll == 0
            && self.vm.line_wrap(area.full_width, area.columns).map_err(Fault::Error)? != romacs_layout_input::LineWrap::Truncate;
        let width = area.body_width.max(area.column);
        let cells = |line: &str| line.chars().count() as i64 * area.column;
        let mut lines = text.split('\n').collect::<Vec<_>>();
        let last = cells(lines.pop().unwrap_or(""));
        let rows = |px: i64| if continued { 1 + px.saturating_sub(1) / width } else { 1 };
        let row = lines.iter().map(|line| rows(cells(line))).sum::<i64>() + if continued { last / width } else { 0 };
        let x = if continued { last % width } else { last - area.first_visible * area.column };
        let y = area.header_line + row * area.line - area.vscroll;
        if y < area.header_line || y + area.line > area.header_line + area.body_height || !(0..area.body_width).contains(&x) { return Ok(None); }
        Ok(Some(Visible { x, y, rtop: 0, rbot: 0, rowh: area.line, vpos: row }))
    }

    /// pos_visible_p (xdisp.c:1693) for CHARPOS in window W from its start
    /// TOP, over the host's layout. The host draws a row's glyphs from its
    /// top, so a glyph's Y is its row's, and a row is as tall as the host
    /// makes it. W's text is GNU's for the host only where no row breaks.
    fn hosted_pos_visible(&mut self, w: Value, charpos: i64, top: i64, caller: &str) -> std::result::Result<Option<Visible>, Fault> {
        let area = self.windows().text_area(w, caller)?;
        if area.vscroll != 0 { return Err(unsupported("of a vertically scrolled displayed window")); }
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let (mut hosted, cx, mut it) = leaf.hosted_from(&area, top)?;
            let src = &mut hosted;
            let top_it = it;
            let last_y = area.body_height;
            if charpos < 0 {
                // The window end: the start of the row at the text area's
                // bottom, or the text's end above it.
                motion::move_to(src, &cx, &mut it, -1, -1, last_y - 1, -1, Op { y: true, ..Op::default() })?;
                let mut rest = it;
                motion::move_in_line(src, &cx, &mut rest, -1, -1, Op::default())?;
                let height = if rest.max_h.height() > 0 { rest.max_h.height() } else { src.default_height };
                let bottom = it.current_y + height;
                return Ok(Some(Visible { x: it.current_x - area.hscroll * area.column, y: area.header_line + it.current_y, rtop: 0,
                    rbot: (bottom - last_y).max(0), rowh: bottom.min(last_y) - it.current_y, vpos: it.vpos }));
            }
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
            let height = if rest.max_h.height() > 0 { rest.max_h.height() } else { src.default_height };
            let bottom = it.current_y + height;
            let (mut x, mut y) = (it.current_x, it.current_y);
            // Stopped at CHARPOS, at or past the first visible x, before its
            // row produced its line number.  pos_visible_p: the last width
            // produced, else the first row's.
            if let Some(lnum) = cx.lnum && it.lnum_x == 0 && it.current_x >= cx.first_visible_x && motion::charpos(src, &it)? == charpos {
                x += if it.lnum_last > 0 { it.lnum_last } else { lnum.prefix(src.line_at(top_it.index)?).1 };
            }
            if motion::loaded(src, &it)?.is_some_and(|e| matches!(e.method, motion::Method::Vector { .. })) {
                // Stopped on a display vector's last glyph (an ellipsis):
                // where the character before CHARPOS ends, past any vector
                // showing it.
                (x, y) = (0, it.current_y);
                if charpos >= 2 && top < charpos {
                    let mut before = top_it;
                    motion::move_to(src, &cx, &mut before, charpos - 1, -1, -1, -1, Op { pos: true, ..Op::default() })?;
                    let (end, eol) = if motion::charpos(src, &before)? != charpos - 1 { (before, false) }
                        else { motion::past_vector(src, &cx, &mut before, charpos)? };
                    if !eol && end.current_x <= cx.last_visible_x { (x, y) = (end.current_x, end.current_y); }
                }
            } else if motion::charpos(src, &it)? != charpos {
                // Past CHARPOS: it is hidden, or a display replaces it.
                if let Some((start, string)) = src.source().replacing_display(charpos)? {
                    (x, y) = display_start(src, &cx, top_it, top, charpos, start, string, it.lnum_x)?;
                }
            }
            // X counts from the text area's left edge, scrolled
            // (window_hscroll_limited); Y from the window's top: the text
            // starts below the header line (window_top_y).
            let x = x - area.hscroll * area.column;
            // pos_visible_p: the glyph's top, the row's ascent so far less its own below the row's.
            let y = y + it.max_h.drop(it.h);
            Ok(Some(Visible { x, y: area.header_line + y, rtop: 0, rbot: (bottom - last_y).max(0),
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

    /// The end of the ROWS screen lines WINDOW shows of its buffer from
    /// START over the host's layout: where its next screen line begins, or
    /// ZV (GNU's window_end_pos for a window of ROWS lines).
    pub(crate) fn hosted_rows_end(&mut self, window: Value, start: i64, rows: i64) -> std::result::Result<i64, Fault> {
        let area = self.windows().text_area(window, "redisplay")?;
        let buffer = area.buffer;
        self.with_buffer(buffer, |leaf| {
            let (mut hosted, cx, mut it) = leaf.hosted_from(&area, start)?;
            motion::move_to(&mut hosted, &cx, &mut it, -1, -1, -1, rows, Op { vpos: true, ..Op::default() })?;
            motion::charpos(&mut hosted, &it)
        })
    }

    /// buffer_posn_from_coords (dispnew.c:6260) at text-area pixel X, Y:
    /// the glyph there, from the window start TOP.
    fn hosted_posn(&mut self, area: &TextArea, top: i64, x: i64, y: i64) -> std::result::Result<Posn, Fault> {
        let (mut hosted, cx, mut it) = self.hosted_from(area, top)?;
        motion::move_to(&mut hosted, &cx, &mut it, -1, 0, y, -1, Op { x: true, y: true, ..Op::default() })?;
        // Iterator positions include the hscroll.
        let x = x + area.first_visible * area.column;
        motion::move_in_line(&mut hosted, &cx, &mut it, cx.zv, x, Op { x: true, ..Op::default() })?;
        let pos = motion::charpos(&mut hosted, &it)?;
        // The glyph under X, else the space glyph GNU appends after a
        // line's end and at ZV, in the default face.
        let glyph = if it.loaded { hosted.slot(it.index)?.elem } else { None };
        let (width, height) = match glyph.map(|e| (e, e.shape)) {
            Some((_, motion::Shape::Pixel { c, .. })) if c != 0x0a => hosted.glyph(it.index)?,
            Some((_, motion::Shape::PixelTab { space, .. })) => (motion::pixel_tab(&cx, &it, space), hosted.glyph(it.index)?.1),
            Some((_, motion::Shape::Image { width, ascent, descent, .. })) => (width, ascent + descent),
            Some((e, motion::Shape::PixelStretch { width, unnumbered, .. })) => {
                let width = if it.lnum_x > 0 { width } else { unnumbered.unwrap_or(width) };
                (motion::pixel_stretch(&cx, &it, &e, width), hosted.glyph(it.index)?.1)
            }
            _ => hosted.default_advance(' ')?,
        };
        let string = glyph.filter(|e| e.object != Value::NIL).and_then(|e| Some((e.object, e.strpos?)));
        let image = glyph.and_then(|e| if let motion::Shape::Image { at, .. } = e.shape { Some(hosted.source().image(at).1) } else { None });
        // DY from the glyph's top, which sits the row's ascent less its own
        // below the row's (pos_visible_p).
        let rise = it.max_h.drop(it.h);
        Ok(Posn { pos, col: it.hpos, row: it.vpos, dx: x - it.current_x, dy: y - it.current_y - rise, width, height, string, image })
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
        let b = match self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) {
            Some(Object::Buffer(b)) => b,
            // A unibyte buffer's bytes are its characters.
            Some(Object::ByteBuffer(b)) => {
                let r = b.restriction.start.0..b.restriction.end.0;
                return Ok((r.start + scan(&b.bytes[r]).min(b.restriction.end.0 - b.restriction.start.0)) as i64 + 1);
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        let r = b.accessible();
        let text = &b.as_str()[r.start.0..r.end.0];
        let mut at = scan(text.as_bytes());
        while !text.is_char_boundary(at) { at += 1; }
        Ok(b.char_of_byte(crate::coordinates::ByteOffset(r.start.0 + at)).map_err(Fault::Error)?.0 as i64 + 1)
    }

    /// The line numbers `display-line-numbers' shows in AREA's window, its
    /// buffer current, for an iterator whose first row shows FIRST
    /// (maybe_produce_line_number): the label's digits, the glyphs a row's
    /// number takes and their x, in pixels once a host lays the window out
    /// (the `line-number' face's), else columns.
    pub(crate) fn line_numbers(&mut self, area: &TextArea, first: i64, point: i64) -> std::result::Result<Option<LineLabels>, Fault> {
        let var = |leaf: &mut Self, name: &str| -> std::result::Result<Value, Fault> {
            let symbol = leaf.intern(name);
            Ok(leaf.variable_value(symbol)?.unwrap_or(Value::NIL))
        };
        let mode = var(self, "display-line-numbers")?;
        if mode == Value::NIL || self.vm.window(area.window).map_err(Fault::Error)?.mini { return Ok(None); }
        let (relative, visual) = (self.intern("relative"), self.intern("visual"));
        if mode == visual { return Err(unsupported("with visual line numbers")); }
        if var(self, "display-line-numbers-offset")?.integer_value().is_some_and(|n| n != 0)
            || var(self, "display-line-numbers-widen")? != Value::NIL {
            return Err(unsupported("with widened or offset line numbers"));
        }
        // The first row's line, past text its start hides (handle_stop).
        let this_line = self.shown_line(area, first)?;
        // The desired matrix's rows (required_matrix_height): the text
        // area's lines, from the frame's line height where GNU takes its
        // smallest font's, a partial one at either edge, and three bands.
        let line = i64::from(self.vm.window_state.line.max(1));
        let nrows = (area.body_height + area.vscroll + line - 1) / line + 2 + 3;
        let current_absolute = var(self, "display-line-numbers-current-absolute")? != Value::NIL;
        let max_lnum = if mode == relative && !current_absolute { nrows - 2 } else { this_line + nrows - 1 }.max(1);
        let mut width = max_lnum.ilog10() as i64 + 1;
        if let Some(least) = var(self, "display-line-numbers-width")?.integer_value().filter(|&n| n >= 0) {
            let limit = area.columns - area.left_margin - area.right_margin - 5 - 2;
            width = width.max(least.clamp(1, limit.max(1)));
        }
        let glyphs = width + 2;
        let labels = |x| LineLabels { width, x, relative: mode == relative, current_absolute, point_line: 0 };
        let point_line = self.line_index(point)?;
        // The text area less the truncation or continuation glyph and
        // three columns more must hold the number, else there is none.
        let room = |x: i64, column: i64, special: i64| x < area.body_width.min(area.columns * column) - special - 3 * column;
        if !area.hosted { return Ok(room(glyphs, 1, 1).then_some(LineLabels { point_line, ..labels(1) })); }
        if !self.vm.has_layout_host() {
            return Ok(room(glyphs * area.column, area.column, area.column).then_some(LineLabels { point_line, ..labels(area.column) }));
        }
        // Both faces' digits and spaces advance alike, in rows no taller
        // than the default's; an undefined face merges to the default.
        let window = Some(area.window);
        let (special, line_height) = self.vm.host_face_advance_in(window, "default", '\\').map_err(Fault::Error)?;
        let mut digit = None;
        for (face, c) in [("line-number", '0'), ("line-number", ' '), ("line-number-current-line", '0'), ("line-number-current-line", ' ')] {
            let (advance, height) = self.vm.host_face_advance_in(window, face, c).or_else(|_| self.vm.host_face_advance_in(window, "default", c)).map_err(Fault::Error)?;
            if *digit.get_or_insert(advance) != advance || height > line_height {
                return Err(unsupported("with a line-number face of its own widths or height"));
            }
        }
        let digit = i64::from(digit.unwrap_or(0));
        Ok(room(glyphs * digit, area.column, i64::from(special)).then_some(LineLabels { point_line, ..labels(digit) }))
    }
    /// The line of the first element AREA's window shows of the current
    /// buffer from POS: past text there it hides or replaces with nothing.
    /// Where the iterator refuses what it finds first (composed text), the
    /// row starts at POS.
    fn shown_line(&mut self, area: &TextArea, pos: i64) -> std::result::Result<i64, Fault> {
        let buffer = self.vm.slots[self.vm.buffer_slot].value;
        let (begv, zv) = self.accessible_positions(buffer)?;
        let pos = pos.clamp(begv, zv);
        match Source::new(self, Self::view(area, buffer, begv, zv, None), pos).and_then(|mut source| source.line_at(0)) {
            Err(Fault::Error(_)) => self.line_index(pos),
            line => line,
        }
    }
    /// BUFFER's accessible text's first and last positions.
    pub(crate) fn accessible_positions(&self, buffer: Value) -> std::result::Result<(i64, i64), Fault> {
        let Ok(b) = self.vm.shown_text(buffer) else { return Err(unsupported("in a unibyte buffer")) };
        let r = b.accessible();
        let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
        Ok((pos(r.start)?, pos(r.end)?))
    }
    /// The current buffer's point.
    fn point_position(&mut self) -> std::result::Result<i64, Fault> {
        match self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) {
            Some(Object::Buffer(b)) => Ok(b.char_of_byte(b.point).map_err(Fault::Error)?.0 as i64 + 1),
            Some(Object::ByteBuffer(b)) => Ok(b.point.0 as i64 + 1),
            _ => Err(unsupported("in a unibyte buffer")),
        }
    }
    /// The number of lines before POS's in the accessible text.
    pub(super) fn line_index(&mut self, pos: i64) -> std::result::Result<i64, Fault> {
        let Ok(b) = self.vm.shown_text(self.vm.slots[self.vm.buffer_slot].value) else {
            return Err(unsupported("in a unibyte buffer"));
        };
        let r = b.accessible();
        let at = b.byte_of_char(crate::coordinates::CharOffset((pos - 1).max(0) as usize)).map_err(Fault::Error)?.max(r.start).min(r.end);
        Ok((b.line_of_byte(at).map_err(Fault::Error)? - b.line_of_byte(r.start).map_err(Fault::Error)?) as i64)
    }

    /// How WINDOW labels its lines from its start, its buffer current.
    pub(crate) fn window_line_numbers(&mut self, window: Value) -> std::result::Result<Option<LineLabels>, Fault> {
        // Most windows show none: say so before measuring the window.
        let buffer = self.vm.window(window).map_err(Fault::Error)?.buffer;
        let mode = self.intern("display-line-numbers");
        if self.with_buffer(buffer, |leaf| leaf.variable_value(mode))?.is_none_or(|v| v == Value::NIL) { return Ok(None); }
        let area = self.windows().text_area(window, "redisplay")?;
        let top = self.windows().window_end(window)?.2;
        self.with_buffer(area.buffer, |leaf| {
            // Redisplay makes the window's point the buffer's.
            let point = leaf.vm.window_point_value(window).map_err(Fault::Error)?.as_integer().map_err(Fault::Error)?;
            leaf.line_numbers(&area, top, point)
        })
    }

    /// line-number-display-width (indent.c:2150) for the selected window:
    /// the label's digits, or with PIXELWISE `columns' the row prefix in
    /// frame columns, else in pixels.
    pub(crate) fn line_number_display_width(&mut self, pixelwise: Value) -> super::super::Out {
        let window = self.vm.selected_window_value().map_err(Fault::Error)?;
        let area = self.windows().text_area(window, "line-number-display-width")?;
        let (width, pixels) = self.line_number_widths(&area)?;
        // A terminal's column is a pixel.
        let column = if area.hosted { area.column.max(1) } else { 1 };
        let columns = self.intern("columns");
        if pixelwise == columns { return Ok(self.float(pixels as f64 / column as f64)); }
        Value::integer(if pixelwise != Value::NIL { pixels } else { width }).map_err(Fault::Error)
    }

    /// GNU's line_number_display_width (indent.c:2122) for AREA's window:
    /// its labels' digits and their row prefix in pixels, as the window
    /// start's line produces them.
    fn line_number_widths(&mut self, area: &TextArea) -> std::result::Result<(i64, i64), Fault> {
        let buffer = area.buffer;
        let top = self.windows().window_end(area.window)?.2;
        let numbers = self.with_buffer(buffer, |leaf| {
            let point = leaf.point_position()?;
            leaf.line_numbers(area, top, point)
        })?;
        Ok(match numbers {
            Some(labels) => (labels.width, labels.prefix(self.with_buffer(buffer, |leaf| leaf.line_index(top))?).1),
            None => (0, 0),
        })
    }

    /// init_iterator's line wrap and last visible x on a terminal frame,
    /// with line numbers for an iterator whose first row shows FIRST.
    fn iterator_context(&mut self, area: &TextArea, zv: i64, first: i64) -> std::result::Result<Ctx, Fault> {
        let var = |leaf: &mut Self, name: &str| -> std::result::Result<Value, Fault> {
            let symbol = leaf.intern(name);
            Ok(leaf.variable_value(symbol)?.unwrap_or(Value::NIL))
        };
        if var(self, "selective-display")? != Value::NIL { return Err(unsupported("with `selective-display'")); }
        let point = self.point_position()?;
        let lnum = self.line_numbers(area, first, point)?;
        // GNU's initial terminal frame counts line numbers in their width
        // but not in the iterator's x (no glyph row to copy them from).
        if lnum.is_some() && !area.hosted { return Err(unsupported("with line numbers")); }
        // A horizontally scrolled window truncates its lines (init_iterator).
        let wrap = if area.hscroll != 0 { Wrap::Truncate } else { self.vm.line_wrap(area.full_width, area.columns).map_err(Fault::Error)? };
        if wrap == Wrap::Word && var(self, "word-wrap-by-category")? != Value::NIL {
            return Err(unsupported("with `word-wrap-by-category'"));
        }
        let slot = if wrap == Wrap::Truncate { 0 } else { 1 };
        let glyph = self.special_glyph_width(slot)?;
        let border = i64::from(!area.rightmost);
        let zv_after_newline = match self.vm.object_if(self.vm.slots[self.vm.buffer_slot].value) {
            Some(Object::Buffer(b)) => b.as_str()[..b.accessible().end.0].ends_with('\n'),
            Some(Object::ByteBuffer(b)) => b.restriction.end.0 > 0 && b.bytes[b.restriction.end.0 - 1] == b'\n',
            _ => false,
        };
        // The host paints a display table's glyphs for characters and its
        // escape and control glyphs as the elements publish them, and its
        // truncation and continuation glyphs when they are characters
        // without a face.
        if area.hosted && let Some(table) = self.display_table()? && !self.host_line_glyphs(table) {
            return Err(unsupported("with a display table's faced truncation or continuation glyphs in a displayed window"));
        }
        Ok(Ctx {
            first_visible_x: area.first_visible,
            last_visible_x: area.first_visible + area.columns - area.left_margin - area.right_margin - glyph - border,
            wrap,
            tab_width: self.tab_width()? as i64,
            zv,
            zv_after_newline,
            lnum,
        })
    }

    /// Whether the display table TABLE's truncation and continuation glyphs
    /// (slots 0 and 1) are unset or characters without a face, which the
    /// host draws.
    fn host_line_glyphs(&mut self, table: Value) -> bool {
        self.vm.display_table_line_slots(table)
            .is_some_and(|slots| slots.iter().all(|&g| g == Value::NIL || self.vm.plain_glyph(g).is_some()))
    }

    /// The character the host shows for the truncation (SLOT 0) or
    /// continuation (SLOT 1) glyph: the display table's, else `$' or `\'.
    pub(super) fn host_line_glyph(&mut self, slot: usize) -> std::result::Result<char, Fault> {
        let glyph = self.special_glyph(slot)?;
        Ok(self.vm.plain_glyph(glyph).unwrap_or(if slot == 0 { '$' } else { '\\' }))
    }

    /// The truncation (SLOT 0) or continuation (SLOT 1) glyph's columns:
    /// the display table's glyph, else `$' or `\'.
    fn special_glyph_width(&mut self, slot: usize) -> std::result::Result<i64, Fault> {
        let glyph = self.special_glyph(slot)?;
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

    /// The display table's glyph in SLOT, nil without one.
    fn special_glyph(&mut self, slot: usize) -> std::result::Result<Value, Fault> {
        Ok(match self.display_table()? {
            Some(table) => match self.vm.object_if(table) {
                Some(Object::CharTable(t)) => t.extras.get(slot).copied().unwrap_or(Value::NIL),
                _ => Value::NIL,
            },
            None => Value::NIL,
        })
    }

    /// The tab, header and mode lines MODE-LINES asks for. VM2 draws no
    /// tab line (DIVERGENCES.md VM2-WIN3).
    fn mode_line_heights(&mut self, area: &TextArea, mode_lines: Value) -> std::result::Result<i64, Fault> {
        let (mode_line, header_line) = (self.intern("mode-line"), self.intern("header-line"));
        let all = mode_lines == Value::TRUE;
        Ok(if all || mode_lines == mode_line { area.mode_line } else { 0 }
            + if all || mode_lines == header_line { area.header_line } else { 0 })
    }
}

impl<'a> Leaf<'a> {
    /// The host-measured elements of AREA's window in its current buffer,
    /// their context, and the iterator at its start TOP, which may begin a
    /// continuation line (start_display).
    fn hosted_from(&mut self, area: &TextArea, top: i64) -> std::result::Result<(Hosted<'_, 'a>, Ctx, It), Fault> {
        let buffer = self.vm.slots[self.vm.buffer_slot].value;
        let (begv, zv) = match self.vm.shown_text(buffer).ok() {
            Some(b) => {
                let r = b.accessible();
                let pos = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(Fault::Error);
                (pos(r.start)?, pos(r.end)?)
            }
            _ => return Err(unsupported("in a unibyte buffer")),
        };
        let cx = self.iterator_context(area, zv, top)?;
        let view = Self::view(area, buffer, begv, zv, cx.lnum);
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
struct Posn { pos: i64, col: i64, row: i64, dx: i64, dy: i64, width: i64, height: i64, string: Option<(Value, usize)>,
    /// The image spec whose glyph is there.
    image: Option<Value> }

/// window_text_pixel_size's X-LIMIT and Y-LIMIT: the widest x (nil none,
/// anything but a natural number INT_MAX) and the tallest y (nil INT_MAX,
/// anything but a natural number 0).
fn limits(args: &TextSizeArgs) -> (Option<i64>, i64) {
    let max_x = match args.x_limit.integer_value() {
        Some(n) if (0..=INT_MAX).contains(&n) => Some(n),
        _ if args.x_limit != Value::NIL => Some(INT_MAX),
        _ => None,
    };
    let max_y = if args.y_limit == Value::NIL { INT_MAX } else { args.y_limit.integer_value().filter(|n| (0..=INT_MAX).contains(n)).unwrap_or(0) };
    (max_x, max_y)
}

/// init_iterator's last visible x on a window-system frame, in the
/// host's pixels: the text area, less the truncation or continuation
/// glyph's advance in the default face when there is no right fringe to
/// show its indicator in (xdisp.c:3510-3518). The host continues,
/// truncates and word-wraps rows as GNU does (romacs-layout2 `WordBreaks').
fn hosted_context(hosted: &mut Hosted<'_, '_>, mut cx: Ctx, area: &TextArea) -> std::result::Result<Ctx, Fault> {
    let glyph = hosted.source().leaf().host_line_glyph(usize::from(cx.wrap != Wrap::Truncate))?;
    let advance = if area.right_fringe > 0 { 0 } else { hosted.default_advance(glyph)?.0 };
    // Iterator x counts from the line's start, the hidden columns too.
    cx.first_visible_x = area.first_visible * area.column;
    cx.last_visible_x = cx.first_visible_x + area.body_width - advance;
    Ok(cx)
}

/// pos_visible_p's place for CHARPOS, which a display replacing the text
/// from START covers, STRING (or nil) showing there, from the window
/// start TOP at TOP_IT: as the cursor shows there, on the display line
/// where the display begins, at the first element at or after CHARPOS;
/// ROW_LNUM the line number CHARPOS's row produced.
#[allow(clippy::too_many_arguments)]
fn display_start(src: &mut Hosted<'_, '_>, cx: &Ctx, top_it: It, top: i64, charpos: i64, start: i64, string: Value, row_lnum: i64) -> std::result::Result<(i64, i64), Fault> {
    let mut it = top_it;
    if start > top { motion::move_to(src, cx, &mut it, start - 1, -1, -1, -1, Op { pos: true, ..Op::default() })?; }
    let (buffer, at) = motion::position(src, &it)?;
    let newline = buffer && (motion::loaded(src, &it)?.is_some_and(|e| matches!(e.shape, motion::Shape::Pixel { c: 10, .. }))
        || src.source().char_after(at)? == Some(0x0a));
    let next_line = newline || {
        let to_x = it.current_x + it.pixel_width;
        motion::move_in_line(src, cx, &mut it, -1, to_x, Op { x: true, ..Op::default() })? == motion::Move::LineContinued
    };
    if next_line { let vpos = it.vpos + 1; motion::move_to(src, cx, &mut it, -1, -1, -1, vpos, Op { vpos: true, ..Op::default() })?; }
    let top_y = it.current_y;
    let mut it = top_it;
    motion::move_to(src, cx, &mut it, -1, 0, top_y, -1, Op { x: true, y: true, ..Op::default() })?;
    let (mut x, stopped_in_string, mut before) = motion::x_at_or_after(src, cx, &mut it, charpos, string)?;
    // The walk produced no line number: add the row's own, as produced.
    if cx.lnum.is_some() && it.lnum_x == 0 { (x, before) = (x + row_lnum, before + row_lnum); }
    let newline_in_string = src.source().leaf().vm.str_view(string).is_some_and(|v| v.bytes().contains(&b'\n'));
    Ok((if newline_in_string && stopped_in_string { before } else { x }, top_y))
}

/// line_bottom_y from the start of the screen line showing POS: its
/// height, 0 for the empty line at the end of the buffer.
fn row_height<E: motion::Lines>(src: &mut E, cx: &Ctx, pos: i64) -> std::result::Result<i64, Fault> {
    let mut it = motion::lines_up(src, cx, pos, 0)?;
    motion::move_in_line(src, cx, &mut it, -1, -1, Op::default())?;
    Ok(it.max_h.height())
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
