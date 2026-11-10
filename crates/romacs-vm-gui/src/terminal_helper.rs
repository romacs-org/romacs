//! A terminal's helper process: `romacs-vm-gui --terminal-helper SHELL`.
//!
//! One process per terminal, for the terminal's whole life: the pty, the
//! shell (its child: Alacritty forks it), the emulator and its reader
//! thread. It is spawned through the session's keeper, or by a session
//! without one, with one descriptor of its own: 3, a control socket.
//!
//! A session attaches by passing one end of a fresh socketpair over the
//! control socket. That conversation carries records (`terminal_proto.rs`);
//! the newest conversation replaces any older one, so a session that died
//! in the middle of a record leaves nothing its successor could misread.
//! With no conversation the helper goes on reading the pty and emulating:
//! the shell never blocks on a session that is away, and what it printed is
//! in the grid when one comes.
//!
//! The end of the control socket is the helper's end: every holder of the
//! other side (the keeper, or the one session) has closed it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, mpsc};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use alacritty_terminal::{Term, tty};
use romacs_record::{Decoder, Hello, Reader, channel, record};

use crate::terminal_proto::{self as proto, Cell, Push, Row, kind};

/// The control socket's descriptor in a helper.
pub const CONTROL: i32 = 3;

/// A flag bit Alacritty does not define, set on the first cell of the
/// newest history line at each push. Lines enter history only below it and
/// history is never rewritten, so its distance from the bottom at the next
/// push is exactly how many lines scrolled in. It is never sent.
const MARK: u16 = 1 << 15;

/// What wakes the helper's one loop.
enum Wake {
    /// The emulator has news.
    Terminal,
    Conversation(UnixStream),
    Record(u64, record::Record),
    Gone(u64),
    ControlClosed,
}

#[derive(Clone)]
struct Listener(mpsc::Sender<Event>, crossbeam_channel::Sender<Wake>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
        let _ = self.1.send(Wake::Terminal);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Size(u16, u16);

impl Dimensions for Size {
    fn total_lines(&self) -> usize { usize::from(self.1) }
    fn screen_lines(&self) -> usize { usize::from(self.1) }
    fn columns(&self) -> usize { usize::from(self.0) }
}

impl Size {
    fn window(self) -> WindowSize { WindowSize { num_cols: self.0, num_lines: self.1, cell_width: 1, cell_height: 1 } }

    fn checked(columns: u16, rows: u16) -> Result<Self, String> {
        if columns < 2 || rows == 0 { Err("Terminal dimensions require at least two columns and one row".into()) } else { Ok(Self(columns, rows)) }
    }
}

unsafe extern "C" {
    fn pidfd_open(pid: i32, flags: u32) -> i32;
    fn pidfd_send_signal(fd: i32, signal: i32, info: *const std::ffi::c_void, flags: u32) -> i32;
    fn tcgetpgrp(fd: i32) -> i32;
}

/// The pty, the shell and the emulator.
struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    channel: EventLoopSender,
    events: mpsc::Receiver<Event>,
    finished: mpsc::Receiver<()>,
    running: bool,
    size: Size,
    child: Option<OwnedFd>,
    /// A duplicate of the pty's master, and the shell's pid.
    foreground: Option<(std::fs::File, i32)>,
    /// What the last push described, to tell what the next must carry.
    pushed: Option<Pushed>,
}

struct Pushed {
    size: Size,
    alternate: bool,
    /// Whether a history line was marked.
    marked: bool,
}

/// A history line by its distance above the screen.
fn back(lines: usize) -> Line { Line(-i32::try_from(lines).unwrap_or(i32::MAX)) }

fn narrow<T: TryFrom<usize> + Default>(value: usize) -> T { T::try_from(value).unwrap_or_default() }

/// What a program is told a colour is, when it asks and set none itself.
fn default_rgb(index: usize) -> Rgb {
    let [r, g, b] = proto::default_color(index);
    Rgb { r, g, b }
}

/// A grid line as it is sent: cells up to the last that is not a default
/// blank, without the mark.
fn row_of(row: &alacritty_terminal::grid::Row<alacritty_terminal::term::cell::Cell>, columns: usize) -> Row {
    let plain = |cell: &alacritty_terminal::term::cell::Cell| {
        cell.c == ' ' && cell.fg == Color::Named(NamedColor::Foreground) && cell.bg == Color::Named(NamedColor::Background)
            && cell.flags.bits() & !MARK == 0 && cell.zerowidth().is_none()
    };
    let mut end = columns;
    while end > 0 && plain(&row[Column(end - 1)]) { end -= 1; }
    (0..end).map(|column| {
        let cell = &row[Column(column)];
        Cell { c: cell.c, zero_width: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(), fg: cell.fg, bg: cell.bg,
               flags: Flags::from_bits_truncate(cell.flags.bits() & !MARK) }
    }).collect()
}

