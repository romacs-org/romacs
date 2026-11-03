use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, TermMode, cell::Flags};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use alacritty_terminal::{Term, tty};
use romacs_vm::{TerminalColor, TerminalHost, TerminalInputState, TerminalSnapshot, TerminalStyle, TerminalStyleRun};

#[derive(Clone)]
struct Listener { events: mpsc::Sender<Event>, mailbox: crate::terminal_wait::Mailbox, owner: u64 }

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.events.send(event);
        self.mailbox.mark(self.owner);
    }
}

#[derive(Clone, Copy)]
struct Size(u16, u16);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        usize::from(self.1)
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.1)
    }
    fn columns(&self) -> usize {
        usize::from(self.0)
    }
}

impl Size {
    fn window(self) -> WindowSize {
        WindowSize {
            num_cols: self.0,
            num_lines: self.1,
            cell_width: 1,
            cell_height: 1,
        }
    }

    fn checked(columns: u16, rows: u16) -> Result<Self, String> {
        if columns < 2 || rows == 0 {
            Err("Terminal dimensions require at least two columns and one row".into())
        } else {
            Ok(Self(columns, rows))
        }
    }
}

struct Session {
    terminal: Arc<FairMutex<Term<Listener>>>,
    channel: EventLoopSender,
    events: mpsc::Receiver<Event>,
    finished: mpsc::Receiver<()>,
    running: bool,
    size: Size,
    last: Option<TerminalSnapshot>,
    dirty: bool,
    #[cfg(target_os = "linux")]
    child: Option<std::os::fd::OwnedFd>,
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn pidfd_open(pid: i32, flags: u32) -> i32;
    fn pidfd_send_signal(fd: i32, signal: i32, info: *const std::ffi::c_void, flags: u32) -> i32;
}

impl Session {
    fn input(&self, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.channel
            .send(Msg::Input(Cow::Owned(bytes)))
            .map_err(|e| e.to_string())
    }

    fn events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::PtyWrite(text) => {
                    let _ = self.input(text.into_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    let _ = self.input(format(self.size.window()).into_bytes());
                }
                Event::ColorRequest(index, format) => {
                    let color = {
                        let terminal = self.terminal.lock();
                        if index < alacritty_terminal::term::color::COUNT {
                            terminal.colors()[index].unwrap_or_else(|| default_color(index))
                        } else {
                            default_color(index)
                        }
                    };
                    let _ = self.input(format(color).into_bytes());
                }
                Event::Wakeup => self.dirty = true,
                Event::Exit => { self.running = false; self.dirty = true; }
                _ => {}
            }
        }
        if self.finished.try_recv().is_ok() {
            self.running = false;
            self.dirty = true;
        }
    }

    fn publication(&mut self) -> TerminalSnapshot {
        self.events();
        if !self.dirty && let Some(last) = &self.last { return last.clone(); }
        self.dirty = false;
        let terminal = self.terminal.lock();
        let grid = terminal.grid();
        let cursor = grid.cursor.point;
        let mut text = String::new();
        let mut styles: Vec<TerminalStyleRun> = Vec::new();
        let mut cursor_byte = 0;
        let mut screen_start_byte = 0;
        for line in grid.topmost_line().0..=grid.bottommost_line().0 {
            if line == 0 {
                screen_start_byte = text.len();
            }
            let row = &grid[Line(line)];
            let wrapped = row[Column(grid.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
            let mut end = grid.columns();
            if !wrapped {
                while end > 0 {
                    let cell = &row[Column(end - 1)];
                    if cell.c != ' ' || cell.zerowidth().is_some()
                        || cell.bg != Color::Named(NamedColor::Background) || cell.flags.contains(Flags::INVERSE) {
                        break;
                    }
                    end -= 1;
                }
                if line == cursor.line.0 {
                    end = end.max(cursor.column.0 + usize::from(grid.cursor.input_needs_wrap));
                }
            }
            for column in 0..end {
                if line == cursor.line.0 && column == cursor.column.0 {
                    cursor_byte = text.len();
                }
                let cell = &row[Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let start = text.len();
                text.push(if cell.c == '\t' { ' ' } else { cell.c });
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra.iter().copied());
                }
                let style = cell_style(cell, terminal.colors());
                if style != TerminalStyle::default() {
                    if let Some(last) = styles.last_mut() && last.bytes.end == start && last.style == style {
                        last.bytes.end = text.len();
                    } else {
                        styles.push(TerminalStyleRun { bytes: start..text.len(), style });
                    }
                }
            }
            if line == cursor.line.0 && (cursor.column.0 >= end || grid.cursor.input_needs_wrap) {
                cursor_byte = text.len();
            }
            if !wrapped && line != grid.bottommost_line().0 {
                text.push('\n');
            }
        }
        let mode = terminal.mode();
        let mut snapshot = TerminalSnapshot {
            revision: self.last.as_ref().map_or(1, |last| last.revision),
            text: text.into(),
            styles: styles.into(),
            default_colors: [terminal.colors()[NamedColor::Foreground], terminal.colors()[NamedColor::Background]].map(|color| color.map(|rgb| [rgb.r, rgb.g, rgb.b])),
            cursor_byte,
            screen_start_byte,
            columns: self.size.0,
            rows: self.size.1,
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            running: self.running,
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        };
        if let Some(last) = &self.last {
            if last == &snapshot { return last.clone(); }
            snapshot.revision = snapshot.revision.saturating_add(1);
        }
        self.last = Some(snapshot.clone());
        snapshot
    }
}

