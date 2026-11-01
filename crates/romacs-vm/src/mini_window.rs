//! The mini-window's display (xdisp.c): its buffer's text with the overlay
//! strings the display iterator delivers at each position
//! (load_overlay_strings), and the height that display takes
//! (resize_mini_window). Completion UIs such as icomplete show their
//! candidates this way. Other windows do not show overlay strings yet:
//! their point visibility and scrolling (redisplay.rs) count buffer lines,
//! not display lines.
use crate::{data::Object, overlay_order::OverlayEntry, *};
use romacs_layout_input::FaceRun;
use std::ops::Range;

/// A run of display text: `len` bytes of buffer text from buffer byte
/// `buffer`, or, when `string`, an overlay string shown at buffer byte `buffer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    pub display: usize,
    pub buffer: usize,
    pub len: usize,
    pub string: bool,
}
romacs_snapshot::snapshot_struct!(Segment { display, buffer, len, string });

/// A buffer's text with its overlay strings spliced in.
#[derive(Default)]
pub(super) struct Spliced {
    pub text: String,
    /// The `face' property of each run of string text, in display bytes.
    pub string_faces: Vec<(Range<usize>, Value)>,
    pub segments: Vec<Segment>,
    /// (buffer byte, display byte): the first string character shown at
    /// that buffer byte whose `cursor' property is non-nil.
    pub cursors: Vec<(usize, usize)>,
}

impl Spliced {
    pub fn has_strings(&self) -> bool {
        self.segments.iter().any(|s| s.string)
    }
    /// Where BUFFER's text shows, after the strings at BUFFER.
    pub fn after_strings(&self, buffer: usize) -> usize {
        display_of(&self.segments, buffer, self.text.len())
    }
    /// Where point at BUFFER shows: on a string character there that asks
    /// for the cursor, else after the strings (set_cursor_from_row).
    pub fn cursor(&self, buffer: usize) -> usize {
        self.cursors.iter().find(|&&(b, _)| b == buffer).map_or_else(|| self.after_strings(buffer), |&(_, d)| d)
    }
    /// Where a display starting at BUFFER begins: before the strings there.
    pub fn before_strings(&self, buffer: usize) -> usize {
        self.segments.iter().find(|s| s.string && s.buffer == buffer)
            .map_or_else(|| self.after_strings(buffer), |s| s.display)
    }
}

/// The display byte of BUFFER's own text in SEGMENTS (END past the text).
fn display_of(segments: &[Segment], buffer: usize, end: usize) -> usize {
    segments.iter().find(|s| !s.string && s.buffer <= buffer && buffer < s.buffer + s.len)
        .map_or(end, |s| s.display + buffer - s.buffer)
}

/// The buffer byte shown at display byte DISPLAY: a string's own position
/// for its characters, BUFFER_END past the text.
pub(crate) fn buffer_of(segments: &[Segment], display: usize, buffer_end: usize) -> usize {
    segments.iter().find(|s| s.display <= display && display < s.display + s.len)
        .map_or(buffer_end, |s| if s.string { s.buffer } else { s.buffer + display - s.display })
}

/// The display's face runs: RUNS over buffer bytes moved to where SEGMENTS
/// show that text, and STRINGS, already in display bytes, between them.
pub(super) fn display_face_runs(runs: &[FaceRun], segments: &[Segment], strings: Vec<FaceRun>) -> Vec<FaceRun> {
    let mut out: Vec<FaceRun> = strings;
    for s in segments.iter().filter(|s| !s.string) {
        let end = s.buffer + s.len;
        for run in runs.iter().filter(|r| r.bytes.start < end && s.buffer < r.bytes.end) {
            let (from, to) = (run.bytes.start.max(s.buffer), run.bytes.end.min(end));
            out.push(FaceRun { bytes: s.display + from - s.buffer..s.display + to - s.buffer, face: run.face });
        }
    }
    out.sort_by_key(|r| r.bytes.start);
    let mut merged: Vec<FaceRun> = vec![];
    for run in out {
        match merged.last_mut() {
            Some(last) if last.face == run.face && last.bytes.end == run.bytes.start => last.bytes.end = run.bytes.end,
            _ => merged.push(run),
        }
    }
    merged
}

/// Where each display row of TEXT starts, a byte offset: every text line
/// starts one, and when WRAPPED a glyph that does not fit in what is left
/// of a row of WIDTH columns starts the next. A TAB goes to the next tab
/// stop of its line, a control character takes two (^X), others their
/// Unicode width.
fn row_starts(text: &str, width: usize, wrapped: bool) -> Vec<usize> {
    let mut starts = vec![0];
    let (mut at, mut column) = (0, 0);
    for (byte, c) in text.char_indices() {
        if c == '\n' {
            starts.push(byte + 1);
            (at, column) = (0, 0);
            continue;
        }
        let glyph = match c {
            '\t' => 8 - column % 8,
            '\u{0}'..='\u{1f}' | '\u{7f}' => 2,
            _ => unicode_width::UnicodeWidthChar::width(c).unwrap_or(1),
        };
        if wrapped && at > 0 && at + glyph > width { starts.push(byte); at = 0; }
        (at, column) = (at + glyph, column + glyph);
    }
    starts
}

