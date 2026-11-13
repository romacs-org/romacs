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
    pub alternate_screen: bool,
    /// Whether the shell's own process group is the PTY's foreground one
    /// (`tcgetpgrp' of the PTY is the shell's pid): the shell, not a
    /// program it runs, reads the terminal, as at its prompt.
    pub shell_foreground: bool,
}

impl TerminalSnapshot {
    /// Read protocol flags from an immutable publication.
    pub fn input_state(&self) -> TerminalInputState {
        // A publication does not know the foreground process group.
        TerminalInputState { running: self.running, application_cursor: self.application_cursor, bracketed_paste: self.bracketed_paste, alternate_screen: self.alternate_screen, shell_foreground: false }
    }
}

/// Per-evaluator native terminal service. Calls must not evaluate Lisp.
pub trait TerminalHost {
    fn create(&mut self, shell: &str, directory: &str, columns: u16, rows: u16) -> Result<u64, String>;
    /// A terminal that already runs, behind the control socket of its
    /// helper process: one a keeper spawned for this session, or kept for
    /// it across a restart. PTY is the master of the pty the spawner made
    /// for it, when it made one: the helper takes it if its shell is not
    /// started yet. A host with no helper processes has none.
    ///
    /// # Errors
    /// The helper does not answer, or speaks a version this session cannot read.
    fn adopt(&mut self, _control: std::os::fd::OwnedFd, _pty: Option<std::os::fd::OwnedFd>, _columns: u16, _rows: u16) -> Result<u64, String> {
        Err("This terminal host adopts no helper process".into())
    }
    fn send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String>;
    fn resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String>;
    fn input_state(&mut self, session: u64) -> Result<TerminalInputState, String>;
    /// Drain identities whose native state changed since the last publication pass.
    fn changed_sessions(&mut self) -> Vec<u64>;
    fn snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String>;
    fn close(&mut self, session: u64) -> Result<(), String>;
    /// How many of SESSION's updates arrived as the whole grid, and how
    /// many as a change to it; `None` for a host that has no such transport.
    fn transport_counts(&mut self, _session: u64) -> Option<(u64, u64)> { None }
    /// The record version SESSION's helper process speaks, for as long as
    /// it lives; `None` for a host with no helper processes.
    fn helper_version(&mut self, _session: u64) -> Option<u16> { None }
}

pub(crate) struct TerminalBinding {
    pub(crate) buffer: Value,
    pub(crate) session: u64,
    pub(crate) revision: Option<u64>,
    /// The windows showing the buffer that do not follow the terminal: a
    /// command moved their point off the cursor or their start off the
    /// grid. The rest follow. Every window writer obeys it (publication,
    /// geometry); the alternate screen clears it. Roots for collection.
    pub(crate) away: Vec<Value>,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) styles: std::sync::Arc<[TerminalStyleRun]>,
    pub(crate) default_colors: [Option<[u8; 3]>; 2],
    /// The last publication's cursor and first grid row, in buffer bytes.
    pub(crate) cursor_byte: usize,
    pub(crate) screen_start_byte: usize,
    /// Whether the last publication showed the alternate screen.
    pub(crate) alternate: bool,
    /// The keymap generation (and screen) at which neither the route's map
    /// nor the global map, `unremapped_maps', remapped `romacs-term-send-key':
    /// a keymap change, or another route or global map, asks again, as GNU's
    /// where_is_cache is flushed. The maps are roots for collection.
    pub(crate) unremapped: Option<(u64, bool)>,
    pub(crate) unremapped_maps: Vec<Value>,
}
romacs_snapshot::snapshot_struct!(TerminalBinding { buffer, session, revision, away, columns, rows, styles, default_colors, cursor_byte, screen_start_byte, alternate, unremapped, unremapped_maps });

