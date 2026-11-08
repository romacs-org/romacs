//! Line-per-row point visibility. The default VM policy preserves GNU's
//! automatic centering and forced-start behavior. Positive
//! `scroll-conservatively` values reveal by minimum line movement within the
//! threshold; values above 100 always reveal minimally. The owned GUI chooses
//! 101 in Lisp, independently of the redisplay mechanism.
//!
//! Without a host layout, rows are logical lines of the frame's line
//! height. Over a host's layout, which continues and truncates lines, the
//! display iterator counts screen lines and chooses the start, which may
//! begin a continuation line (DIVERGENCES.md VM2-RD1).
//!
//! The rows a window shows are what its redisplay fontifies
//! ([`Vm::displayed_range`]), counted the same way.
use crate::{
    coordinates::{ByteOffset, CharOffset},
    data::Object,
    *,
};

/// The byte where the `rows` logical lines from `from` end: after their last
/// newline, or `zv`.
fn rows_end(text: &str, from: usize, rows: usize, zv: usize) -> usize {
    let Some(last) = rows.checked_sub(1) else { return from };
    text.as_bytes()[from..zv]
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == b'\n')
        .nth(last)
        .map_or(zv, |(i, _)| from + i + 1)
}

/// The bytes of TEXT a window starting at START can show in ROWS rows,
/// within ACCESSIBLE: from the start of START's line through the end of
/// the ROWS-th line from START. Every row is at least one frame line tall
/// and every line takes at least one row, so no shown byte lies outside.
pub(crate) fn shown_bytes(text: &str, accessible: std::ops::Range<usize>, start: usize, rows: usize) -> std::ops::Range<usize> {
    let start = start.clamp(accessible.start, accessible.end);
    let line = text.as_bytes()[accessible.start..start].iter().rposition(|&c| c == b'\n')
        .map_or(accessible.start, |i| accessible.start + i + 1);
    line..rows_end(text, start, rows, accessible.end)
}

impl Vm {
    /// Redisplay's one point-visibility step for live WINDOW, whose text
    /// area is TEXT_HEIGHT pixels: its start shows point on a whole row
    /// (`redisplay_point_visibility'), then its hscroll keeps point's
    /// cursor inside the text area (`hscroll_window'), both over the rows
    /// painting lays out at the window's own text area. A changed hscroll
    /// can change how lines wrap, so the start is checked again.
    pub(crate) fn keep_point_visible(&mut self, window: Value, text_height: u32) -> Result<()> {
        self.redisplay_point_visibility(window, text_height)?;
        if self.hscroll_window(window, None)? { self.redisplay_point_visibility(window, text_height)?; }
        Ok(())
    }

    /// GNU's hscroll_window_tree (xdisp.c:16625) for live WINDOW: with
    /// `auto-hscroll-mode' and no explicit scroll holding it, scroll so
    /// that point's cursor is in the text area outside `hscroll-margin'
    /// columns of either edge, by `hscroll-step' or, when that is 0, to put
    /// point mid-window (near the right edge at a line's end); never below
    /// the window's `min_hscroll'. Remembers point as the window's old
    /// point. True when the hscroll changed. PUBLISHED, the window's last
    /// published snapshot with point at that byte of its text, places
    /// point from its line (`point_line_snapshot') without building the
    /// window's text and faces again.
    pub(crate) fn hscroll_window(&mut self, window: Value, published: Option<(&romacs_layout_input::WindowSnapshot, usize)>) -> Result<bool> {
        let w = self.window(window)?;
        if w.mini || !w.live() { return Ok(false); }
        let (buffer, hscroll, min, old) = (w.buffer, w.hscroll, w.min_hscroll, w.old_point);
        let point = self.window_point_value(window)?.as_integer()?;
        // Moving point ends an explicit scroll's hold.
        let old = if old == Value::NIL { None } else { self.marker_position_value(old)?.integer_value() };
        if self.window(window)?.suspend_auto_hscroll && old != Some(point) { self.window_mut(window)?.suspend_auto_hscroll = false; }
        self.set_window_old_point(window, Value::integer(point)?, buffer)?;
        if self.window(window)?.suspend_auto_hscroll || self.buffer_variable(buffer, "auto-hscroll-mode")? == Value::NIL { return Ok(false); }
        // Only a truncated row needs scrolling: a window that continues its
        // lines and is not scrolled has none.
        if hscroll == 0 && self.window_line_wrap(window, buffer)? != romacs_layout_input::LineWrap::Truncate { return Ok(false); }
        let Ok(b) = self.shown_text(buffer) else { return Ok(false) };
        let accessible = b.accessible();
        let point_byte = b.byte_of_char(CharOffset((point.max(1) - 1) as usize))?.0.clamp(accessible.start.0, accessible.end.0);
        let line = published.and_then(|(snapshot, at)| point_line_snapshot(snapshot, at));
        let place = match line {
            Some(line) if self.has_layout_host() => self.lay_out_alone(line)?.point,
            _ => self.point_place(window, point_byte)?,
        };
        let Some(place) = place else { return Ok(false) };
        let current_line = self.hscrolls_current_line(window, buffer)?;
        let column = i64::from(self.window_state.column.max(1));
        let width = i64::from(self.window_text_width(window)?);
        let margin = self.buffer_variable(buffer, "hscroll-margin")?.integer_value().unwrap_or(0).clamp(0, 1_000_000) * column;
        let step = self.buffer_variable(buffer, "hscroll-step")?;
        let step = Step::of(step.integer_value(), match self.object(step) { Ok(Object::Float(f)) => Some(*f), _ => None });
        let (x, offset, line_x) = (place.x.round() as i64, place.gutter.round() as i64, place.line_x.round() as i64);
        let scroll = (hscroll != 0 && x <= margin + offset)
            || (place.truncated_right && x >= width - margin)
            || (current_line && hscroll != min && !place.truncated_left);
        if !scroll { return Ok(false); }
        let wanted = match step {
            Step::Columns(0) => if place.at_eol { width - 4 * column } else { width / 2 },
            _ if x >= width - margin => match step {
                Step::Fraction(f) => (width as f64 * (1.0 - f)) as i64 - margin,
                Step::Columns(n) => width - n * column - margin,
            },
            Step::Fraction(f) => (width as f64 * f) as i64 + margin + offset,
            Step::Columns(n) => n * column + margin + offset,
        };
        let new = ((line_x - wanted).max(0) / column).max(min);
        // Point's row in `current-line' mode can need the same amount
        // re-applied on another line: the snapshot places it by point.
        if new == hscroll { return Ok(false); }
        self.window_mut(window)?.hscroll = new;
        self.display_state.version += 1;
        Ok(true)
    }

