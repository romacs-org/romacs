//! The session's side of its terminals.
//!
//! A terminal is a helper process (`terminal_helper.rs`). This is the
//! `TerminalHost` the VM talks to: it attaches a conversation to a helper,
//! keeps a mirror of the helper's grid from the pushes it is sent, and
//! turns the mirror into what a terminal buffer shows: text, style runs,
//! the cursor's place. That projection is display policy and lives here, so
//! it changes with the session and never with a running helper.
//!
//! Nothing here asks the helper a question and waits: keys and resizes are
//! sent, and modes, the shell's hold of the terminal and the grid arrive by
//! push. A reader thread per terminal applies pushes to the mirror; the
//! VM's thread projects on its own time and acknowledges, which is what
//! lets the helper coalesce while the session is busy.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use romacs_keeper::spawn::{self, End, Spec};
use romacs_record::{Decoder, Hello, Writer, channel, record};
use romacs_vm::{TerminalColor, TerminalHost, TerminalInputState, TerminalSnapshot, TerminalStyle, TerminalStyleRun};

use crate::terminal_proto::{self as proto, Cell, Push, Row, default_color, dim, kind};

/// The colours a program set, by Alacritty's index.
type Palette = [Option<[u8; 3]>; proto::COLORS];

fn terminal_color(color: Color, colors: &Palette) -> TerminalColor {
    match color {
        Color::Spec(rgb) => TerminalColor::Rgb([rgb.r, rgb.g, rgb.b]),
        Color::Named(NamedColor::Foreground) => TerminalColor::Foreground,
        Color::Named(NamedColor::Background) => TerminalColor::Background,
        Color::Named(NamedColor::DimForeground) if colors[NamedColor::DimForeground as usize].is_none() => TerminalColor::DimForeground,
        Color::Named(NamedColor::BrightForeground) if colors[NamedColor::BrightForeground as usize].is_none() => TerminalColor::Foreground,
        Color::Named(name) => indexed_color(name as usize, colors),
        Color::Indexed(index) => indexed_color(usize::from(index), colors),
    }
}

/// The colour of slot INDEX: the session's own where it set one, else the
/// theme's for the sixteen base colours and their dim forms, else literal.
fn indexed_color(index: usize, colors: &Palette) -> TerminalColor {
    let dims = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
    match colors[index] {
        Some(set) => TerminalColor::Rgb(set),
        None if index < 16 => TerminalColor::Ansi(u8::try_from(index).unwrap_or(0)),
        None if dims.contains(&index) => {
            let base = index - NamedColor::DimBlack as usize;
            colors[base].map_or(TerminalColor::DimAnsi(u8::try_from(base).unwrap_or(0)), |set| TerminalColor::Rgb(dim(set)))
        }
        None => TerminalColor::Rgb(default_color(index)),
    }
}

fn cell_style(cell: &Cell, colors: &Palette) -> TerminalStyle {
    let foreground = match cell.fg {
        Color::Spec(rgb) if cell.flags.contains(Flags::DIM) => TerminalColor::Rgb(dim([rgb.r, rgb.g, rgb.b])),
        Color::Named(name) if (cell.flags & Flags::DIM_BOLD).intersects(Flags::DIM) => terminal_color(Color::Named(name.to_dim()), colors),
        Color::Indexed(index) if cell.flags & Flags::DIM_BOLD == Flags::DIM && index < 8 => indexed_color(NamedColor::DimBlack as usize + usize::from(index), colors),
        Color::Indexed(index) if cell.flags & Flags::DIM_BOLD == Flags::DIM && index < 16 => terminal_color(Color::Indexed(index - 8), colors),
        color => terminal_color(color, colors),
    };
    TerminalStyle { foreground, background: terminal_color(cell.bg, colors), inverse: cell.flags.contains(Flags::INVERSE), hidden: cell.flags.contains(Flags::HIDDEN) }
}

/// One grid line as text and style runs, the runs' bytes from its start.
#[derive(Clone, Default)]
struct Projected {
    text: String,
    styles: Vec<TerminalStyleRun>,
    /// Where the cursor falls in `text`, when it is on this line.
    cursor: Option<usize>,
}