impl TerminalBinding {
    pub(crate) fn new(buffer: Value, session: u64, columns: u16, rows: u16) -> Self {
        Self { buffer, session, revision: None, away: Vec::new(), columns, rows, styles: std::sync::Arc::from([]),
               default_colors: [None; 2], cursor_byte: 0, screen_start_byte: 0, alternate: false, unremapped: None, unremapped_maps: Vec::new() }
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
        self.terminal_host = host.map(|host| Box::new(crate::host_calls::Counted(host)) as Box<dyn TerminalHost>);
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
            let (buffer, session, revision) = {
                let binding = &self.terminal_bindings[index];
                (binding.buffer, binding.session, binding.revision)
            };
            if !self.live_buffer_value(buffer) {
                self.terminal_host.as_mut().ok_or("No terminal host attached")?.close(session)?;
                self.terminal_bindings.remove(index);
                self.terminal_ended(session, buffer);
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
            if revision == Some(snapshot.revision) {
                index += 1;
                continue;
            }
            // The alternate screen has no away: its windows all follow.
            if snapshot.alternate_screen { self.terminal_bindings[index].away.clear(); }
            self.prune_away(index)?;
            // Buffer point is the selected window's when it shows the buffer.
            let selected = self.selected_window_value()?;
            let selected_shows = self.window(selected)?.buffer == buffer;
            let follow = !(selected_shows && self.terminal_bindings[index].away.contains(&selected));
            // Decided before the new text moves (or, when history rolls
            // over, collapses) their markers.
            let following = self.following_windows(index)?;
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
                let Object::Buffer(text) = self.objects.entry_mut(index) else { unreachable!("terminal buffer") };
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
            if prefix != end || prefix != inserted_end {
                self.evaporate_overlays_at(buffer, ByteOffset(prefix)..ByteOffset(inserted_end))?;
            }
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
            self.terminal_bindings[index].alternate = snapshot.alternate_screen;
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
        let buffer = self.terminal_bindings[index].buffer;
        let mut out = vec![];
        for w in self.leaf_windows()? {
            if self.window(w)?.buffer == buffer && !self.terminal_bindings[index].away.contains(&w) { out.push(w); }
        }
        Ok(out)
    }

    /// Forget away windows that are dead or no longer show the buffer.
    fn prune_away(&mut self, index: usize) -> Result<(), String> {
        let buffer = self.terminal_bindings[index].buffer;
        let mut away = std::mem::take(&mut self.terminal_bindings[index].away);
        let mut kept = Vec::with_capacity(away.len());
        for w in away.drain(..) {
            let live = matches!(self.object_if(w), Some(crate::data::Object::Window(window)) if window.live());
            if live && self.window(w)?.buffer == buffer { kept.push(w); }
        }
        self.terminal_bindings[index].away = kept;
        Ok(())
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
            if self.terminal_bindings[index].revision.is_some() {
                self.prune_away(index)?;
                for &w in &showing {
                    if !self.terminal_bindings[index].away.contains(&w) { self.terminal_viewport(index, w, w == selected)?; }
                }
            }
        }
        Ok(())
    }

