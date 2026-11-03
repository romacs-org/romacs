use crate::{Value, Vm};

/// Terminal defaults follow the editor theme: the default colours and the
/// sixteen base colours (`ansi-color-*' faces) a session has not set
/// itself; a colour the session set, and the 256-colour cube and greys,
/// remain literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalColor { Foreground, Background, DimForeground, Ansi(u8), DimAnsi(u8), Rgb([u8; 3]) }
romacs_snapshot::snapshot_enum!(TerminalColor {
    TerminalColor::Foreground => (),
    TerminalColor::Background => (),
    TerminalColor::DimForeground => (),
    TerminalColor::Ansi(f0) => (f0),
    TerminalColor::DimAnsi(f0) => (f0),
    TerminalColor::Rgb(f0) => (f0),
});

/// The sixteen base colours where no theme gives them: Alacritty's.
pub const DEFAULT_ANSI: [[u8; 3]; 16] = [
    [24, 24, 24], [172, 66, 66], [144, 169, 89], [244, 191, 117],
    [106, 159, 181], [170, 117, 159], [117, 181, 170], [216, 216, 216],
    [107, 107, 107], [197, 85, 85], [170, 194, 110], [254, 202, 136],
    [130, 184, 200], [194, 140, 184], [147, 211, 195], [248, 248, 248],
];

/// A colour at the 0.66 a dim cell draws in, Alacritty's factor.
fn dim(rgb: [u8; 3]) -> [u8; 3] { rgb.map(|v| (f32::from(v) * 0.66) as u8) }

/// An immutable cell colour projected without creating global Lisp faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TerminalStyle {
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub inverse: bool,
    pub hidden: bool,
}
romacs_snapshot::snapshot_struct!(TerminalStyle { foreground, background, inverse, hidden });

impl Default for TerminalStyle {
    fn default() -> Self {
        Self { foreground: TerminalColor::Foreground, background: TerminalColor::Background, inverse: false, hidden: false }
    }
}

impl TerminalStyle {
    /// The face this style draws in over DEFAULT, the sixteen base colours
    /// ANSI resolved from the theme.
    pub(crate) fn face(self, default: &romacs_layout_input::Face, ansi: &[[u8; 3]; 16]) -> romacs_layout_input::Face {
        let color = |value| match value {
            TerminalColor::Foreground => default.foreground,
            TerminalColor::Background => default.background,
            TerminalColor::DimForeground => dim(default.foreground),
            TerminalColor::Ansi(n) => ansi[usize::from(n) % 16],
            TerminalColor::DimAnsi(n) => dim(ansi[usize::from(n) % 16]),
            TerminalColor::Rgb(rgb) => rgb,
        };
        let (mut foreground, mut background) = (color(self.foreground), color(self.background));
        if self.inverse { std::mem::swap(&mut foreground, &mut background); }
        if self.hidden { foreground = background; }
        romacs_layout_input::Face { foreground, background, ..default.clone() }
    }
}

/// A half-open UTF-8 byte range in the terminal's logical text projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalStyleRun { pub bytes: std::ops::Range<usize>, pub style: TerminalStyle }
romacs_snapshot::snapshot_struct!(TerminalStyleRun { bytes, style });

/// An owned terminal publication; no evaluator values cross the PTY boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub revision: u64,
    pub text: std::sync::Arc<str>,
    pub styles: std::sync::Arc<[TerminalStyleRun]>,
    pub default_colors: [Option<[u8; 3]>; 2],
    pub cursor_byte: usize,
    pub screen_start_byte: usize,
    pub columns: u16,
    pub rows: u16,
    pub alternate_screen: bool,
    pub running: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

/// Small protocol state for input routing; independent of transcript materialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalInputState {
    pub running: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

impl TerminalSnapshot {
    /// Read protocol flags from an immutable publication.
    pub fn input_state(&self) -> TerminalInputState {
        TerminalInputState { running: self.running, application_cursor: self.application_cursor, bracketed_paste: self.bracketed_paste }
    }
}