    /// Text-area rows of the window with `text_height` pixels: (fully
    /// visible, displayed including a partial last row).
    pub(crate) fn window_rows(&self, text_height: u32) -> (usize, usize) {
        let line = self.window_state.line.max(1);
        ((text_height / line) as usize, text_height.div_ceil(line) as usize)
    }

    /// Keep point fully visible in live, non-minibuffer `window`, whose text
    /// area is `text_height` pixels (the window minus its mode line).
    pub(crate) fn redisplay_point_visibility(&mut self, window: Value, text_height: u32) -> Result<()> {
        let conservative = if let Some(&slot) = self.names.get("scroll-conservatively") {
            self.variable_value(slot)?.integer_value().unwrap_or(0).max(0) as usize
        } else {
            0
        };
        let w = self.window(window)?;
        if w.mini || !w.live() {
            return Ok(());
        }
        let (buffer, start_marker, mut forced) = (w.buffer, w.start, w.force_start);
        // GNU clears the force as it honours it (`xdisp.c:20747`).
        self.window_mut(window)?.force_start = false;
        // Over a host's layout the forced start stands with point moved
        // onto its rows, unless point is on a partly shown row or in a
        // scroll margin, which scroll as an unforced window does.
        if forced && self.display_state.hosted && self.has_layout_host() {
            match self.honour_forced_start(window)? {
                Some(true) => return Ok(()),
                Some(false) => forced = false,
                None => {}
            }
        }
        let line = self.window_state.line.max(1);
        let (rows, displayed) = self.window_rows(text_height);
        if rows == 0 {
            return Ok(());
        }
        self.mirror_shown(buffer)?;
        let point = self.window_point_value(window)?.as_integer()? as usize;
        let start = self.marker_position_value(start_marker)?.integer_value().unwrap_or(1) as usize;
        let Ok(b) = self.shown_text(buffer) else {
            return Ok(());
        };
        let accessible = b.accessible();
        let (begv, zv) = (accessible.start.0, accessible.end.0);
        let text = b.as_str();
        let start_byte = b.byte_of_char(CharOffset(start.saturating_sub(1)))?.0.clamp(begv, zv);
        let point_byte = b.byte_of_char(CharOffset(point.saturating_sub(1)))?.0.clamp(begv, zv);
        // Row of point counted from the start, or None when point is above it.
        let row = (point_byte >= start_byte).then(|| {
            text.as_bytes()[start_byte..point_byte].iter().filter(|&&c| c == b'\n').take(displayed).count()
        });
        let clamped = b.char_of_byte(coordinates::ByteOffset(start_byte))?.0 + 1;
        // Over a host's layout, which continues lines, rows are screen
        // lines: the rows painting lays out. Point on a fully visible row
        // keeps the start (GNU's try_cursor_movement and try_window); else
        // those rows around point choose another (try_scrolling, recenter).
        // Before a window's first paint the display iterator chooses,
        // unless it refuses the text.
        if !forced && self.display_state.hosted && self.has_layout_host() {
            if self.point_on_visible_rows(window, start_byte, point_byte)? {
                if clamped != start { self.set_marker_object(start_marker, Value::integer(clamped as i64)?, buffer)?; }
                return Ok(());
            }
            let Ok(b) = self.shown_text(buffer) else { return Ok(()) };
            let text = b.as_str();
            let point_line = text[begv..point_byte].rfind('\n').map_or(begv, |newline| begv + newline + 1);
            // Point below the start but in the top scroll margin scrolls
            // the window down, as point above it does (try_scrolling).
            let bounds = self.margin_bounds(window)?;
            let below = point_byte >= start_byte && bounds.is_none_or(|(top, _)| point_byte >= top);
            let margin = self.window_layout(window)?.map_or(Ok(0.0), |laid| self.scroll_margin_px(window, laid.body_height))?;
            // Within a threshold the scroll is measured in the screen
            // lines painting lays out, which invisible text and continued
            // lines make differ from buffer lines; the rows that measure
            // it below the start choose the new start too.
            let (distance, measured) = match (1..=100).contains(&conservative) {
                true => self.scroll_distance(window, (start_byte, point_byte), rows, conservative, below)?,
                false => (Some(0), None),
            };
            let distance = match distance {
                Some(distance) => distance,
                None => {
                    let Ok(b) = self.shown_text(buffer) else { return Ok(()) };
                    let text = b.as_str().as_bytes();
                    match below {
                        false => text[point_line.min(start_byte)..start_byte].iter().filter(|&&c| c == b'\n').count(),
                        true => text[start_byte..point_byte].iter().filter(|&&c| c == b'\n').count().saturating_sub(rows - 1),
                    }
                }
            };
            let distance = distance + (margin / line as f32) as usize;
            let minimal = conservative > 100 || (conservative > 0 && distance <= conservative);
            let back = recenter_rows(text_height, line);
            if let Some(chosen) = self.start_from_rows(window, point_byte, measured, (minimal, below, conservative > 100))? {
                let Ok(b) = self.shown_text(buffer) else { return Ok(()) };
                let position = Value::integer((b.char_of_byte(ByteOffset(chosen))?.0 + 1) as i64)?;
                return self.set_marker_object(start_marker, position, buffer).map(drop);
            }
            if let Ok(chosen) = crate::prim::hosted_window_start(self, window, clamped as i64, point as i64, minimal, i64::from(back)) {
                let position = Value::integer(chosen.unwrap_or(clamped as i64))?;
                if chosen.is_some() || clamped != start { self.set_marker_object(start_marker, position, buffer)?; }
                return Ok(());
            }
        }
        let Ok(b) = self.shown_text(buffer) else {
            return Ok(());
        };
        let text = b.as_str();
        if row.is_some_and(|row| row < rows) {
            if clamped != start {
                self.set_marker_object(start_marker, Value::integer(clamped as i64)?, buffer)?;
            }
            return Ok(());
        }
        if forced && row.is_none_or(|row| row >= displayed) {
            // Point is not displayed at all: keep the start, move point to the
            // first row whose bottom reaches half the window height, never past
            // the row holding ZV (`xdisp.c:20824-20894`).
            let half = text_height / 2;
            let target = half.div_ceil(line).saturating_sub(1) as usize;
            let mut at = start_byte;
            for _ in 0..target {
                match text[at..zv].find('\n') {
                    Some(newline) if at + newline + 1 <= zv => at += newline + 1,
                    _ => break,
                }
            }
            let position = Value::integer((b.char_of_byte(coordinates::ByteOffset(at))?.0 + 1) as i64)?;
            if clamped != start {
                self.set_marker_object(start_marker, Value::integer(clamped as i64)?, buffer)?;
            }
            return self.set_window_point_position(window, position);
        }
        let point_line = text[begv..point_byte].rfind('\n').map_or(begv, |newline| begv + newline + 1);
        if conservative > 0 {
            let distance = if conservative > 100 {
                0
            } else if point_byte < start_byte {
                text.as_bytes()[point_line..start_byte].iter().filter(|&&c| c == b'\n').count()
            } else {
                text.as_bytes()[start_byte..point_byte].iter().filter(|&&c| c == b'\n').count() - rows + 1
            };
            if conservative > 100 || distance <= conservative {
                let mut at = point_line;
                if point_byte >= start_byte {
                    for _ in 1..rows {
                        if at <= begv { break; }
                        at = text[begv..at - 1].rfind('\n').map_or(begv, |newline| begv + newline + 1);
                    }
                }
                let position = Value::integer((b.char_of_byte(coordinates::ByteOffset(at))?.0 + 1) as i64)?;
                return self.set_marker_object(start_marker, position, buffer).map(drop);
            }
        }
        // Recenter (`xdisp.c:21230`, `move_it_vertically_backward`):
        // dy = window_box_height / 2; back max(1, dy / line) lines from point,
        // one more when the remainder exceeds min(height, 2 * line / 3).
        let back = recenter_rows(text_height, line);
        let (mut at, mut moved) = (point_line, 0);
        while moved < back && at > begv {
            at = text[begv..at - 1].rfind('\n').map_or(begv, |newline| begv + newline + 1);
            moved += 1;
        }
        // GNU `xdisp.c:21249`: moving back can land a whole window above
        // point (a one-row window); then start at point's own line.
        if moved * line >= text_height {
            at = point_line;
        }
        let position = Value::integer((b.char_of_byte(coordinates::ByteOffset(at))?.0 + 1) as i64)?;
        self.set_marker_object(start_marker, position, buffer).map(drop)
    }