/// Project ROW of a grid COLUMNS wide. CURSOR is the cursor's column and
/// whether its next character wraps, when it is on this line; LAST is
/// whether this is the grid's bottom line, which ends without a newline.
fn project(row: &Row, columns: usize, cursor: Option<(usize, bool)>, last: bool, colors: &Palette) -> Projected {
    let blank = Cell::default();
    let cell = |column: usize| row.get(column).unwrap_or(&blank);
    let wrapped = columns > 0 && cell(columns - 1).flags.contains(Flags::WRAPLINE);
    let mut end = columns;
    if !wrapped {
        while end > 0 {
            let c = cell(end - 1);
            if c.c != ' ' || !c.zero_width.is_empty() || c.bg != Color::Named(NamedColor::Background) || c.flags.contains(Flags::INVERSE) { break; }
            end -= 1;
        }
        if let Some((column, wraps)) = cursor { end = end.max(column + usize::from(wraps)); }
    }
    let mut out = Projected::default();
    for column in 0..end {
        if cursor.is_some_and(|(at, _)| at == column) { out.cursor = Some(out.text.len()); }
        let c = cell(column);
        if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) { continue; }
        let start = out.text.len();
        out.text.push(if c.c == '\t' { ' ' } else { c.c });
        out.text.extend(c.zero_width.iter());
        let style = cell_style(c, colors);
        if style != TerminalStyle::default() {
            match out.styles.last_mut() {
                Some(run) if run.bytes.end == start && run.style == style => run.bytes.end = out.text.len(),
                _ => out.styles.push(TerminalStyleRun { bytes: start..out.text.len(), style }),
            }
        }
    }
    if let Some((column, wraps)) = cursor && (column >= end || wraps) { out.cursor = Some(out.text.len()); }
    if !wrapped && !last { out.text.push('\n'); }
    out
}

/// The helper's grid as this session knows it.
struct Mirror {
    /// History then screen; a history line keeps its projection.
    lines: VecDeque<(Row, Option<Projected>)>,
    state: Push,
    colors: Box<Palette>,
}

impl Mirror {
    /// Bring the mirror to PUSH. An inconsistent push is refused, and the
    /// caller asks for a full one.
    fn apply(&mut self, mut push: Push) -> Result<(), ()> {
        let total = push.history as usize + usize::from(push.rows);
        if push.scrolled.is_none() {
            self.lines.clear();
            self.lines.resize(total, (Row::new(), None));
        } else {
            if (push.columns, push.rows) != (self.state.columns, self.state.rows) { return Err(()); }
            for _ in 0..push.scrolled.unwrap_or(0) { self.lines.push_back((Row::new(), None)); }
            if self.lines.len() < total { return Err(()); }
            self.lines.drain(..self.lines.len() - total);
        }
        let mut colors = Box::new([None; proto::COLORS]);
        for (index, rgb) in &push.colors { *colors.get_mut(usize::from(*index)).ok_or(())? = Some(*rgb); }
        if colors != self.colors {
            self.colors = colors;
            for line in &mut self.lines { line.1 = None; }
        }
        for (line, row) in std::mem::take(&mut push.lines) {
            let index = usize::try_from(i64::from(push.history) + i64::from(line)).map_err(drop)?;
            *self.lines.get_mut(index).ok_or(())? = (row, None);
        }
        self.state = push;
        Ok(())
    }

    /// What a terminal buffer shows of the mirror.
    fn snapshot(&mut self, revision: u64) -> TerminalSnapshot {
        let (history, columns) = (self.state.history as usize, usize::from(self.state.columns));
        let cursor_line = history + usize::from(self.state.cursor_line);
        let last = self.lines.len().saturating_sub(1);
        let (mut text, mut styles, mut cursor_byte, mut screen_start_byte) = (String::new(), Vec::new(), 0, 0);
        for (index, (row, cached)) in self.lines.iter_mut().enumerate() {
            if index == history { screen_start_byte = text.len(); }
            let cursor = (index == cursor_line).then_some((usize::from(self.state.cursor_column), self.state.cursor_wraps));
            // A history line never holds the cursor and never changes.
            let fresh = (index >= history || cached.is_none()).then(|| project(row, columns, cursor, index == last, &self.colors));
            let line = match (&fresh, &*cached) { (Some(fresh), _) => fresh, (None, Some(cached)) => cached, (None, None) => unreachable!("projected above") };
            if let Some(at) = line.cursor { cursor_byte = text.len() + at; }
            styles.extend(line.styles.iter().map(|run| TerminalStyleRun { bytes: text.len() + run.bytes.start..text.len() + run.bytes.end, style: run.style }));
            text.push_str(&line.text);
            if index < history && let Some(fresh) = fresh { *cached = Some(fresh); }
        }
        // Runs that touch across a line's end are one run, as the grid walk made them.
        let mut merged: Vec<TerminalStyleRun> = Vec::with_capacity(styles.len());
        for run in styles {
            match merged.last_mut() {
                Some(before) if before.bytes.end == run.bytes.start && before.style == run.style => before.bytes.end = run.bytes.end,
                _ => merged.push(run),
            }
        }
        let mode = TermMode::from_bits_truncate(self.state.mode);
        TerminalSnapshot {
            revision, text: text.into(), styles: merged.into(),
            default_colors: [self.colors[NamedColor::Foreground as usize], self.colors[NamedColor::Background as usize]],
            cursor_byte, screen_start_byte, columns: self.state.columns, rows: self.state.rows,
            alternate_screen: mode.contains(TermMode::ALT_SCREEN), running: self.state.running,
            application_cursor: mode.contains(TermMode::APP_CURSOR), bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        }
    }
}