/// Per-evaluator native terminal service. Calls must not evaluate Lisp.
pub trait TerminalHost {
    fn create(&mut self, shell: &str, directory: &str, columns: u16, rows: u16) -> Result<u64, String>;
    fn send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String>;
    fn resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String>;
    fn input_state(&mut self, session: u64) -> Result<TerminalInputState, String>;
    /// Drain identities whose native state changed since the last publication pass.
    fn changed_sessions(&mut self) -> Vec<u64>;
    fn snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String>;
    fn close(&mut self, session: u64) -> Result<(), String>;
}

pub(crate) struct TerminalBinding {
    pub(crate) buffer: Value,
    pub(crate) session: u64,
    pub(crate) revision: Option<u64>,
    pub(crate) follow: bool,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) styles: std::sync::Arc<[TerminalStyleRun]>,
    pub(crate) default_colors: [Option<[u8; 3]>; 2],
    /// The last publication's cursor and first grid row, in buffer bytes.
    pub(crate) cursor_byte: usize,
    pub(crate) screen_start_byte: usize,
}
romacs_snapshot::snapshot_struct!(TerminalBinding { buffer, session, revision, follow, columns, rows, styles, default_colors, cursor_byte, screen_start_byte });

impl TerminalBinding {
    pub(crate) fn new(buffer: Value, session: u64, columns: u16, rows: u16) -> Self {
        Self { buffer, session, revision: None, follow: true, columns, rows, styles: std::sync::Arc::from([]),
               default_colors: [None; 2], cursor_byte: 0, screen_start_byte: 0 }
    }
}

/// The first byte of the row that shows CURSOR on the last of ROWS rows
/// when the grid from SCREEN is taller than ROWS, else SCREEN.
fn viewport_start(text: &str, screen: usize, cursor: usize, rows: usize) -> usize {
    let breaks: Vec<usize> = text[screen..cursor].match_indices('\n').map(|(i, _)| screen + i + 1).collect();
    if breaks.len() < rows { screen } else { breaks[breaks.len() - rows] }
}

impl Vm {
    /// Attach the terminal service before creating terminal buffers.
    pub fn set_terminal_host(&mut self, host: Option<Box<dyn TerminalHost>>) -> Result<(), String> {
        if !self.terminal_bindings.is_empty() {
            return Err("Terminal sessions must be closed before replacing their host".into());
        }
        if host.is_some() {
            self.declare_terminal_errors()?;
        }
        self.terminal_host = host;
        Ok(())
    }

    /// The error a terminal command signals without a running terminal,
    /// which attaching the terminal service declares. A world builder, which
    /// attaches none, declares it where a live launch attaches one.
    pub fn declare_terminal_errors(&mut self) -> Result<(), String> {
        self.eval_source_raw("(put 'romacs-terminal-not-running 'error-conditions '(romacs-terminal-not-running error)) (put 'romacs-terminal-not-running 'error-message \"Terminal is not running\")")?;
        Ok(())
    }

    /// Snapshot without evaluating Lisp, selecting windows or sending PTY input.
    pub fn terminal_snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String> {
        self.terminal_host.as_mut().ok_or("No terminal host attached")?.snapshot(session)
    }

    /// Query input flags without materializing terminal history.
    pub fn terminal_input_state(&mut self, session: u64) -> Result<TerminalInputState, String> {
        self.terminal_host.as_mut().ok_or("No terminal host attached")?.input_state(session)
    }

    /// Send explicit input to a live terminal session.
    pub fn terminal_send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String> {
        self.terminal_host.as_mut().ok_or("No terminal host attached")?.send(session, bytes)
    }

    /// Resize the terminal grid and PTY as one native operation.
    pub fn terminal_resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String> {
        if columns == 0 || rows == 0 { return Err("Invalid terminal geometry".into()); }
        self.terminal_host.as_mut().ok_or("No terminal host attached")?.resize(session, columns, rows)
    }