    /// redisplay_window's forced start (xdisp.c:20747-20900) for WINDOW
    /// over the rows painting lays out from it: Some(true) when the start
    /// stands, with point moved, if no whole row shows it, to the start of
    /// the row reaching half the text area (the last row, should the text
    /// end above it); Some(false) when point is on a partly shown row or in
    /// a scroll margin, where GNU scrolls instead; None without a layout.
    fn honour_forced_start(&mut self, window: Value) -> Result<Option<bool>> {
        let Some((end, shown)) = self.lay_out_window(window)? else { return Ok(None) };
        let body = end.body_height;
        let rows: Vec<_> = end.rows.iter().filter(|r| r.y < body).copied().collect();
        let Some(first) = rows.first() else { return Ok(None) };
        let point = self.window_point_value(window)?.as_integer()?;
        let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok(None) };
        let point_byte = b.byte_of_char(CharOffset((point.max(1) - 1) as usize))?.0;
        let shown_end = shown.buffer_byte(end.end);
        let reached_end = shown.at_zv && end.end >= shown.text.utf8.len();
        let on = |r: &&romacs_layout_input::Row| shown.buffer_byte(r.start) <= point_byte;
        let at = (point_byte >= shown.buffer_byte(first.start) && (point_byte < shown_end || reached_end))
            .then(|| rows.iter().rposition(|r| on(&r))).flatten();
        let new_vpos = match at {
            None => body / 2.0,
            Some(i) if rows[i].y + rows[i].height > body => return Ok(Some(false)),
            Some(i) => {
                let margin = self.scroll_margin_px(window, body)?;
                let lines = (margin / self.window_state.line.max(1) as f32) as usize;
                if i < lines || rows[i].y >= body - margin { return Ok(Some(false)); }
                return Ok(Some(true));
            }
        };
        let row = rows.iter().find(|r| r.y + r.height >= new_vpos).unwrap_or(&rows[rows.len() - 1]);
        let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok(None) };
        let position = Value::integer((b.char_of_byte(ByteOffset(shown.buffer_byte(row.start)))?.0 + 1) as i64)?;
        self.set_window_point_position(window, position)?;
        Ok(Some(true))
    }

    /// The character positions `window`, showing the current buffer, spans
    /// with a `text_height`-pixel text area: from its start through the end
    /// of its last displayed row, counting the rows layout paints from
    /// -vscroll. Over a host's layout the rows are screen lines, as the
    /// display iterator continues and truncates them; where it refuses the
    /// text, logical lines, which never show less. None when it shows no
    /// text.
    pub(crate) fn displayed_range(&mut self, window: Value, text_height: u32) -> Result<Option<(usize, usize)>> {
        self.mirror_shown(self.slots[self.buffer_slot].value)?;
        let w = self.window(window)?;
        let (rows, start) = (self.window_rows(text_height.saturating_add(w.vscroll_px)).1, w.start);
        let start = self.marker_position_value(start)?.integer_value().unwrap_or(1).max(1) as usize;
        let painted = if self.display_state.hosted { self.window_layout(window)? } else { None };
        let hosted = match painted {
            Some(_) => None,
            None => (self.display_state.hosted && self.has_layout_host())
                .then(|| crate::prim::hosted_rows_end(self, window, start as i64, rows as i64).ok())
                .flatten(),
        };
        let Ok(b) = self.shown_text(self.slots[self.buffer_slot].value) else {
            return Ok(None);
        };
        let accessible = b.accessible();
        let from = b.byte_of_char(CharOffset(start - 1))?.0.clamp(accessible.start.0, accessible.end.0);
        let start = b.char_of_byte(ByteOffset(from))?.0 + 1;
        let end = match (painted, hosted) {
            (Some(laid), _) => b.char_of_byte(ByteOffset(laid.end.min(accessible.end.0)))?.0 + 1,
            (None, Some(end)) => end as usize,
            (None, None) => b.char_of_byte(ByteOffset(rows_end(b.as_str(), from, rows, accessible.end.0)))?.0 + 1,
        };
        Ok((start < end).then_some((start, end)))
    }

    /// POINT_BYTE is on a fully visible row of WINDOW from START_BYTE,
    /// outside its scroll margins (`margin_bounds'), which a vertically
    /// scrolled window, whose first row may be partly hidden, never
    /// answers.
    pub(crate) fn point_on_visible_rows(&mut self, window: Value, start_byte: usize, point_byte: usize) -> Result<bool> {
        if self.window(window)?.vscroll_px != 0 { return Ok(false); }
        if point_byte < start_byte || !self.window_layout(window)?.is_some_and(|laid| point_byte < laid.visible_end) { return Ok(false); }
        Ok(self.margin_bounds(window)?.is_none_or(|(top, bottom)| (top..bottom).contains(&point_byte)))
    }

    /// The start byte that shows POINT_BYTE, not on a whole row of WINDOW
    /// outside its scroll margins, from the rows painting lays out around
    /// it: with MINIMAL, point just below the top margin, else, BELOW,
    /// just above the bottom one (GNU's try_scrolling); otherwise point
    /// recentred (`recenter_row'). MEASURED, rows `rows_above' laid out
    /// already, serve when they fill the text area above point. Scrolling
    /// down moves the start by pixels, to a row's first shown byte, unless
    /// BY_LINES (`scroll-conservatively' over 100); other motions go back
    /// by lines. None where `rows_between' answers none.
    fn start_from_rows(&mut self, window: Value, point_byte: usize, measured: Option<Vec<(usize, f32, usize)>>, (minimal, below, by_lines): (bool, bool, bool)) -> Result<Option<usize>> {
        if self.window(window)?.vscroll_px != 0 { return Ok(None); }
        let Some(laid) = self.window_layout(window)? else { return Ok(None) };
        let margin = self.scroll_margin_px(window, laid.body_height)?;
        let begv = { let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok(None) }; b.accessible().start.0 };
        let rows = match measured.filter(|rows| rows.first().is_some_and(|r| r.0 <= begv) || above_point(rows, point_byte).0 >= laid.body_height) {
            Some(rows) => rows,
            None => match self.rows_above(window, point_byte, (laid.body_height, 0), begv)? { Some(rows) => rows, None => return Ok(None) },
        };
        if !minimal {
            // Each row and whether it starts a line, as the recentring's
            // motion back by lines needs.
            let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok(None) };
            let text = b.as_str().as_bytes();
            let rows: Vec<_> = rows.iter().map(|&(start, height, _)| (start, height, start <= begv || text[start - 1] == b'\n')).collect();
            let line = self.window_state.line.max(1) as f32;
            let dy = (laid.body_height / 2.0).floor();
            return Ok(rows.iter().rposition(|r| r.0 <= point_byte).map(|at| rows[recenter_row(&rows, at, point_byte, (laid.body_height, dy), line)].0));
        }
        if by_lines && below && let Some(start) = self.conservative_start(window, &rows, point_byte, (laid.body_height, margin))? {
            return Ok(Some(start));
        }
        Ok(Some(match start_for_point(&rows, point_byte, (laid.body_height, margin), below).map(|top| rows[top]) {
            Some((start, _, shown)) => if below && !by_lines { shown } else { start },
            None => point_byte,
        }))
    }

    /// try_scrolling's new start over `scroll-conservatively' 100 from
    /// ROWS, or from the rows through point when they stop short of
    /// WINDOW's start (`conservative_row').
    fn conservative_start(&mut self, window: Value, rows: &[(usize, f32, usize)], point_byte: usize, (body, margin): (f32, f32)) -> Result<Option<usize>> {
        let start = self.marker_position_value(self.window(window)?.start)?.integer_value().unwrap_or(1);
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let start = b.byte_of_char(CharOffset((start - 1).max(0) as usize))?.0;
        let accessible = b.accessible();
        let line_start = crate::prim::line_beginning(b.body(), accessible.start.0, accessible.end.0, start, 0);
        let laid;
        let rows = match rows.first().is_some_and(|r| r.0 <= start) {
            true => rows,
            false => match self.rows_between(window, line_start, point_byte)? { Some(rows) => { laid = rows; &laid[..] } None => return Ok(None) },
        };
        let (Some(from), Some(at)) = (rows.iter().rposition(|r| r.0 <= start), rows.iter().rposition(|r| r.0 <= point_byte)) else { return Ok(None) };
        let line = self.window_state.line.max(1) as f32;
        if rows.iter().all(|r| r.1 == line) { return Ok(None); }
        Ok(conservative_row(&rows[from..=at], body, margin, line).map(|k| rows[from + k].0))
    }

    /// Each of ROWS with whether it starts a line (`recenter_row').
    fn rows_with_line_starts(&self, window: Value, rows: &[(usize, f32, usize)], begv: usize) -> Result<Vec<(usize, f32, bool)>> {
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Err("buffer required".into()) };
        let text = b.as_str().as_bytes();
        Ok(rows.iter().map(|&(start, height, _)| (start, height, start <= begv || text[start - 1] == b'\n')).collect())
    }

    /// The rows painting lays out in WINDOW from FROM's line until they
    /// reach REACH pixels past the row showing FROM, or ZV: those rows and
    /// the index of FROM's.
    fn rows_down(&mut self, window: Value, from: usize, reach: f32) -> Result<Option<(Vec<(usize, f32, usize)>, usize)>> {
        let mut lines = (reach / self.window_state.line.max(1) as f32).ceil() as i64 + 2;
        loop {
            let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
            let accessible = b.accessible();
            let (begv, zv) = (accessible.start.0, accessible.end.0);
            let (start, to) = (crate::prim::line_beginning(b.body(), begv, zv, from, 0), crate::prim::line_beginning(b.body(), begv, zv, from, lines));
            let Some(rows) = self.rows_between(window, start, to)? else { return Ok(None) };
            let Some(at) = rows.iter().rposition(|r| r.0 <= from) else { return Ok(None) };
            if to >= zv || rows[at..].iter().map(|r| r.1).sum::<f32>() >= reach { return Ok(Some((rows, at))); }
            lines = lines.saturating_mul(2);
        }
    }

    /// Frecenter on a window a host lays out, for its pixel branches: ARG
    /// none centres point, a negative ARG puts it that many rows from the
    /// bottom (MARGIN and HT its scroll margin and height in lines). The
    /// new start's character position; None where lines serve (no layout)
    /// or, as GNU, when nothing fits.
    pub(crate) fn recenter_start(&mut self, window: Value, arg: Option<i64>, (margin, ht): (i64, i64)) -> Result<Option<i64>> {
        if self.window(window)?.vscroll_px != 0 { return Ok(None); }
        let Some(laid) = self.window_layout(window)? else { return Ok(None) };
        let (body, line) = (laid.body_height, self.window_state.line.max(1) as f32);
        let point = self.window_point_value(window)?.as_integer()?;
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let (begv, point) = (b.accessible().start.0, b.byte_of_char(CharOffset((point - 1).max(0) as usize))?.0);
        let Some(rows) = self.rows_above(window, point, (body, 0), begv)? else { return Ok(None) };
        let rows = self.rows_with_line_starts(window, &rows, begv)?;
        let Some(at) = rows.iter().rposition(|r| r.0 <= point) else { return Ok(None) };
        // Rows a line tall make pixels lines: the iterator's motion serves.
        let uniform = rows.iter().all(|r| r.1 == line);
        let top = match arg {
            None if uniform => return Ok(None),
            None => recenter_row(&rows, at, point, (body, (body / 2.0).floor()), line),
            Some(arg) => {
                let lines = (-arg).clamp(margin + 1, (ht - margin).max(margin + 1)) as usize;
                let Some((below, from)) = self.rows_down(window, point, lines as f32 * line * 2.0)? else { return Ok(None) };
                if uniform && below[from..].iter().take(lines + 1).all(|r| r.1 == line) { return Ok(None); }
                let height = recenter_height(&below[from..], lines, body, line);
                if height <= 0.0 { return Ok(None); }
                let mut top = recenter_row(&rows, at, point, (body, height), line);
                while top < at && rows[top..at].iter().map(|r| r.1).sum::<f32>() > height { top += 1; }
                top
            }
        };
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        Ok(Some(b.char_of_byte(ByteOffset(rows[top].0))?.0 as i64 + 1))
    }

    /// window_scroll_pixel_based's new start for a whole screen from
    /// character position START, down when FORWARD: `next-screen-context-lines'
    /// CONTEXT lines stay. None without a layout.
    pub(crate) fn scroll_start(&mut self, window: Value, start: i64, forward: bool, context: i64) -> Result<Option<i64>> {
        if self.window(window)?.vscroll_px != 0 { return Ok(None); }
        let Some(laid) = self.window_layout(window)? else { return Ok(None) };
        let (body, line) = (laid.body_height, self.window_state.line.max(1) as f32);
        let dy = line.max(((body / line).floor() - context as f32) * line);
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let accessible = b.accessible();
        let (begv, zv, from) = (accessible.start.0, accessible.end.0, b.byte_of_char(CharOffset((start - 1).max(0) as usize))?.0);
        let byte = if forward {
            let Some((rows, at)) = self.rows_down(window, from, dy)? else { return Ok(None) };
            if rows.iter().all(|r| r.1 == line) { return Ok(None); }
            let (mut i, mut y) = (at, 0.0);
            while i < rows.len() && y + rows[i].1 <= dy { y += rows[i].1; i += 1; }
            match rows.get(i.max(at + 1)) { Some(row) => row.2, None => zv }
        } else {
            let Some(rows) = self.rows_above(window, from, (dy, 0), begv)? else { return Ok(None) };
            if rows.iter().all(|r| r.1 == line) { return Ok(None); }
            let rows = self.rows_with_line_starts(window, &rows, begv)?;
            let Some(at) = rows.iter().rposition(|r| r.0 <= from) else { return Ok(None) };
            let top = recenter_row(&rows, at, from, (body, dy), line);
            rows[if rows[top].0 >= from { top.saturating_sub(1) } else { top }].0
        };
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        Ok(Some(b.char_of_byte(ByteOffset(byte))?.0 as i64 + 1))
    }

    /// The whole rows WINDOW's text area shows from character position
    /// START. None without a layout.
    pub(crate) fn rows_shown_from(&mut self, window: Value, start: i64) -> Result<Option<i64>> {
        let Some(laid) = self.window_layout(window)? else { return Ok(None) };
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let from = b.byte_of_char(CharOffset((start - 1).max(0) as usize))?.0;
        let Some((rows, at)) = self.rows_down(window, from, laid.body_height)? else { return Ok(None) };
        let (mut y, mut count) = (0.0, 0);
        for row in &rows[at..] {
            if y + row.1 > laid.body_height { break; }
            y += row.1;
            count += 1;
        }
        Ok(Some(count))
    }

    /// The rows painting lays out in WINDOW from a line above POINT_BYTE's
    /// through point's: from as few lines back as give the rows above
    /// point's REACH pixels and COUNT rows, else from STOP's line. A row
    /// can show many buffer lines (invisible text, a display property) or
    /// a line many rows, so the lines double until the rows suffice: one
    /// layout when the first guess, a row a line, holds. None as for
    /// `rows_between'.
    fn rows_above(&mut self, window: Value, point_byte: usize, (reach, count): (f32, usize), stop: usize) -> Result<Option<Vec<(usize, f32, usize)>>> {
        // Every row is at least a frame line tall: this many lines above
        // point's fill REACH when each shows on one row.
        let floor = { let table = self.window_faces(window)?; self.row_floor_of(&table) };
        let mut lines = ((reach / floor as f32).ceil() as i64).max(count as i64) + 1;
        loop {
            let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok(None) };
            let accessible = b.accessible();
            let (begv, zv) = (accessible.start.0, accessible.end.0);
            let stop = crate::prim::line_beginning(b.body(), begv, zv, stop.clamp(begv, point_byte), 0);
            let from = crate::prim::line_beginning(b.body(), begv, zv, point_byte, -lines).max(stop);
            let Some(rows) = self.rows_between(window, from, point_byte)? else { return Ok(None) };
            let (height, above) = above_point(&rows, point_byte);
            if from == stop || (height >= reach && above >= count) { return Ok(Some(rows)); }
            lines = lines.saturating_mul(2);
        }
    }

    /// The screen lines WINDOW, showing its buffer from START_BYTE in
    /// ROWS whole rows, scrolls to show POINT_BYTE (try_scrolling's
    /// distance): BELOW, from its last whole row to point's; else from
    /// point's row to the start's, none from the top margin. Any distance past LIMIT is LIMIT + 1.
    /// Below, the rows that measured it, from the start's line or LIMIT
    /// rows above the window. None without a layout.
    fn scroll_distance(&mut self, window: Value, (start_byte, point_byte): (usize, usize), rows: usize, limit: usize, below: bool)
        -> Result<(Option<usize>, Option<Vec<(usize, f32, usize)>>)> {
        if below {
            let Some(laid) = self.rows_above(window, point_byte, (0.0, rows + limit + 1), start_byte)? else { return Ok((None, None)) };
            let at = above_point(&laid, point_byte).1;
            let distance = match laid.first().is_some_and(|r| r.0 <= start_byte) {
                true => (at - laid.iter().rposition(|r| r.0 <= start_byte).unwrap_or(0)).saturating_sub(rows - 1),
                false => limit + 1,
            };
            return Ok((Some(distance.min(limit + 1)), Some(laid)));
        }
        // Point in the top margin, below the start.
        if point_byte >= start_byte { return Ok((Some(0), None)); }
        let mut lines = limit as i64 + 1;
        loop {
            let Ok(b) = self.shown_text(self.window(window)?.buffer) else { return Ok((None, None)) };
            let accessible = b.accessible();
            let (begv, zv) = (accessible.start.0, accessible.end.0);
            let (line, start_line) = (crate::prim::line_beginning(b.body(), begv, zv, point_byte, 0), crate::prim::line_beginning(b.body(), begv, zv, start_byte, 0));
            let to = crate::prim::line_beginning(b.body(), begv, zv, point_byte, lines).min(start_line);
            let Some(laid) = self.rows_between(window, line, to)? else { return Ok((None, None)) };
            let at = above_point(&laid, point_byte).1;
            if to == start_line {
                let start = laid.iter().rposition(|r| r.0 <= start_byte).unwrap_or(0);
                return Ok((Some(start.saturating_sub(at).min(limit + 1)), None));
            }
            if laid.len() - at > limit { return Ok((Some(limit + 1), None)); }
            lines = lines.saturating_mul(2);
        }
    }

    fn set_window_point_position(&mut self, window: Value, position: Value) -> Result<()> {
        if window == self.selected_window_value()? {
            self.set_buffer_point_for_window(window, position)
        } else {
            let (marker, buffer) = (self.window(window)?.point, self.window(window)?.buffer);
            self.set_marker_object(marker, position, buffer).map(drop)
        }
    }
}