impl Session {
    #[cfg(target_os = "linux")]
    fn shutdown(&mut self) -> Option<mpsc::Receiver<()>> {
        use std::os::fd::AsRawFd;
        let child = self.child.take()?;
        // SAFETY: the owned pidfd targets this child without PID reuse; null siginfo and
        // zero flags pass no userspace memory. HUP lets the shell retire its own jobs.
        unsafe { pidfd_send_signal(child.as_raw_fd(), 1, std::ptr::null(), 0); }
        let finished = std::mem::replace(&mut self.finished, mpsc::channel().1);
        let channel = self.channel.clone();
        let (done, reaped) = mpsc::channel();
        std::thread::spawn(move || {
            if finished.recv_timeout(std::time::Duration::from_millis(250)).is_err() {
                // SAFETY: the pidfd remains owned until this waiter ends, so escalation
                // cannot target a later process even if the child has already exited.
                unsafe { pidfd_send_signal(child.as_raw_fd(), 9, std::ptr::null(), 0); }
                let _ = channel.send(Msg::Shutdown);
                let _ = finished.recv_timeout(std::time::Duration::from_millis(500));
            }
            let _ = done.send(());
        });
        Some(reaped)
    }

    #[cfg(not(target_os = "linux"))]
    fn shutdown(&mut self) -> Option<mpsc::Receiver<()>> {
        let _ = self.channel.send(Msg::Shutdown);
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) { let _ = self.shutdown(); }
}

fn terminal_color(color: Color, colors: &alacritty_terminal::term::color::Colors) -> TerminalColor {
    match color {
        Color::Spec(rgb) => TerminalColor::Rgb([rgb.r, rgb.g, rgb.b]),
        Color::Named(NamedColor::Foreground) => TerminalColor::Foreground,
        Color::Named(NamedColor::Background) => TerminalColor::Background,
        Color::Named(NamedColor::DimForeground) if colors[NamedColor::DimForeground].is_none() => TerminalColor::DimForeground,
        Color::Named(NamedColor::BrightForeground) if colors[NamedColor::BrightForeground].is_none() => TerminalColor::Foreground,
        Color::Named(name) => indexed_color(name as usize, colors),
        Color::Indexed(index) => indexed_color(usize::from(index), colors),
    }
}