/// What the reader thread and the VM's thread share of one terminal.
struct Shared {
    mirror: Mirror,
    /// The newest push applied and not yet acknowledged.
    unacked: Option<u64>,
    /// Whether the mirror changed since the last snapshot.
    dirty: bool,
    /// The helper is gone, or stopped making sense.
    gone: bool,
    /// Pushes received that carried every line, and that carried a delta.
    fulls: u64,
    deltas: u64,
}

struct Session {
    stream: UnixStream,
    /// Keeps the helper alive when this session spawned it itself.
    _control: UnixStream,
    shared: Arc<Mutex<Shared>>,
    _child: Option<Owned>,
    last: Option<TerminalSnapshot>,
}

/// A helper this session spawned itself, seen to its end when let go:
/// by a close, or by an attachment that failed.
struct Owned(Option<spawn::Child>);

impl Drop for Owned {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() { reap(child); }
    }
}

fn send(mut stream: &UnixStream, kind: u16, payload: &[u8]) -> Result<(), String> {
    let mut wire = Vec::new();
    record::encode(0, proto::CLASS, kind, payload, &mut wire);
    stream.write_all(&wire).map_err(|_| "Terminal session has exited".to_string())
}

/// Apply the helper's pushes to the mirror until the conversation ends.
fn listen(id: u64, stream: UnixStream, mut decoder: Decoder, shared: Arc<Mutex<Shared>>, mailbox: crate::terminal_wait::Mailbox) -> std::io::Result<()> {
    std::thread::Builder::new().name("term-mirror".into()).spawn(move || {
        let (mut buffer, mut parts) = (vec![0u8; 1 << 16], Vec::new());
        'read: loop {
            loop {
                match decoder.pop() {
                    Ok(Some(record)) if record.kind == kind::PART => parts.extend_from_slice(&record.payload),
                    Ok(Some(record)) if record.kind == kind::PUSH => {
                        parts.extend_from_slice(&record.payload);
                        let push = Push::decode(&std::mem::take(&mut parts));
                        let mut shared = shared.lock().unwrap();
                        let Ok(push) = push else { break 'read };
                        let (full, revision) = (push.scrolled.is_none(), push.revision);
                        if shared.mirror.apply(push).is_err() {
                            // Ask again from nothing: an OPEN is answered in full.
                            let size = Writer::default().u16(shared.mirror.state.columns).u16(shared.mirror.state.rows).finish();
                            if send(&stream, kind::OPEN, &size).is_err() || send(&stream, kind::ACK, &[]).is_err() { break 'read; }
                            continue;
                        }
                        if full { shared.fulls += 1; } else { shared.deltas += 1; }
                        (shared.unacked, shared.dirty) = (Some(revision), true);
                        drop(shared);
                        mailbox.mark(id);
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break 'read,
                }
            }
            match channel::receive(&stream, &mut buffer, &mut Vec::new()) {
                Ok(count @ 1..) => decoder.push(&buffer[..count]),
                _ => break,
            }
        }
        let mut shared = shared.lock().unwrap();
        (shared.gone, shared.dirty) = (true, true);
        shared.mirror.state.running = false;
        drop(shared);
        mailbox.mark(id);
    }).map(drop)
}

/// This process's umask, which a helper it spawns itself keeps.
fn umask() -> u32 {
    std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|status| status.lines().find_map(|line| line.strip_prefix("Umask:").and_then(|mask| u32::from_str_radix(mask.trim(), 8).ok())))
        .unwrap_or(0o022)
}

/// The session's terminals, each a helper process.
pub struct Service {
    sessions: HashMap<u64, Session>,
    next_id: u64,
    mailbox: crate::terminal_wait::Mailbox,
    program: std::path::PathBuf,
}

impl Default for Service {
    fn default() -> Self { Self::new() }
}

impl Service {
    /// A service whose helpers run the host binary: this process's own
    /// executable, or for a process built beside it (a test) the
    /// `romacs-vm-gui` in its directory or the one above.
    #[must_use]
    pub fn new() -> Self {
        let own = std::env::current_exe().unwrap_or_default();
        let beside = |dir: Option<&std::path::Path>| dir.map(|dir| dir.join("romacs-vm-gui")).filter(|host| host.is_file());
        let host = if own.file_name().is_some_and(|name| name == "romacs-vm-gui") { None } else {
            beside(own.parent()).or_else(|| beside(own.parent().and_then(std::path::Path::parent)))
        };
        Self::with_program(host.unwrap_or(own))
    }

    /// A service whose helpers run PROGRAM, for a process that is not the
    /// host binary itself (a test).
    #[must_use]
    pub fn with_program(program: impl Into<std::path::PathBuf>) -> Self {
        Self { sessions: HashMap::new(), next_id: 0, mailbox: crate::terminal_wait::Mailbox::default(), program: program.into() }
    }

    /// Wait for native changes without polling a terminal or its history.
    #[must_use]
    pub fn ready(&self) -> crossbeam_channel::Receiver<()> { self.mailbox.ready() }

    /// Wake the same wait from another thread, for native work that is not a terminal's.
    #[must_use]
    pub fn waker(&self) -> std::sync::Arc<dyn Fn() + Send + Sync> {
        let mailbox = self.mailbox.clone();
        std::sync::Arc::new(move || mailbox.wake())
    }

    fn session(&mut self, id: u64) -> Result<&mut Session, String> {
        self.sessions.get_mut(&id).ok_or_else(|| format!("Unknown terminal session {id}"))
    }

    /// Start a conversation with the helper behind CONTROL at this size.
    fn attach(&mut self, control: UnixStream, columns: u16, rows: u16, child: Option<Owned>) -> Result<u64, String> {
        if columns < 2 || rows == 0 { return Err("Terminal dimensions require at least two columns and one row".into()); }
        let gone = |_| "The terminal's helper process is gone".to_string();
        let (ours, theirs) = UnixStream::pair().map_err(|e| e.to_string())?;
        channel::send_all(&control, b"c", &[theirs.as_fd()]).map_err(gone)?;
        drop(theirs);
        let mut wire = Vec::new();
        Hello { epoch: 0, pid: std::process::id(), classes: vec![proto::session_class()] }.record().encode_into(&mut wire);
        (&ours).write_all(&wire).map_err(gone)?;
        // The helper's hello: who it is, and whether this session can read it.
        ours.set_read_timeout(Some(std::time::Duration::from_secs(10))).map_err(|e| e.to_string())?;
        let (mut decoder, mut buffer) = (Decoder::default(), [0u8; 4096]);
        let hello = loop {
            if let Some(record) = decoder.pop().map_err(|e| e.to_string())? { break record; }
            match channel::receive(&ours, &mut buffer, &mut Vec::new()) {
                Ok(count @ 1..) => decoder.push(&buffer[..count]),
                _ => return Err("The terminal's helper process did not answer".into()),
            }
        };
        let helper = Hello::read(&hello).map_err(|e| format!("The terminal's helper process: {e}"))?;
        romacs_record::hello::agree(&proto::session_class(), &helper.classes).version()
            .map_err(|why| format!("This session cannot read the terminal's helper (pid {}): {why}", helper.pid))?;
        ours.set_read_timeout(None).map_err(|e| e.to_string())?;
        send(&ours, kind::OPEN, &Writer::default().u16(columns).u16(rows).finish())?;
        let id = self.next_id.checked_add(1).ok_or("Terminal session identifiers exhausted")?;
        let state = Push { columns, rows, running: true, ..Push::default() };
        let mirror = Mirror { lines: VecDeque::from(vec![(Row::new(), None); usize::from(rows)]), state, colors: Box::new([None; proto::COLORS]) };
        let shared = Arc::new(Mutex::new(Shared { mirror, unacked: None, dirty: true, gone: false, fulls: 0, deltas: 0 }));
        listen(id, ours.try_clone().map_err(|e| e.to_string())?, decoder, shared.clone(), self.mailbox.clone()).map_err(|e| e.to_string())?;
        self.sessions.insert(id, Session { stream: ours, _control: control, shared, _child: child, last: None });
        self.next_id = id;
        self.mailbox.mark(id);
        Ok(id)
    }

}

/// See a helper this session spawned itself to its end.
fn reap(child: spawn::Child) {
    std::thread::spawn(move || {
        for _ in 0..40 {
            if spawn::wait(spawn::Whom::Child(&child.pidfd)).is_some_and(|(_, status)| status.ended()) { return; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = spawn::signal(&child.pidfd, None, libc::SIGKILL);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = spawn::wait(spawn::Whom::Child(&child.pidfd));
    });
}

impl TerminalHost for Service {
    /// A terminal whose helper this session spawns itself and so takes
    /// along when it ends: the path of a session without a keeper.
    fn create(&mut self, shell: &str, directory: &str, columns: u16, rows: u16) -> Result<u64, String> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        if columns < 2 || rows == 0 { return Err("Terminal dimensions require at least two columns and one row".into()); }
        let env = std::env::vars_os().map(|(name, value)| [name.into_vec(), b"=".to_vec(), value.into_vec()].concat()).collect();
        let spec = Spec { argv: vec![self.program.as_os_str().as_bytes().to_vec(), b"--terminal-helper".to_vec(), shell.as_bytes().to_vec()],
                          env, cwd: directory.as_bytes().to_vec(), umask: umask(), setsid: true,
                          fds: vec![(0, End::Null), (1, End::Null), (2, End::Null), (3, End::Socket)], ..Spec::default() };
        let mut child = spawn::spawn(&spec).map_err(|e| format!("Cannot start the terminal's helper {}: {e}", self.program.display()))?;
        let control = UnixStream::from(child.fds.remove(0));
        self.attach(control, columns, rows, Some(Owned(Some(child))))
    }

    fn adopt(&mut self, control: OwnedFd, columns: u16, rows: u16) -> Result<u64, String> {
        self.attach(UnixStream::from(control), columns, rows, None)
    }

    fn send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String> {
        let session = self.session(session)?;
        if !session.shared.lock().unwrap().mirror.state.running { return Err("Terminal session has exited".into()); }
        if bytes.is_empty() { Ok(()) } else { send(&session.stream, kind::INPUT, bytes) }
    }

    fn resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String> {
        if columns < 2 || rows == 0 { return Err("Terminal dimensions require at least two columns and one row".into()); }
        let session = self.session(session)?;
        if !session.shared.lock().unwrap().mirror.state.running { return Ok(()); }
        let _ = send(&session.stream, kind::OPEN, &Writer::default().u16(columns).u16(rows).finish());
        Ok(())
    }

    fn input_state(&mut self, session: u64) -> Result<TerminalInputState, String> {
        let shared = self.session(session)?.shared.lock().unwrap();
        let (state, mode) = (&shared.mirror.state, TermMode::from_bits_truncate(shared.mirror.state.mode));
        Ok(TerminalInputState { running: state.running, application_cursor: mode.contains(TermMode::APP_CURSOR), bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
                                alternate_screen: mode.contains(TermMode::ALT_SCREEN), shell_foreground: state.running && state.shell_foreground })
    }

    fn changed_sessions(&mut self) -> Vec<u64> { self.mailbox.take() }

    fn snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String> {
        let session = self.session(session)?;
        let mut shared = session.shared.lock().unwrap();
        if let (false, Some(last)) = (shared.dirty, &session.last) { return Ok(last.clone()); }
        shared.dirty = false;
        let revision = session.last.as_ref().map_or(1, |last| last.revision);
        let mut snapshot = shared.mirror.snapshot(revision);
        // The helper may send its next push: this one is on its way to the buffer.
        if shared.unacked.take().is_some() && !shared.gone { let _ = send(&session.stream, kind::ACK, &[]); }
        drop(shared);
        if let Some(last) = &session.last {
            if *last == snapshot { return Ok(last.clone()); }
            snapshot.revision = snapshot.revision.saturating_add(1);
        }
        session.last = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn transport_counts(&mut self, session: u64) -> Option<(u64, u64)> {
        let shared = self.sessions.get(&session)?.shared.lock().unwrap();
        Some((shared.fulls, shared.deltas))
    }

    fn close(&mut self, session: u64) -> Result<(), String> {
        let session = self.sessions.remove(&session).ok_or_else(|| format!("Unknown terminal session {session}"))?;
        let _ = send(&session.stream, kind::CLOSE, &[]);
        Ok(())
    }
}