/// The height of the rows of ROWS (start byte, height, first shown byte;
/// top to bottom) above POINT's, and how many there are.
fn above_point(rows: &[(usize, f32, usize)], point: usize) -> (f32, usize) {
    let at = rows.iter().rposition(|r| r.0 <= point).unwrap_or(0);
    (rows[..at].iter().map(|r| r.1).sum(), at)
}

/// The row of ROWS (each a start byte, height and first shown byte, top
/// to bottom) that shows POINT where try_scrolling scrolls just enough: when
/// BELOW, on the last whole row above the bottom MARGIN of a BODY-pixel
/// text area, as many rows above it as fit; else on the first row below
/// the top margin. None when no row shows POINT.
fn start_for_point(rows: &[(usize, f32, usize)], point: usize, (body, margin): (f32, f32), below: bool) -> Option<usize> {
    let at = rows.iter().rposition(|&(start, ..)| start <= point)?;
    let top = match below {
        true => {
            let (mut top, mut height) = (at, rows[at].1 + margin);
            while top > 0 && height + rows[top - 1].1 <= body {
                top -= 1;
                height += rows[top].1;
            }
            top
        }
        false => {
            let (mut top, mut height) = (at, 0.0);
            while top > 0 && height < margin {
                top -= 1;
                height += rows[top].1;
            }
            top
        }
    };
    Some(top)
}