/// How a string character displays with `ctl-arrow' at COLUMN: a tab as
/// spaces to the next tab stop, other controls as ^X, a raw byte as \OOO.
fn displayed(c: u32, column: usize, out: &mut String) {
    match c {
        0x0a => out.push('\n'),
        0x09 => out.extend(std::iter::repeat_n(' ', 8 - column % 8)),
        0x00..=0x1f | 0x7f => { out.push('^'); out.push(char::from_u32(c ^ 0x40).unwrap()); }
        0x3fff80..=0x3fffff => out.push_str(&format!("\\{:o}", c - 0x3fff00)),
        _ => out.push(char::from_u32(c).unwrap_or('\u{fffd}')),
    }
}

impl Vm {
    /// BUFFER's TEXT as WINDOW displays it, with the before- and
    /// after-strings of the overlays that apply to WINDOW.
    pub(super) fn splice_overlay_strings(&mut self, window: Value, buffer: Value, text: &str) -> Result<Spliced> {
        let mut out = Spliced::default();
        let mut copied = 0;
        // TEXT ends at ZV: an overlay past it is outside the accessible region.
        for (at, strings) in self.overlay_strings(window, buffer)?.into_iter().filter(|(at, _)| *at <= text.len()) {
            if at > copied {
                out.segments.push(Segment { display: out.text.len(), buffer: copied, len: at - copied, string: false });
                out.text.push_str(&text[copied..at]);
            }
            copied = at;
            for string in strings {
                let start = out.text.len();
                self.splice_string(string, at, &mut out)?;
                out.segments.push(Segment { display: start, buffer: at, len: out.text.len() - start, string: true });
            }
        }
        if text.len() > copied {
            out.segments.push(Segment { display: out.text.len(), buffer: copied, len: text.len() - copied, string: false });
            out.text.push_str(&text[copied..]);
        }
        Ok(out)
    }

    fn splice_string(&mut self, string: Value, at: usize, out: &mut Spliced) -> Result<()> {
        let (face, cursor) = (self.display_symbol("face"), self.display_symbol("cursor"));
        let chars: Vec<u32> = self.str_view(string).ok_or("overlay string required")?.chars().collect();
        for (i, c) in chars.into_iter().enumerate() {
            let start = out.text.len();
            let column = out.text[..start].rfind('\n').map_or(start, |n| start - n - 1);
            displayed(c, column, &mut out.text);
            let value = self.property_textget(string, i as i64, face).map_err(crate::forwarded::fault_text)?;
            match out.string_faces.last_mut() {
                Some((run, last)) if run.end == start && *last == value => run.end = out.text.len(),
                _ => out.string_faces.push((start..out.text.len(), value)),
            }
            if !out.cursors.iter().any(|&(b, _)| b == at)
                && self.property_textget(string, i as i64, cursor).map_err(crate::forwarded::fault_text)? != Value::NIL
            {
                out.cursors.push((at, start));
            }
        }
        Ok(())
    }

    /// Each buffer byte where overlay strings show in WINDOW, in order,
    /// with its strings in load_overlay_strings' order: the after-strings
    /// of overlays ending there and the before-strings of those starting there.
    fn overlay_strings(&mut self, window: Value, buffer: Value) -> Result<Vec<(usize, Vec<Value>)>> {
        let (before, after, priority) =
            (self.display_symbol("before-string"), self.display_symbol("after-string"), self.display_symbol("priority"));
        let Object::Buffer(b) = self.object(buffer)? else { return Err("text buffer required".into()) };
        let end = b.len_bytes();
        let mut overlays = vec![];
        self.visit_overlays_in(buffer, coordinates::ByteOffset(0)..coordinates::ByteOffset(end), |overlay, _| overlays.push(overlay))?;
        let mut shown = vec![];
        for overlay in overlays {
            if !self.overlay_applies_to(overlay, window) { continue; }
            let Some((start, end)) = self.overlay_range(overlay)? else { continue };
            let priority = self.overlay_property(overlay, priority)?.integer_value().unwrap_or(0);
            for (key, at, is_after) in [(before, start, false), (after, end, true)] {
                let string = self.overlay_property(overlay, key)?;
                if self.str_view(string).is_some_and(|s| s.chars > 0) {
                    shown.push((at as usize, OverlayEntry { overlay, string, priority, after: is_after, start: i64::from(start) }));
                }
            }
        }
        // By position, then in itree order: decreasing start.
        shown.sort_by_key(|&(at, entry)| (at, std::cmp::Reverse(entry.start)));
        let mut out: Vec<(usize, Vec<OverlayEntry>)> = vec![];
        for (at, entry) in shown {
            match out.last_mut() {
                Some((last, entries)) if *last == at => entries.push(entry),
                _ => out.push((at, vec![entry])),
            }
        }
        Ok(out.into_iter().map(|(at, entries)| (at, overlay_order::display_order(entries))).collect())
    }