    /// The selected terminal's identity and whether it currently receives input.
    pub fn selected_terminal(&mut self) -> Result<Option<(u64, bool)>, String> {
        let window = self.selected_window_value()?;
        let buffer = self.window(window)?.buffer;
        Ok(self.terminal_bindings.iter().find(|b| b.buffer == buffer).map(|b| (b.session, b.follow)))
    }

    /// Apply publications to their buffers; return whether a displayed buffer changed.
    pub fn poll_terminal_output(&mut self) -> Result<bool, String> {
        use crate::coordinates::ByteOffset;
        use crate::data::Object;
        let Some(host) = self.terminal_host.as_mut() else { return Ok(false); };
        let ready = host.changed_sessions();
        if self.terminal_bindings.is_empty() { return Ok(false); }
        self.ensure_windows()?;
        let mut changed = false;
        let mut index = 0;
        while index < self.terminal_bindings.len() {
            let (buffer, session, revision, follow) = {
                let binding = &self.terminal_bindings[index];
                (binding.buffer, binding.session, binding.revision, binding.follow)
            };
            if !self.live_buffer_value(buffer) {
                self.terminal_host.as_mut().ok_or("No terminal host attached")?.close(session)?;
                self.terminal_bindings.remove(index);
                changed |= self.buffer_is_displayed(buffer)?;
                continue;
            }
            if revision.is_some() && !ready.contains(&session) { index += 1; continue; }
            let snapshot = self.terminal_snapshot(session)?;
            if snapshot.columns == 0 || snapshot.rows == 0
                || !snapshot.text.is_char_boundary(snapshot.cursor_byte)
                || !snapshot.text.is_char_boundary(snapshot.screen_start_byte)
                || snapshot.screen_start_byte > snapshot.cursor_byte
            {
                return Err("Invalid terminal publication".into());
            }
            let mut previous_end = 0;
            for run in snapshot.styles.iter() {
                if run.bytes.start < previous_end || run.bytes.start >= run.bytes.end
                    || !snapshot.text.is_char_boundary(run.bytes.start) || !snapshot.text.is_char_boundary(run.bytes.end) {
                    return Err("Invalid terminal style range".into());
                }
                previous_end = run.bytes.end;
            }
            if revision == Some(snapshot.revision) || (!follow && snapshot.alternate_screen && revision.is_some()) {
                index += 1;
                continue;
            }
            // Decided before the new text moves (or, when history rolls
            // over, collapses) their markers.
            let following = if follow { self.following_windows(index)? } else { vec![] };
            let Object::Buffer(text) = self.object_mut(buffer)? else {
                return Err("Terminal requires a Unicode buffer".into());
            };
            let old = text.as_str();
            let point = text.point.0;
            let mut prefix = old.bytes().zip(snapshot.text.bytes()).take_while(|(a, b)| a == b).count();
            while !old.is_char_boundary(prefix) || !snapshot.text.is_char_boundary(prefix) { prefix -= 1; }
            let mut suffix = old[prefix..].bytes().rev().zip(snapshot.text[prefix..].bytes().rev()).take_while(|(a, b)| a == b).count();
            while !old.is_char_boundary(old.len() - suffix) || !snapshot.text.is_char_boundary(snapshot.text.len() - suffix) { suffix -= 1; }
            let end = old.len() - suffix;
            let inserted_end = snapshot.text.len() - suffix;
            let next_point = if follow {
                snapshot.cursor_byte
            } else if point <= prefix {
                point
            } else if point >= end {
                inserted_end + point - end
            } else {
                prefix + (point - prefix).min(inserted_end - prefix)
            };
            if prefix != end || prefix != inserted_end {
                let index = buffer.index(0)? - 1;
                let marks = self.family_record(index);
                let Object::Buffer(text) = &mut self.objects[index] else { unreachable!("terminal buffer") };
                let edited = text.replace(ByteOffset(prefix)..ByteOffset(end), &snapshot.text.as_bytes()[prefix..inserted_end]);
                self.family_fetch(marks);
                edited?;
            }
            let Object::Buffer(text) = self.object_mut(buffer)? else { unreachable!("terminal buffer") };
            let mut next_point = next_point.min(snapshot.text.len());
            while !snapshot.text.is_char_boundary(next_point) { next_point -= 1; }
            let accessible = text.accessible();
            text.point = ByteOffset(next_point.clamp(accessible.start.0, accessible.end.0));
            text.state.set_modified(false);
            self.finish_annotation_edit(buffer)?;
            if !following.is_empty() {
                let point = Value::integer(snapshot.text[..snapshot.cursor_byte].chars().count() as i64 + 1)?;
                let start = Value::integer(snapshot.text[..snapshot.screen_start_byte].chars().count() as i64 + 1)?;
                for window in following {
                    let window = self.window(window)?;
                    let (point_marker, start_marker) = (window.point, window.start);
                    self.set_marker_object(point_marker, point, buffer)?;
                    self.set_marker_object(start_marker, start, buffer)?;
                }
            }
            self.terminal_bindings[index].cursor_byte = snapshot.cursor_byte;
            self.terminal_bindings[index].screen_start_byte = snapshot.screen_start_byte;
            self.terminal_bindings[index].default_colors = snapshot.default_colors;
            self.terminal_bindings[index].styles = snapshot.styles.clone();
            self.terminal_bindings[index].revision = Some(snapshot.revision);
            changed |= self.buffer_is_displayed(buffer)?;
            index += 1;
        }
        Ok(changed)
    }