/// The row of ROWS (start byte, height, whether it starts a line; top to
/// bottom) redisplay's recentring starts the window at, POINT being on row
/// AT of a BODY-pixel text area whose lines are LINE pixels (xdisp.c:21150):
/// move_it_vertically_backward from point by BACK pixels, half the text
/// area there, which
/// goes back by whole lines, then further back when short of that by more
/// than two thirds of a row, or forward row by row when past it; point's
/// own row when that lands a whole window above it.
fn recenter_row(rows: &[(usize, f32, bool)], at: usize, point: usize, (body, back): (f32, f32), line: f32) -> usize {
    let target = body - back;
    let line_start = |i: usize| (0..=i).rev().find(|&j| rows[j].2).unwrap_or(0);
    let above = |i: usize| rows[i..at].iter().map(|r| r.1).sum::<f32>();
    // back_to_previous_visible_line_start from point: its line's start,
    // the line before when point starts a line.
    let (mut i, mut from_point, mut dy) = (at, true, back);
    loop {
        for _ in 0..((dy / line) as usize).max(1) {
            let back = if from_point && !(rows[i].2 && rows[i].0 == point) { Some(i) } else { i.checked_sub(1) };
            from_point = false;
            match back { Some(j) => i = line_start(j), None => break }
            if i == 0 { break; }
        }
        let y = body - above(i);
        let height = rows.get(i + 1).unwrap_or(&rows[i]).1;
        if target < y && y - target > body.min(height * 2.0 / 3.0) && i > 0 {
            dy = y - target;
            continue;
        }
        if target >= y + height {
            // Forward row by row while short of the target, back one past it.
            let mut y = y;
            while i < at && target > y {
                y += rows[i].1;
                i += 1;
            }
            if y > target { i -= 1; }
        }
        return if body - above(i) <= 0.0 { at } else { i };
    }
}