    /// xdisp.c resize_mini_window: give the mini-window the lines its
    /// display needs (each line's rows as its window wraps it), within
    /// `max-mini-window-height', as
    /// `resize-mini-windows' allows: nil never resizes; `grow-only' grows
    /// the active minibuffer and otherwise fits the echo area exactly; t
    /// always fits exactly. Display starts at the beginning unless
    /// `redisplay-adhoc-scroll-in-resize-mini-windows' is nil; when the
    /// text does not fit, that variable shows its end instead. True when
    /// the mini-window's height changed.
    pub(crate) fn resize_mini_window(&mut self, mini: Value) -> Result<bool> {
        let value = |vm: &mut Self, name: &str| { let slot = vm.slot(name); vm.variable_value(slot) };
        let mode = value(self, "resize-mini-windows")?;
        if value(self, "inhibit-redisplay")? != Value::NIL { return Ok(false); }
        let adhoc = value(self, "redisplay-adhoc-scroll-in-resize-mini-windows")? != Value::NIL;
        let (buffer, marker, width) = { let w = self.window(mini)?; (w.buffer, w.start, w.width) };
        let Object::Buffer(b) = self.object(buffer)? else { return Err("text buffer required".into()) };
        let (text, empty) = (b.as_str().to_owned(), b.accessible().is_empty());
        let begv = b.char_of_byte(b.accessible().start)?.0 as i64 + 1;
        if adhoc { self.set_window_start_if_moved(marker, buffer, begv)?; }
        if mode == Value::NIL || mode == Value::UNBOUND { return Ok(false); }
        let spliced = self.splice_overlay_strings(mini, buffer, &text)?;
        let unit = self.window_state.line.max(1);
        let total = self.window_state.height;
        let limit = value(self, "max-mini-window-height")?;
        let max = match self.object_if(limit) {
            Some(Object::Float(f)) => (f * f64::from(total)) as i64,
            _ => limit.integer_value().map_or(i64::from(total / 4), |n| n.saturating_mul(i64::from(unit))),
        }.clamp(i64::from(unit), i64::from(total.max(unit)));
        let max_lines = (max / i64::from(unit)) as usize;
        // A continued row ends a column short, for the `\\' glyph (layout2).
        let column = self.window_state.column.max(1);
        let row = (width.saturating_sub(column) / column).max(1) as usize;
        let wrapped = self.window_line_wrap(mini, buffer)? != romacs_layout_input::LineWrap::Truncate;
        let starts = row_starts(&spliced.text, row, wrapped);
        let lines = starts.len();
        if lines > max_lines && adhoc {
            // Start at the screen row that shows the last MAX_LINES, a
            // continuation row of a text line too (xdisp.c resize_mini_window).
            let byte = buffer_of(&spliced.segments, starts[lines - max_lines], text.len());
            let Object::Buffer(b) = self.object(buffer)? else { return Err("text buffer required".into()) };
            let start = b.char_of_byte(coordinates::ByteOffset(byte))?.0 as i64 + 1;
            self.set_window_start_if_moved(marker, buffer, start)?;
        }
        let height = lines.min(max_lines).max(1) as u32;
        let old = self.window_state.mini_lines;
        // grow-only's shrink to one line, then the next cycle's growth
        // (window.c shrink_mini_window), in one step.
        let exact = mode != self.display_symbol("grow-only") || empty || !self.minibuffer_active();
        if height > old || (exact && height != old) {
            self.window_state.mini_lines = height;
            return Ok(true);
        }
        Ok(false)
    }

    fn set_window_start_if_moved(&mut self, marker: Value, buffer: Value, start: i64) -> Result<()> {
        if self.marker_position_value(marker)?.integer_value() != Some(start) {
            self.set_marker_object(marker, Value::integer(start)?, buffer)?;
        }
        Ok(())
    }

    /// Whether a minibuffer is active (minibuf.c minibuf_level).
    fn minibuffer_active(&mut self) -> bool {
        let slot = self.slot("romacs--minibuf-level");
        self.variable_value(slot).ok().and_then(|v| v.integer_value()).is_some_and(|level| level > 0)
    }
}