    /// Show a following terminal's grid in WINDOW from its first row, or,
    /// while WINDOW is shorter than the grid, from the row that puts the
    /// cursor on its last line. A window other than the selected one also
    /// takes the cursor as point. Only following windows come here: a window
    /// its user scrolled is away (`TerminalBinding::away`).
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
        let wanted = viewport_start(all, screen, cursor, rows);
        let position = |byte| -> Result<Value, String> { Value::integer(text.char_of_byte(ByteOffset(byte))?.0 as i64 + 1) };
        let (wanted_start, wanted_point) = (position(wanted)?, position(cursor)?);
        if start != Some(wanted) { self.set_marker_object(start_marker, wanted_start, buffer)?; }
        if !selected && point != Some(cursor) { self.set_marker_object(point_marker, wanted_point, buffer)?; }
        Ok(())
    }

    /// A window's point and start, as bytes of BUFFER: the selected
    /// window's point is the buffer's.
    fn window_place(&mut self, window: Value, buffer: Value, selected: bool) -> Result<Option<(usize, usize)>, String> {
        let w = self.window(window)?;
        let (start_marker, point_marker) = (w.start, w.point);
        let start = self.marker_byte(self.marker(start_marker)?)?;
        let point = if selected {
            match self.object(buffer)? { crate::data::Object::Buffer(text) => Some(text.point.0), _ => None }
        } else {
            self.marker_byte(self.marker(point_marker)?)?
        };
        Ok(start.zip(point).map(|(start, point)| (point, start)))
    }

    /// Mark away every window showing a published terminal on its normal
    /// screen whose point left the cursor, or whose start left the grid:
    /// before the screen's first row, or past the cursor's row. Run after
    /// each command (`romacs-term--track'), so a command in any window, a
    /// scroll from another (`scroll-other-window') included, counts.
    pub(crate) fn terminal_track(&mut self) -> Result<(), String> {
        if self.terminal_bindings.is_empty() { return Ok(()); }
        let leaves = self.leaf_windows()?;
        let selected = self.selected_window_value()?;
        for index in 0..self.terminal_bindings.len() {
            let binding = &self.terminal_bindings[index];
            if binding.revision.is_none() || binding.alternate { continue; }
            let (buffer, cursor, screen) = (binding.buffer, binding.cursor_byte, binding.screen_start_byte);
            let line = match self.object(buffer)? {
                crate::data::Object::Buffer(text) if cursor <= text.as_str().len() => text.as_str()[..cursor].rfind('\n').map_or(0, |i| i + 1),
                _ => continue,
            };
            for &w in &leaves {
                if self.window(w)?.buffer != buffer || self.terminal_bindings[index].away.contains(&w) { continue; }
                let Some((point, start)) = self.window_place(w, buffer, w == selected)? else { continue };
                if point != cursor || start < screen || start > line.max(screen) {
                    self.terminal_bindings[index].away.push(w);
                }
            }
        }
        Ok(())
    }

    /// Make WINDOW follow binding INDEX again, at once: its start where a
    /// publication would put it and its point (the buffer's, when it is
    /// the selected window) on the cursor.
    pub(crate) fn terminal_snap(&mut self, index: usize, window: Value) -> Result<(), String> {
        self.terminal_bindings[index].away.retain(|&w| w != window);
        let binding = &self.terminal_bindings[index];
        if binding.revision.is_none() || self.window(window)?.buffer != binding.buffer { return Ok(()); }
        let (buffer, cursor) = (binding.buffer, binding.cursor_byte);
        let selected = window == self.selected_window_value()?;
        self.terminal_viewport(index, window, selected)?;
        if selected {
            let crate::data::Object::Buffer(text) = self.object_mut(buffer)? else { return Ok(()) };
            let accessible = text.accessible();
            if text.as_str().is_char_boundary(cursor) {
                text.point = crate::coordinates::ByteOffset(cursor.clamp(accessible.start.0, accessible.end.0));
            }
        }
        Ok(())
    }

    /// Send EVENT, a key just read at the start of a command, straight to the
    /// selected terminal when the command loop would only have sent it:
    /// no minibuffer or recursive edit is reading; the terminal follows its
    /// cursor in the selected window, runs, and has its own local map; no keyboard macro, input method, keyboard
    /// translation or overriding map is in effect; no buffer-local command
    /// hook but romacs-term's own is set (global ones are skipped for the
    /// key); no text property, emulation or minor-mode
    /// map binds or remaps the key; and the route's map (romacs-term's
    /// generated one, under `romacs-term-mode-map') or the global map
    /// through it gives `romacs-term-send-key'.  Then the bytes are written
    /// and true returned; else false, and the command loop reads the key as
    /// usual.  Lisp never runs here.
    pub fn terminal_passthrough_key(&mut self, event: &crate::CommandEvent) -> Result<bool, String> {
        if self.terminal_bindings.is_empty() { return Ok(false); }
        // A minibuffer or recursive edit reads its own keys, whichever
        // window is selected.
        for level in ["romacs--minibuf-level", "romacs--command-loop-level"] {
            let slot = self.slot(level);
            if self.variable_value(slot)?.integer_value().is_some_and(|n| n > 0) { return Ok(false); }
        }
        let window = self.selected_window_value()?;
        let w = self.window(window)?;
        if w.mini { return Ok(false); }
        let buffer = w.buffer;
        let Some(index) = self.terminal_bindings.iter().position(|b| b.buffer == buffer) else { return Ok(false) };
        let binding = &self.terminal_bindings[index];
        if binding.revision.is_none() || binding.away.contains(&window) { return Ok(false); }
        let (session, alternate) = (binding.session, binding.alternate);
        let value = |vm: &mut Self, name: &str| -> Result<Value, String> {
            let slot = vm.slot(name);
            let v = vm.value_in_buffer(slot, buffer)?;
            Ok(if v == Value::UNBOUND { Value::NIL } else { v })
        };
        for name in ["defining-kbd-macro", "executing-kbd-macro", "overriding-terminal-local-map", "overriding-local-map",
                     "keyboard-translate-table"] {
            if value(self, name)? != Value::NIL { return Ok(false); }
        }
        // A translation of the key, or one it begins, is the reader's
        // (`input-decode-map', `key-translation-map').
        if let Ok(key) = match event {
            crate::CommandEvent::Character(code) => Value::integer(*code),
            crate::CommandEvent::Named(name) => Ok(self.intern(name)),
        } {
            for name in ["input-decode-map", "key-translation-map"] {
                let map = value(self, name)?;
                let Some(map) = self.resolve_keymap(map) else { continue };
                if self.lookup_one_event(map, key, false, true).map_or(true, |b| b != Value::NIL) { return Ok(false); }
            }
        }
        // GNU's `input-method-function' is `list' when no input method is on.
        let method = value(self, "input-method-function")?;
        if method != Value::NIL && method != self.intern("list") { return Ok(false); }
        // Global command hooks are skipped: the key's effect is in the pty,
        // not the buffer (VM2-TERM-KEYS).  A buffer-local hook other than
        // romacs-term's own (and the local `t') is a package watching this
        // buffer's keys, so they go through the loop.
        let ours = self.intern("romacs-term--track");
        for hook in ["pre-command-hook", "post-command-hook"] {
            let slot = self.slot(hook);
            let local = value(self, hook)?;
            if local == self.slots[slot].value { continue; }
            let mut tail = local;
            for _ in 0..64 {
                let Ok((head, rest)) = self.cons_pair(tail) else { break };
                if head != ours && head != Value::TRUE { return Ok(false); }
                tail = rest;
            }
            if tail != Value::NIL && self.cons_pair(tail).is_err() { return Ok(false); }
        }
        let mode_map = value(self, "romacs-term-mode-map")?;
        let index_of = buffer.index(0)? - 1;
        if *self.local_keymap(index_of) != mode_map || mode_map == Value::NIL { return Ok(false); }
        let event = match event {
            crate::CommandEvent::Character(code) => Value::integer(*code)?,
            crate::CommandEvent::Named(name) => self.intern(name),
        };
        let send = self.intern("romacs-term-send-key");
        // A keymap or local-map property at point comes first: the
        // character's (`get-char-property'), or the one an insertion there
        // would inherit (`get-pos-property'), as keymap.c get_local_map
        // finds them (`Vm::current_active_maps').
        // The reader widens before it looks: a
        // narrowed buffer goes through the loop.
        let point = match self.object(buffer)? {
            crate::data::Object::Buffer(text) if text.accessible() == (crate::coordinates::ByteOffset(0)..crate::coordinates::ByteOffset(text.as_str().len())) =>
                text.char_of_byte(text.point)?.0 as i64 + 1,
            _ => return Ok(false),
        };
        for property in ["keymap", "local-map"] {
            let property = self.intern(property);
            let at = Value::integer(point)?;
            if self.get_char_property_values(at, property, buffer)? != Value::NIL { return Ok(false); }
            if self.get_pos_property_values(at, property, buffer).map_err(|_| "keymap property")? != Value::NIL { return Ok(false); }
        }
        // The route's map, then the global map, as the command loop's lookup:
        // `romacs-term-send-key' directly, or a global command the route
        // remaps to it (`self-insert-command' for non-ASCII characters).
        // While the shell itself has the terminal (its prompt), the shell
        // route keeps `romacs-term-shell-exceptions' for Emacs too.
        let shell = !alternate && self.terminal_input_state(session).is_ok_and(|s| s.shell_foreground);
        let route = value(self, if alternate { "romacs-term-alternate-map" } else if shell { "romacs-term--shell" } else { "romacs-term--passthrough" })?;
        let global = self.slots[self.names["#:vm2-global-keymap"]].value;
        // Events with default bindings, as the reader's `key-binding'; remaps
        // without, as `command-remapping'.
        let lookup = |vm: &mut Self, map: Value, key: Value, remap: bool| vm.lookup_one_event(map, key, remap, !remap).ok();
        let Some(direct) = lookup(self, route, event, false) else { return Ok(false) };
        let remapped = if direct == send { None } else if direct != Value::NIL { return Ok(false) } else {
            let Some(command) = lookup(self, global, event, false) else { return Ok(false) };
            if command.tag() != 7 || lookup(self, route, command, true) != Some(send) { return Ok(false); }
            Some(command)
        };
        let generation = (self.keymap_generation(), alternate);
        let binding = &self.terminal_bindings[index];
        if binding.unremapped != Some(generation) || binding.unremapped_maps != [route, global] {
            for map in [route, global] {
                if lookup(self, map, send, true) != Some(Value::NIL) { return Ok(false); }
            }
            self.terminal_bindings[index].unremapped = Some(generation);
            self.terminal_bindings[index].unremapped_maps = vec![route, global];
        }
        // Emulation and minor-mode maps that are on and bind the key or remap
        // a command the lookup went through come first: the loop decides.
        let mut alists = vec![];
        let mut tail = value(self, "emulation-mode-map-alists")?;
        for _ in 0..64 {
            let Ok((head, rest)) = self.cons_pair(tail) else { break };
            alists.push(if head.tag() == 7 { let v = self.value_in_buffer(head.index(7)? - 1, buffer)?; if v == Value::UNBOUND { Value::NIL } else { v } } else { head });
            tail = rest;
        }
        alists.push(value(self, "minor-mode-overriding-map-alist")?);
        alists.push(value(self, "minor-mode-map-alist")?);
        for alist in alists {
            let mut tail = alist;
            for _ in 0..4096 {
                let Ok((entry, rest)) = self.cons_pair(tail) else { break };
                tail = rest;
                let Ok((variable, map)) = self.cons_pair(entry) else { continue };
                if variable.tag() != 7 { continue; }
                let on = self.value_in_buffer(variable.index(7)? - 1, buffer)?;
                if on == Value::NIL || on == Value::UNBOUND { continue; }
                let Some(map) = self.resolve_keymap(map) else { continue };
                for (key, remap) in [(Some(event), false), (Some(send), true), (remapped, true)] {
                    let Some(key) = key else { continue };
                    if lookup(self, map, key, remap) != Some(Value::NIL) { return Ok(false); }
                }
            }
        }
        let state = self.terminal_input_state(session)?;
        if !state.running { return Ok(false); }
        let key = if let Some(code) = event.integer_value() {
            match crate::terminal_keys::character_event(code) { Some(key) => key, None => return Ok(false) }
        } else {
            let name = self.symbol_names[event.index(7)? - 1].clone();
            let bytes = match crate::terminal_keys::encode(crate::terminal_keys::symbol_event(&name), state.application_cursor) {
                Ok(bytes) if !bytes.is_empty() => bytes,
                _ => return Ok(false),
            };
            self.terminal_send(session, &bytes)?;
            return Ok(true);
        };
        match crate::terminal_keys::encode(key, state.application_cursor) {
            Ok(bytes) if !bytes.is_empty() => { self.terminal_send(session, &bytes)?; Ok(true) }
            _ => Ok(false),
        }
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
        let default = romacs_layout_input::Face { decor: Default::default(), foreground: [200, 200, 200], background: [0, 0, 0], family: String::new(), pixel_size: 10.0, weight: 400, italic: false, width: 5 };
        let mut ansi = super::DEFAULT_ANSI;
        ansi[1] = [100, 50, 10];
        let style = |foreground| TerminalStyle { foreground, ..Default::default() }.face(&default, &ansi).foreground;
        assert_eq!(style(TerminalColor::Ansi(1)), [100, 50, 10]);
        assert_eq!(style(TerminalColor::DimAnsi(1)), [66, 33, 6], "a dim base colour is the theme's at 0.66");
        assert_eq!(style(TerminalColor::Ansi(2)), super::DEFAULT_ANSI[2]);
        assert_eq!(style(TerminalColor::Rgb([1, 2, 3])), [1, 2, 3], "a colour the session set stays literal");
    }
}