    /// The window whose size BUFFER's pty follows: the selected window
    /// (the one a minibuffer was entered from, while it is selected) when
    /// it shows BUFFER, else the first window in tree order that does.
    pub(crate) fn controlling_window(&mut self, buffer: Value, showing: &[Value]) -> Result<Option<Value>, String> {
        let mut selected = self.selected_window_value()?;
        if self.window(selected)?.mini {
            let slot = self.slot("romacs--minibuf-selected-window");
            let outer = self.variable_value(slot)?;
            if matches!(self.object_if(outer), Some(crate::data::Object::Window(w)) if w.live()) { selected = outer; }
        }
        if self.window(selected)?.buffer == buffer && showing.contains(&selected) { return Ok(Some(selected)); }
        Ok(showing.first().copied())
    }

    /// The windows showing binding INDEX's buffer that follow its grid: the
    /// selected one, and any other not scrolled back before the grid of the
    /// last publication.
    fn following_windows(&mut self, index: usize) -> Result<Vec<Value>, String> {
        let binding = &self.terminal_bindings[index];
        let (buffer, screen) = (binding.buffer, binding.screen_start_byte);
        let selected = self.selected_window_value()?;
        let mut out = vec![];
        for w in self.leaf_windows()? {
            let window = self.window(w)?;
            if window.buffer != buffer { continue; }
            let start = self.marker_byte(self.marker(window.start)?)?;
            if w == selected || start.is_none_or(|start| start >= screen) { out.push(w); }
        }
        Ok(out)
    }

    /// Run at each geometry publication: size every terminal's pty to its
    /// controlling window's stable body (`stable_window_body'), so a grown
    /// mini-window never resizes it, and keep a following terminal's
    /// cursor row visible in each window showing it. Hidden terminals keep
    /// their size.
    pub(crate) fn reconcile_terminal_geometry(&mut self) -> Result<(), String> {
        if self.terminal_bindings.is_empty() { return Ok(()); }
        let leaves = self.leaf_windows()?;
        let selected = self.selected_window_value()?;
        for index in 0..self.terminal_bindings.len() {
            let buffer = self.terminal_bindings[index].buffer;
            let mut showing = vec![];
            for &w in &leaves { if self.window(w)?.buffer == buffer { showing.push(w); } }
            let Some(control) = self.controlling_window(buffer, &showing)? else { continue };
            if let Some((columns, rows)) = self.stable_window_body(control)? {
                let binding = &self.terminal_bindings[index];
                if (binding.columns, binding.rows) != (columns, rows) {
                    self.terminal_resize(binding.session, columns, rows)?;
                    self.terminal_bindings[index].columns = columns;
                    self.terminal_bindings[index].rows = rows;
                }
            }
            if self.terminal_bindings[index].follow && self.terminal_bindings[index].revision.is_some() {
                for &w in &showing { self.terminal_viewport(index, w, w == selected)?; }
            }
        }
        Ok(())
    }