/// The colour of slot INDEX: the session's own where it set one, else the
/// theme's for the sixteen base colours and their dim forms, else literal.
fn indexed_color(index: usize, colors: &alacritty_terminal::term::color::Colors) -> TerminalColor {
    let rgb = |rgb: Rgb| TerminalColor::Rgb([rgb.r, rgb.g, rgb.b]);
    let dim = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
    match colors[index] {
        Some(set) => rgb(set),
        None if index < 16 => TerminalColor::Ansi(index as u8),
        None if dim.contains(&index) => {
            let base = index - NamedColor::DimBlack as usize;
            match colors[base] {
                Some(set) => TerminalColor::Rgb([set.r, set.g, set.b].map(|v| (f32::from(v) * 0.66) as u8)),
                None => TerminalColor::DimAnsi(base as u8),
            }
        }
        None => rgb(default_color(index)),
    }
}

fn cell_style(cell: &alacritty_terminal::term::cell::Cell, colors: &alacritty_terminal::term::color::Colors) -> TerminalStyle {
    let foreground = match cell.fg {
        Color::Spec(rgb) if cell.flags.contains(Flags::DIM) => {
            let rgb = [rgb.r, rgb.g, rgb.b].map(|v| (f32::from(v) * 0.66) as u8);
            TerminalColor::Rgb(rgb)
        }
        Color::Named(name) if (cell.flags & Flags::DIM_BOLD).intersects(Flags::DIM) => terminal_color(Color::Named(name.to_dim()), colors),
        Color::Indexed(index) if cell.flags & Flags::DIM_BOLD == Flags::DIM && index < 8 => {
            indexed_color(NamedColor::DimBlack as usize + usize::from(index), colors)
        }
        Color::Indexed(index) if cell.flags & Flags::DIM_BOLD == Flags::DIM && index < 16 => terminal_color(Color::Indexed(index - 8), colors),
        color => terminal_color(color, colors),
    };
    TerminalStyle { foreground, background: terminal_color(cell.bg, colors), inverse: cell.flags.contains(Flags::INVERSE), hidden: cell.flags.contains(Flags::HIDDEN) }
}

/// The colour of INDEX where the theme does not give it: the 256-colour
/// cube and greys, and Alacritty's defaults otherwise.
fn default_color(index: usize) -> Rgb {
    const ANSI: [[u8; 3]; 16] = romacs_vm::DEFAULT_ANSI;
    let [r, g, b] = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let value = index - 16;
            let component = |n: usize| if n == 0 { 0 } else { (55 + 40 * n) as u8 };
            [
                component(value / 36),
                component(value / 6 % 6),
                component(value % 6),
            ]
        }
        232..=255 => [((index - 232) * 10 + 8) as u8; 3],
        index if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&index) => ANSI[index - NamedColor::DimBlack as usize].map(|v| (f32::from(v) * 0.66) as u8),
        257 => ANSI[0],
        _ => ANSI[7],
    };
    Rgb { r, g, b }
}

/// Owns independent PTY sessions backed by the upstream Alacritty event loop.
#[derive(Default)]
pub struct Service {
    sessions: HashMap<u64, Session>,
    next_id: u64,
    reapers: Vec<mpsc::Receiver<()>>,
    mailbox: crate::terminal_wait::Mailbox,
}

impl Service {
    /// Construct a service without starting a shell or changing process environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wait for native changes without polling a terminal or its history.
    pub fn ready(&self) -> crossbeam_channel::Receiver<()> { self.mailbox.ready() }

    /// Wake the same wait from another thread, for native work that is not a terminal's.
    pub fn waker(&self) -> std::sync::Arc<dyn Fn() + Send + Sync> {
        let mailbox = self.mailbox.clone();
        std::sync::Arc::new(move || mailbox.wake())
    }

