use crate::{Value, Vm};

/// Terminal defaults follow the editor theme; explicit ANSI colours remain literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalColor { Foreground, Background, DimForeground, Rgb([u8; 3]) }
romacs_snapshot::snapshot_enum!(TerminalColor {
    TerminalColor::Foreground => (),
    TerminalColor::Background => (),
    TerminalColor::DimForeground => (),
    TerminalColor::Rgb(f0) => (f0),
});

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
    pub(crate) fn face(self, default: &romacs_layout_input::Face) -> romacs_layout_input::Face {
        let color = |value| match value {
            TerminalColor::Foreground => default.foreground,
            TerminalColor::Background => default.background,
            TerminalColor::DimForeground => default.foreground.map(|v| (f32::from(v) * 0.66) as u8),
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
}
romacs_snapshot::snapshot_struct!(TerminalBinding { buffer, session, revision, follow, columns, rows, styles, default_colors });

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
            let Object::Buffer(text) = &mut self.objects[buffer.index(0)? - 1] else {
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
            let Object::Buffer(text) = &mut self.objects[buffer.index(0)? - 1] else { unreachable!("terminal buffer") };
            let mut next_point = next_point.min(snapshot.text.len());
            while !snapshot.text.is_char_boundary(next_point) { next_point -= 1; }
            let accessible = text.accessible();
            text.point = ByteOffset(next_point.clamp(accessible.start.0, accessible.end.0));
            text.state.set_modified(false);
            self.finish_annotation_edit(buffer)?;
            if follow {
                let selected = self.selected_window_value()?;
                if self.window(selected)?.buffer == buffer {
                    let point = snapshot.text[..snapshot.cursor_byte].chars().count() + 1;
                    let start = snapshot.text[..snapshot.screen_start_byte].chars().count() + 1;
                    let window = self.window(selected)?;
                    let (point_marker, start_marker) = (window.point, window.start);
                    self.set_marker_object(point_marker, Value::integer(point as i64)?, buffer)?;
                    self.set_marker_object(start_marker, Value::integer(start as i64)?, buffer)?;
                }
            }
            self.terminal_bindings[index].default_colors = snapshot.default_colors;
            self.terminal_bindings[index].styles = snapshot.styles.clone();
            self.terminal_bindings[index].revision = Some(snapshot.revision);
            changed |= self.buffer_is_displayed(buffer)?;
            index += 1;
        }
        Ok(changed)
    }

    /// Apply changed geometry from the selected terminal-input view only.
    pub fn resize_selected_terminal(&mut self) -> Result<(), String> {
        let window = self.selected_window_value()?;
        let view = self.window(window)?;
        let Some(index) = self.terminal_bindings.iter().position(|b| b.buffer == view.buffer && b.follow) else { return Ok(()); };
        let columns = (view.width.saturating_sub(view.left_margin + view.right_margin) / self.window_state.column.max(1)).clamp(2, u16::MAX as u32) as u16;
        let rows = (view.height / self.window_state.line.max(1)).saturating_sub(1).clamp(1, u16::MAX as u32) as u16;
        let binding = &self.terminal_bindings[index];
        if (binding.columns, binding.rows) != (columns, rows) {
            self.terminal_resize(binding.session, columns, rows)?;
            self.terminal_bindings[index].columns = columns;
            self.terminal_bindings[index].rows = rows;
        }
        Ok(())
    }

    /// Whether idle/background service work is needed, including hidden terminals.
    pub fn has_terminals(&self) -> bool { !self.terminal_bindings.is_empty() }
}