    /// Show a following terminal's grid in WINDOW from its first row, or,
    /// while WINDOW is shorter than the grid, from the row that puts the
    /// cursor on its last line; a start its user scrolled above the grid
    /// stays. A
    /// window other than the selected one also takes the cursor as point.
    fn terminal_viewport(&mut self, index: usize, window: Value, selected: bool) -> Result<(), String> {
        use crate::coordinates::ByteOffset;
        let binding = &self.terminal_bindings[index];
        let (buffer, cursor, screen) = (binding.buffer, binding.cursor_byte, binding.screen_start_byte);
        let w = self.window(window)?;
        let rows = (w.height / self.window_state.line.max(1)).saturating_sub(1).max(1) as usize;
        let (start_marker, point_marker) = (w.start, w.point);
        let start = self.marker_byte(self.marker(start_marker)?)?;
        let point = self.marker_byte(self.marker(point_marker)?)?;
        let crate::data::Object::Buffer(text) = self.object(buffer)? else { return Ok(()) };
        let all = text.as_str();
        if screen > cursor || cursor > all.len() || !all.is_char_boundary(screen) || !all.is_char_boundary(cursor) { return Ok(()); }
        if start.is_some_and(|start| start < screen) { return Ok(()); }
        let wanted = viewport_start(all, screen, cursor, rows);
        let position = |byte| -> Result<Value, String> { Value::integer(text.char_of_byte(ByteOffset(byte))?.0 as i64 + 1) };
        let (wanted_start, wanted_point) = (position(wanted)?, position(cursor)?);
        if start != Some(wanted) { self.set_marker_object(start_marker, wanted_start, buffer)?; }
        if !selected && point != Some(cursor) { self.set_marker_object(point_marker, wanted_point, buffer)?; }
        Ok(())
    }

    /// Whether idle/background service work is needed, including hidden terminals.
    pub fn has_terminals(&self) -> bool { !self.terminal_bindings.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::viewport_start;

    #[test]
    fn viewport_keeps_the_cursor_row_on_the_last_line() {
        let text = "old\nr0\nr1\nr2\nr3";
        let (screen, cursor) = (4, text.len());
        assert_eq!(viewport_start(text, screen, cursor, 5), screen, "a grid that fits shows from its first row");
        assert_eq!(viewport_start(text, screen, cursor, 4), screen);
        assert_eq!(&text[viewport_start(text, screen, cursor, 2)..], "r2\nr3");
        assert_eq!(&text[viewport_start(text, screen, cursor, 1)..], "r3");
    }

    #[test]
    fn base_colours_come_from_the_theme_s_sixteen() {
        use super::{TerminalColor, TerminalStyle};
        let default = romacs_layout_input::Face { foreground: [200, 200, 200], background: [0, 0, 0], family: String::new(), pixel_size: 10.0, weight: 400, italic: false, width: 5 };
        let mut ansi = super::DEFAULT_ANSI;
        ansi[1] = [100, 50, 10];
        let style = |foreground| TerminalStyle { foreground, ..Default::default() }.face(&default, &ansi).foreground;
        assert_eq!(style(TerminalColor::Ansi(1)), [100, 50, 10]);
        assert_eq!(style(TerminalColor::DimAnsi(1)), [66, 33, 6], "a dim base colour is the theme's at 0.66");
        assert_eq!(style(TerminalColor::Ansi(2)), super::DEFAULT_ANSI[2]);
        assert_eq!(style(TerminalColor::Rgb([1, 2, 3])), [1, 2, 3], "a colour the session set stays literal");
    }
}
