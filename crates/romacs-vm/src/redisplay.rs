//! Line-per-row point visibility. The default VM policy preserves GNU's
//! automatic centering and forced-start behavior. Positive
//! `scroll-conservatively` values reveal by minimum line movement within the
//! threshold; values above 100 always reveal minimally. The owned GUI chooses
//! 101 in Lisp, independently of the redisplay mechanism.
//!
//! Rows are logical lines of the frame's line height: `romacs-layout2` paints
//! one logical line per row and does not wrap (DIVERGENCES.md VM2-RD1).
//!
//! The rows a window shows are what its redisplay fontifies
//! ([`Vm::displayed_range`]).
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
        let dy = text_height / 2;
        let mut back = (dy / line).max(1);
        if dy > back * line && dy - back * line > text_height.min(line * 2 / 3) {
            back += 1;
        }
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
    /// -vscroll. None when it shows no text.
    pub(crate) fn displayed_range(&self, window: Value, text_height: u32) -> Result<Option<(usize, usize)>> {
        let w = self.window(window)?;
        let (rows, start) = (self.window_rows(text_height.saturating_add(w.vscroll_px)).1, w.start);
        let start = self.marker_position_value(start)?.integer_value().unwrap_or(1).max(1) as usize;
        let Object::Buffer(b) = self.object(self.slots[self.buffer_slot].value)? else {
            return Ok(None);
        };
        let accessible = b.accessible();
        let from = b.byte_of_char(CharOffset(start - 1))?.0.clamp(accessible.start.0, accessible.end.0);
        let to = rows_end(b.as_str(), from, rows, accessible.end.0);
        let (start, end) = (b.char_of_byte(ByteOffset(from))?.0 + 1, b.char_of_byte(ByteOffset(to))?.0 + 1);
        Ok((start < end).then_some((start, end)))
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