impl Terminal {
    fn start(shell: &str, size: Size, wake: crossbeam_channel::Sender<Wake>) -> Result<Self, String> {
        let (events_tx, events) = mpsc::channel();
        let listener = Listener(events_tx, wake.clone());
        let term = Arc::new(FairMutex::new(Term::new(Config { scrolling_history: 10_000, ..Config::default() }, &size, listener.clone())));
        let mut env = HashMap::new();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        let shell = if shell.is_empty() {
            std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into())
        } else { shell.into() };
        let options = tty::Options { shell: Some(tty::Shell::new(shell, vec!["-i".into()])), working_directory: std::env::current_dir().ok(),
                                     drain_on_exit: true, env };
        let pty = tty::new(&options, size.window(), 0).map_err(|e| e.to_string())?;
        let shell_pid = pty.child().id().cast_signed();
        let foreground = pty.file().try_clone().ok().map(|master| (master, shell_pid));
        // SAFETY: pidfd_open takes a scalar child PID and zero flags, with no pointers.
        let fd = unsafe { pidfd_open(shell_pid, 0) };
        if fd < 0 { return Err(std::io::Error::last_os_error().to_string()); }
        // SAFETY: successful pidfd_open returned a new descriptor owned by this scope.
        let child = unsafe { OwnedFd::from_raw_fd(fd) };
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false).map_err(|e| e.to_string())?;
        let channel = event_loop.channel();
        let handle = event_loop.spawn();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            drop(handle.join());
            let _ = done.send(());
            let _ = wake.send(Wake::Terminal);
        });
        Ok(Self { term, channel, events, finished, running: true, size, child: Some(child), foreground, pushed: None })
    }

    fn input(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() { let _ = self.channel.send(Msg::Input(Cow::Owned(bytes))); }
    }

    /// Answer what the program asked the terminal, and note the shell's end.
    fn events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::PtyWrite(text) => self.input(text.into_bytes()),
                Event::TextAreaSizeRequest(format) => self.input(format(self.size.window()).into_bytes()),
                Event::ColorRequest(index, format) => {
                    let color = {
                        let term = self.term.lock();
                        if index < proto::COLORS { term.colors()[index].unwrap_or_else(|| default_rgb(index)) } else { default_rgb(index) }
                    };
                    self.input(format(color).into_bytes());
                }
                Event::Exit => self.running = false,
                _ => {}
            }
        }
        if self.finished.try_recv().is_ok() { self.running = false; }
    }

    fn resize(&mut self, size: Size) {
        if !self.running || size == self.size { return; }
        let mut term = self.term.lock();
        if self.channel.send(Msg::Resize(size.window())).is_ok() {
            term.resize(size);
            self.size = size;
        }
    }

    fn shell_foreground(&self) -> bool {
        use std::os::fd::AsRawFd;
        // SAFETY: tcgetpgrp takes a descriptor this file owns and no pointers.
        self.running && self.foreground.as_ref().is_some_and(|(master, pid)| unsafe { tcgetpgrp(master.as_raw_fd()) } == *pid)
    }

    /// The push that brings a session from the last push to now: the lines
    /// that scrolled into history and the screen, or with FULL (and
    /// whenever the mark cannot vouch for a delta) every line.
    fn collect(&mut self, full: bool) -> Push {
        let shell_foreground = self.shell_foreground();
        let mut term = self.term.lock();
        let mode = *term.mode();
        let alternate = mode.contains(TermMode::ALT_SCREEN);
        let colors = (0..proto::COLORS).filter_map(|i| term.colors()[i].map(|rgb| (narrow(i), [rgb.r, rgb.g, rgb.b]))).collect();
        let grid = term.grid_mut();
        let (columns, rows, history) = (grid.columns(), grid.screen_lines(), grid.history_size());
        let marked = |grid: &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell>, lines: usize| grid[back(lines)][Column(0)].flags.bits() & MARK != 0;
        // How many lines entered history since the last push, if that can be
        // known: the old mark's distance from the bottom, or all of history
        // when there was none to mark.
        let same = self.pushed.as_ref().filter(|p| !full && p.size == self.size && p.alternate == alternate && Flags::all().bits() & MARK == 0);
        let scrolled = same.and_then(|p| if p.marked { (1..=history).find(|lines| marked(grid, *lines)).map(|lines| lines - 1) } else { Some(history) });
        let first = scrolled.map_or(history, |scrolled| scrolled.min(history));
        let lines = (back(first).0..narrow(rows)).map(|line| (line, row_of(&grid[Line(line)], columns))).collect();
        // Move the mark to the newest history line.
        let stale = match scrolled { Some(scrolled) => scrolled + 1..scrolled + 2, None => 1..history + 1 };
        for lines in stale.filter(|lines| *lines <= history) {
            let flags = &mut grid[back(lines)][Column(0)].flags;
            *flags = Flags::from_bits_retain(flags.bits() & !MARK);
        }
        if history > 0 {
            let flags = &mut grid[Line(-1)][Column(0)].flags;
            *flags = Flags::from_bits_retain(flags.bits() | MARK);
        }
        self.pushed = Some(Pushed { size: self.size, alternate, marked: history > 0 });
        let cursor = grid.cursor.point;
        Push { revision: 0, scrolled: scrolled.map(narrow), columns: self.size.0, rows: self.size.1,
               history: narrow(history), cursor_line: u16::try_from(cursor.line.0).unwrap_or(0), cursor_column: narrow(cursor.column.0),
               cursor_wraps: grid.cursor.input_needs_wrap, mode: mode.bits(), running: self.running, shell_foreground, colors, lines }
    }

    /// Hang the shell up, and kill it if it stays.
    fn shutdown(&mut self) {
        use std::os::fd::AsRawFd;
        let Some(child) = self.child.take() else { return };
        // SAFETY: the owned pidfd targets this child without PID reuse; null siginfo and
        // zero flags pass no userspace memory. HUP lets the shell retire its own jobs.
        unsafe { pidfd_send_signal(child.as_raw_fd(), 1, std::ptr::null(), 0); }
        if self.finished.recv_timeout(std::time::Duration::from_millis(250)).is_err() {
            // SAFETY: the pidfd is still owned here, so escalation cannot
            // target a later process even if the child has already exited.
            unsafe { pidfd_send_signal(child.as_raw_fd(), 9, std::ptr::null(), 0); }
            let _ = self.channel.send(Msg::Shutdown);
            let _ = self.finished.recv_timeout(std::time::Duration::from_millis(500));
        }
    }
}