    fn session(&mut self, id: u64) -> Result<&mut Session, String> {
        self.sessions
            .get_mut(&id)
            .ok_or_else(|| format!("Unknown terminal session {id}"))
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        for (_, mut session) in std::mem::take(&mut self.sessions) {
            if let Some(done) = session.shutdown() { self.reapers.push(done); }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        for done in self.reapers.drain(..) {
            let _ = done.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
        }
    }
}

impl TerminalHost for Service {
    fn create(
        &mut self,
        shell: &str,
        directory: &str,
        columns: u16,
        rows: u16,
    ) -> Result<u64, String> {
        let size = Size::checked(columns, rows)?;
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Terminal session identifiers exhausted")?;
        let (events_tx, events) = mpsc::channel();
        let listener = Listener { events: events_tx, mailbox: self.mailbox.clone(), owner: id };
        let terminal = Arc::new(FairMutex::new(Term::new(
            Config {
                scrolling_history: 10_000,
                ..Config::default()
            },
            &size,
            listener.clone(),
        )));
        let mut env = HashMap::new();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        let shell = if shell.is_empty() {
            std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into())
        } else { shell.into() };
        let options = tty::Options {
            shell: Some(tty::Shell::new(shell, vec!["-i".into()])),
            working_directory: Some(directory.into()),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, size.window(), 0).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        let child = {
            use std::os::fd::FromRawFd;
            // SAFETY: pidfd_open takes a scalar child PID and zero flags, with no pointers.
            let fd = unsafe { pidfd_open(pty.child().id() as i32, 0) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            // SAFETY: successful pidfd_open returned a new descriptor owned by this scope.
            unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }
        };
        let event_loop = EventLoop::new(terminal.clone(), listener, pty, true, false)
            .map_err(|e| e.to_string())?;
        let channel = event_loop.channel();
        let handle = event_loop.spawn();
        let (done, finished) = mpsc::channel();
        let mailbox = self.mailbox.clone();
        std::thread::spawn(move || {
            drop(handle.join());
            let _ = done.send(());
            mailbox.mark(id);
        });
        self.sessions.insert(
            id,
            Session {
                terminal,
                channel,
                events,
                finished,
                running: true,
                size,
                last: None,
                dirty: true,
                #[cfg(target_os = "linux")]
                child: Some(child),
            },
        );
        self.next_id = id;
        self.mailbox.mark(id);
        Ok(id)
    }

    fn send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String> {
        let session = self.session(session)?;
        session.events();
        if !session.running {
            return Err("Terminal session has exited".into());
        }
        session.input(bytes.to_vec())
    }

    fn resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String> {
        let size = Size::checked(columns, rows)?;
        let id = session;
        let session = self.session(session)?;
        session.events();
        if !session.running {
            return Ok(());
        }
        let mut terminal = session.terminal.lock();
        match session.channel.send(Msg::Resize(size.window())) {
            Ok(()) => {},
            Err(alacritty_terminal::event_loop::EventLoopSendError::Send(_)) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
        terminal.resize(size);
        session.size = size;
        session.dirty = true;
        drop(terminal);
        self.mailbox.mark(id);
        Ok(())
    }

    fn input_state(&mut self, session: u64) -> Result<TerminalInputState, String> {
        let session = self.session(session)?;
        session.events();
        let terminal = session.terminal.lock();
        let mode = terminal.mode();
        Ok(TerminalInputState { running: session.running, application_cursor: mode.contains(TermMode::APP_CURSOR), bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE) })
    }

    fn changed_sessions(&mut self) -> Vec<u64> { self.mailbox.take() }

    fn snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String> {
        Ok(self.session(session)?.publication())
    }

    fn close(&mut self, session: u64) -> Result<(), String> {
        let mut session = self.sessions.remove(&session)
            .ok_or_else(|| format!("Unknown terminal session {session}"))?;
        if let Some(done) = session.shutdown() { self.reapers.push(done); }
        self.reapers.retain(|done| matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
        Ok(())
    }
}
