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
        let (buffer, start_marker, forced) = (w.buffer, w.start, w.force_start);
        // GNU clears the force as it honours it (`xdisp.c:20747`).
        self.window_mut(window)?.force_start = false;
        let line = self.window_state.line.max(1);
        let (rows, displayed) = self.window_rows(text_height);
        if rows == 0 {
            return Ok(());
        }
        let point = self.window_point_value(window)?.as_integer()? as usize;
        let start = self.marker_position_value(start_marker)?.integer_value().unwrap_or(1) as usize;
        let Object::Buffer(b) = self.object(buffer)? else {
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
            let Object::Buffer(b) = self.object(buffer)? else { return Ok(()) };
            let text = b.as_str();
            let point_line = text[begv..point_byte].rfind('\n').map_or(begv, |newline| begv + newline + 1);
            let distance = if point_byte < start_byte {
                text.as_bytes()[point_line..start_byte].iter().filter(|&&c| c == b'\n').count()
            } else {
                text.as_bytes()[start_byte..point_byte].iter().filter(|&&c| c == b'\n').count().saturating_sub(rows - 1)
            };
            let minimal = conservative > 100 || (conservative > 0 && distance <= conservative);
            let back = recenter_rows(text_height, line);
            if let Some(chosen) = self.start_from_rows(window, start_byte, point_byte, minimal, back as usize)? {
                let Object::Buffer(b) = self.object(buffer)? else { return Ok(()) };
                let position = Value::integer((b.char_of_byte(ByteOffset(chosen))?.0 + 1) as i64)?;
                return self.set_marker_object(start_marker, position, buffer).map(drop);
            }
            if let Ok(chosen) = crate::prim::hosted_window_start(self, window, clamped as i64, point as i64, minimal, i64::from(back)) {
                let position = Value::integer(chosen.unwrap_or(clamped as i64))?;
                if chosen.is_some() || clamped != start { self.set_marker_object(start_marker, position, buffer)?; }
                return Ok(());
            }
        }
        let Object::Buffer(b) = self.object(buffer)? else {
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

    /// The character positions `window`, showing the current buffer, spans
    /// with a `text_height`-pixel text area: from its start through the end
    /// of its last displayed row, counting the rows layout paints from
    /// -vscroll. Over a host's layout the rows are screen lines, as the
    /// display iterator continues and truncates them; where it refuses the
    /// text, logical lines, which never show less. None when it shows no
    /// text.
    pub(crate) fn displayed_range(&mut self, window: Value, text_height: u32) -> Result<Option<(usize, usize)>> {
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
        let Object::Buffer(b) = self.object(self.slots[self.buffer_slot].value)? else {
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
    /// which a vertically scrolled window, whose first row may be partly
    /// hidden, never answers.
    fn point_on_visible_rows(&mut self, window: Value, start_byte: usize, point_byte: usize) -> Result<bool> {
        if self.window(window)?.vscroll_px != 0 { return Ok(false); }
        Ok(point_byte >= start_byte && self.window_layout(window)?.is_some_and(|laid| point_byte < laid.visible_end))
    }

    /// The start byte that shows POINT_BYTE, not on a whole row of WINDOW
    /// from START_BYTE, from the rows painting lays out around it: with
    /// MINIMAL, point on the first whole row when above the start, else on
    /// the last (GNU's try_scrolling); otherwise point's row BACK rows down
    /// (recenter). None where `rows_between' answers none.
    fn start_from_rows(&mut self, window: Value, start_byte: usize, point_byte: usize, minimal: bool, back: usize) -> Result<Option<usize>> {
        if self.window(window)?.vscroll_px != 0 { return Ok(None); }
        let Some(laid) = self.window_layout(window)? else { return Ok(None) };
        // Every row is at least a frame line tall: this many lines above
        // point's fill the text area.
        let lines = (laid.body_height / self.window_state.line.max(1) as f32).ceil() as i64 + 1;
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let accessible = b.accessible();
        let from = crate::prim::line_beginning(b.as_str().as_bytes(), accessible.start.0, accessible.end.0, point_byte, -lines);
        let Some(rows) = self.rows_between(window, from, point_byte)? else { return Ok(None) };
        Ok(Some(start_for_point(&rows, point_byte, laid.body_height, (minimal, point_byte >= start_byte), back)))
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

/// The start of the row of ROWS (each a start byte and height, top to
/// bottom) that shows POINT where redisplay puts it: with MINIMAL and
/// BELOW, on the last whole row of a BODY-pixel text area, as many rows
/// above it as fit; with MINIMAL alone, on the first; else BACK rows down.
fn start_for_point(rows: &[(usize, f32)], point: usize, body: f32, (minimal, below): (bool, bool), back: usize) -> usize {
    let Some(at) = rows.iter().rposition(|&(start, _)| start <= point) else { return point };
    let top = match (minimal, below) {
        (true, true) => {
            let (mut top, mut height) = (at, rows[at].1);
            while top > 0 && height + rows[top - 1].1 <= body {
                top -= 1;
                height += rows[top].1;
            }
            top
        }
        (true, false) => at,
        (false, _) => at.saturating_sub(back),
    };
    rows[top].0
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