/// The session now talking to this helper.
struct Conversation {
    id: u64,
    stream: UnixStream,
    greeted: bool,
    /// Whether the session has applied the last push.
    acked: bool,
    /// Whether the next push must carry every line.
    full: bool,
}

fn send(stream: &mut UnixStream, kind: u16, payload: &[u8]) -> bool {
    let mut wire = Vec::new();
    record::encode(0, proto::CLASS, kind, payload, &mut wire);
    stream.write_all(&wire).is_ok()
}

/// Read a conversation's records into the loop until it ends.
fn listen(id: u64, stream: UnixStream, wake: crossbeam_channel::Sender<Wake>) {
    std::thread::spawn(move || {
        let (mut decoder, mut buffer) = (Decoder::default(), vec![0u8; 1 << 16]);
        'read: while let Ok(count @ 1..) = channel::receive(&stream, &mut buffer, &mut Vec::new()) {
            decoder.push(&buffer[..count]);
            loop {
                match decoder.pop() {
                    Ok(Some(record)) => if wake.send(Wake::Record(id, record)).is_err() { break 'read },
                    Ok(None) => break,
                    Err(_) => break 'read,
                }
            }
        }
        let _ = wake.send(Wake::Gone(id));
    });
}

/// Run a helper for SHELL (empty for the user's) until its control socket
/// closes or a session closes the terminal.
///
/// # Errors
/// No control socket at descriptor 3.
pub fn run(shell: &str) -> Result<(), String> {
    // SAFETY: a helper is started with its control socket at descriptor 3
    // (`CONTROL`), which nothing else in this process owns.
    let control = UnixStream::from(unsafe { OwnedFd::from_raw_fd(CONTROL) });
    control.as_fd().try_clone_to_owned().map_err(|_| "a terminal helper needs its control socket at descriptor 3")?;
    let (wake, woken) = crossbeam_channel::unbounded();
    // The newest conversation's stream, so a replaced one can be cut even
    // while the loop is writing to it.
    let current: Arc<Mutex<Option<UnixStream>>> = Arc::default();
    {
        let (wake, current) = (wake.clone(), current.clone());
        std::thread::spawn(move || {
            let mut byte = [0u8; 16];
            loop {
                let mut fds = Vec::new();
                match channel::receive(&control, &mut byte, &mut fds) {
                    Ok(1..) => for fd in fds {
                        let stream = UnixStream::from(fd);
                        let Ok(newest) = stream.try_clone() else { continue };
                        if let Some(old) = current.lock().ok().and_then(|mut current| current.replace(newest)) {
                            let _ = old.shutdown(std::net::Shutdown::Both);
                        }
                        let _ = wake.send(Wake::Conversation(stream));
                    },
                    _ => break,
                }
            }
            // The loop may be writing to a session that does not read: cut
            // that, so the end is seen and the shell hung up.
            if let Some(stream) = current.lock().ok().and_then(|mut current| current.take()) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            let _ = wake.send(Wake::ControlClosed);
        });
    }
    let (mut terminal, mut conversation, mut conversations, mut revision) = (None::<Terminal>, None::<Conversation>, 0u64, 0u64);
    let (mut dirty, mut last, mut foreground) = (false, Vec::new(), false);
    loop {
        match woken.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Wake::Terminal) => dirty = true,
            Ok(Wake::Conversation(mut stream)) => {
                conversations += 1;
                let mut wire = Vec::new();
                Hello { epoch: 0, pid: std::process::id(), classes: vec![proto::helper_class()] }.record().encode_into(&mut wire);
                if stream.write_all(&wire).is_ok() && let Ok(reader) = stream.try_clone() {
                    listen(conversations, reader, wake.clone());
                    conversation = Some(Conversation { id: conversations, stream, greeted: false, acked: true, full: true });
                }
            }
            Ok(Wake::Record(id, record)) => if let Some(talk) = conversation.as_mut().filter(|c| c.id == id) {
                let mut r = Reader(&record.payload);
                match record.kind {
                    _ if !talk.greeted => {
                        // A session of another framing, or one that does not
                        // speak this helper's version, is not talked to.
                        talk.greeted = Hello::read(&record).is_ok_and(|theirs| romacs_record::hello::agree(&proto::helper_class(), &theirs.classes).version().is_ok());
                        if !talk.greeted { conversation = None; }
                    }
                    kind::OPEN => if let (Ok(columns), Ok(rows)) = (r.u16(), r.u16()) && let Ok(size) = Size::checked(columns, rows) {
                        match &mut terminal {
                            Some(terminal) => terminal.resize(size),
                            None => match Terminal::start(shell, size, wake.clone()) {
                                Ok(started) => terminal = Some(started),
                                // No terminal to speak of: the helper's end
                                // is what tells the session, and the keeper.
                                Err(error) => return Err(format!("Cannot start {shell}: {error}")),
                            },
                        }
                        (talk.full, dirty) = (true, true);
                    },
                    kind::INPUT => if let Some(terminal) = &terminal { terminal.input(record.payload.clone()); dirty = true; },
                    kind::ACK => { talk.acked = true; dirty = true; }
                    kind::CLOSE => break,
                    _ => {}
                }
            },
            Ok(Wake::Gone(id)) => if conversation.as_ref().is_some_and(|c| c.id == id) { conversation = None; },
            Ok(Wake::ControlClosed) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        let Some(terminal) = &mut terminal else { continue };
        terminal.events();
        // Who has the terminal changes without a byte of output.
        let now = terminal.shell_foreground();
        dirty |= std::mem::replace(&mut foreground, now) != now;
        if let Some(talk) = conversation.as_mut().filter(|c| c.greeted && c.acked && dirty) {
            dirty = false;
            let mut push = terminal.collect(talk.full);
            let body = push.encode();
            // Nothing new to say. Only a delta that scrolled nothing can be
            // dropped: any other has moved the mark, and the session must follow.
            if push.scrolled == Some(0) && body == last { continue; }
            revision += 1;
            push.revision = revision;
            let bytes = push.encode();
            last = body;
            let mut chunks = bytes.chunks(1 << 20).peekable();
            let mut sent = true;
            while let Some(chunk) = chunks.next() {
                sent &= send(&mut talk.stream, if chunks.peek().is_some() { kind::PART } else { kind::PUSH }, chunk);
            }
            (talk.acked, talk.full) = (false, false);
            if !sent { conversation = None; }
        }
    }
    if let Some(terminal) = &mut terminal { terminal.shutdown(); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mark is sound only while Alacritty leaves its bit undefined. A
    /// pin that defines it fails here; until someone looks, `collect`
    /// sends every line, since it checks the same thing.
    #[test]
    fn the_mark_is_a_bit_alacritty_does_not_define() {
        assert_eq!(Flags::all().bits() & MARK, 0);
        assert_ne!(MARK, 0);
    }
}