/// The row of ROWS, the window's from its start through point's, that
/// starts it after try_scrolling over `scroll-conservatively' 100 scrolled
/// point in: point's row must end where the row holding the bottom scroll
/// margin of a BODY-pixel text area ends, so the amount is that distance,
/// a LINE at least, and the rows below the first that scroll off reach it
/// (line_bottom_y from row to row). Point on a partly shown last row asks
/// a line more of margin (`last_line_misfit'), as does a start that leaves
/// point's row partly shown (`too_near_end'). None when no row holds the
/// margin.
fn conservative_row(rows: &[(usize, f32, usize)], body: f32, margin: f32, line: f32) -> Option<usize> {
    let total: f32 = rows.iter().map(|r| r.1).sum();
    let point_top = total - rows.last()?.1;
    let (mut y, mut partial) = (0.0, 0.0);
    for row in rows {
        if y < body && y + row.1 > body { partial = body - y; }
        y += row.1;
    }
    let mut extra = f32::from(u8::from(point_top < body && total > body));
    loop {
        let limit = body - partial - margin - line * extra;
        let (mut top, mut bottom) = (0.0, None);
        for row in rows {
            if top <= limit - 1.0 { bottom = Some(top + row.1); }
            top += row.1;
        }
        let amount = (top - bottom?).max(line);
        let (mut moved, mut k) = (0.0, 0);
        while k + 1 < rows.len() && moved < amount {
            k += 1;
            moved += rows[k].1;
        }
        if k + 1 == rows.len() || rows[k..].iter().map(|r| r.1).sum::<f32>() <= body { return Some(k); }
        extra += 1.0;
    }
}

/// How far back Frecenter moves for a negative ARG, LINES rows from the
/// bottom of a BODY-pixel text area: all of it less the rows from point's
/// (the first of ROWS) down LINES rows, and past ZV a LINE for each row
/// short.
fn recenter_height(rows: &[(usize, f32, usize)], lines: usize, body: f32, line: f32) -> f32 {
    if rows.len() > lines { return body - rows[..lines].iter().map(|r| r.1).sum::<f32>(); }
    let short = lines.saturating_sub(rows.len());
    body - rows.iter().map(|r| r.1).sum::<f32>() - short as f32 * line
}

/// The lines redisplay's recentring moves back from point's (`xdisp.c:21230`,
/// `move_it_vertically_backward`): dy = window_box_height / 2; back
/// max(1, dy / line) lines, one more when the remainder exceeds
/// min(height, 2 * line / 3).
fn recenter_rows(text_height: u32, line: u32) -> u32 {
    let dy = text_height / 2;
    let mut back = (dy / line).max(1);
    if dy > back * line && dy - back * line > text_height.min(line * 2 / 3) {
        back += 1;
    }
    back
}

/// `hscroll-step' as hscroll_window_tree reads it: a non-negative float is
/// a fraction of the text area, a non-negative fixnum columns, anything
/// else 0.
#[derive(Clone, Copy)]
enum Step { Columns(i64), Fraction(f64) }

impl Step {
    fn of(integer: Option<i64>, float: Option<f64>) -> Self {
        match (integer, float) {
            (Some(n), _) => Step::Columns(n.clamp(0, i64::from(i32::MAX))),
            (_, Some(f)) if f >= 0.0 => Step::Fraction(f),
            _ => Step::Columns(0),
        }
    }
}

/// Point's line of PUBLISHED, a window's snapshot with point at byte
/// POINT of its text, as a snapshot of its own: the line's text, runs, raw
/// bytes and stretches from its start, and only the faces they use, the
/// default first, so that laying it out costs the line rather than the
/// face table. None for a snapshot whose rows number their lines.
fn point_line_snapshot(published: &romacs_layout_input::WindowSnapshot, point: usize) -> Option<romacs_layout_input::WindowSnapshot> {
    use romacs_layout_input::{FaceHandle, FaceRun, InlineImage, Stretch, TextSlice, WindowChrome, WindowSnapshot};
    let text = &published.text.utf8;
    if point > text.len() || published.line_numbers.is_some() || !text.is_char_boundary(point) { return None; }
    let from = text[..point].rfind('\n').map_or(0, |at| at + 1);
    let to = text[point..].find('\n').map_or(text.len(), |at| point + at + 1);
    let (mut used, mut index) = (vec![0u32], std::collections::HashMap::from([(0u32, 0u32)]));
    let mut handle = |face: FaceHandle| FaceHandle(*index.entry(face.0).or_insert_with(|| {
        used.push(face.0);
        used.len() as u32 - 1
    }));
    let face_runs = published.text.face_runs.iter().filter(|r| r.bytes.end > from && r.bytes.start < to)
        .map(|r| FaceRun { bytes: r.bytes.start.max(from) - from..r.bytes.end.min(to) - from, face: handle(r.face) }).collect();
    let raw = published.text.raw.iter().filter(|&&b| (from..to).contains(&(b as usize))).map(|&b| b - from as u32).collect();
    let stretches = published.stretches.iter().filter(|s| (from..to).contains(&s.byte)).map(|s| Stretch { byte: s.byte - from, width: s.width }).collect();
    let images = published.images.iter().filter(|i| (from..to).contains(&i.byte)).map(|i| InlineImage { byte: i.byte - from, ..*i }).collect();
    let wrap_flips = published.wrap_flips.iter().filter(|&&b| (from..to).contains(&(b as usize))).map(|&b| b - from as u32).collect();
    let faces = used.iter().map(|&f| published.faces.get(f as usize).cloned()).collect::<Option<Vec<_>>>()?;
    Some(WindowSnapshot { underline_rules: Default::default(), band_stretches: Default::default(),
        id: published.id, width: published.width, height: 1 << 24, start_byte: 0, point_byte: point - from, buffer_version: 0,
        text: TextSlice { extends: vec![], utf8: text[from..to].into(), face_runs, raw },
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: published.hscroll, faces, tab_width: published.tab_width, wrap: published.wrap,
        cursor_off: false, cursor: Default::default(), stretches, images, wrap_flips, line_numbers: None,
        // Only the fringes' widths place the row; their bitmaps' faces are not kept.
        chrome: WindowChrome { left_fringe: published.chrome.left_fringe, right_fringe: published.chrome.right_fringe,
            left_margin: published.chrome.left_margin, right_margin: published.chrome.right_margin, ..WindowChrome::default() },
    })
}
