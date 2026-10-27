//! GNU process.c and callproc.c operating-system mechanics: process objects'
//! native state, descriptors, spawning, reaping and polling. Nothing here
//! calls Lisp; the wait loop that runs filters, sentinels and timers is Lisp
//! (rlisp/vm/subprocess.el) over the leaf primitives in prims/process.rs.
use crate::Value;
use std::collections::BTreeMap;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// Lisp-visible slots of GNU's `struct Lisp_Process`, traced by the collector.
#[derive(Clone, Copy)]
pub(crate) enum Slot {
    Name,
    Command,
    Filter,
    Sentinel,
    Log,
    Buffer,
    Childp,
    Plist,
    Type,
    Mark,
    Status,
    Decode,
    Encode,
    Stderrproc,
    TtyName,
}
pub(crate) const SLOTS: usize = Slot::TtyName as usize + 1;

/// A process object. Its descriptors live in [`Table`] under `serial`, so a
/// moved or copied object never owns an OS resource.
#[derive(Clone)]
pub(crate) struct Process {
    pub slots: [Value; SLOTS],
    pub serial: u64,
    /// GNU p->pid: 0 for none, -2 for a pty-only process.
    pub pid: i64,
    pub alive: bool,
    pub tick: u64,
    pub update_tick: u64,
    /// GNU raw_status and raw_status_new: a wait status not yet converted.
    pub raw_status: Option<i32>,
    pub kill_without_query: bool,
    pub inherit_coding: bool,
    pub pty_in: bool,
    pub pty_out: bool,
    pub nbytes_read: u64,
    pub readmax: usize,
}

impl Process {
    pub(crate) fn new(serial: u64, readmax: usize) -> Self {
        Process {
            slots: [Value::NIL; SLOTS],
            serial,
            pid: 0,
            alive: false,
            tick: 0,
            update_tick: 0,
            raw_status: None,
            kill_without_query: false,
            inherit_coding: false,
            pty_in: false,
            pty_out: false,
            nbytes_read: 0,
            readmax,
        }
    }
    pub(crate) fn get(&self, slot: Slot) -> Value {
        self.slots[slot as usize]
    }
    pub(crate) fn set(&mut self, slot: Slot, value: Value) {
        self.slots[slot as usize] = value;
    }
    pub(crate) fn visit_values(&mut self, f: &mut dyn FnMut(&mut Value)) {
        self.slots.iter_mut().for_each(f);
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        size_of::<Process>()
    }
}

/// GNU's open_fd[] and infd/outfd for one process.
#[derive(Default)]
pub(crate) struct Channel {
    pub infd: Option<OwnedFd>,
    pub outfd: Option<OwnedFd>,
    /// open_fd[SUBPROCESS_STDIN]: a pipe process's unread input end.
    pub child_stdin: Option<OwnedFd>,
    /// open_fd[SUBPROCESS_STDOUT]: the write end a pipe process lends to a
    /// child as its stderr.
    pub child_stdout: Option<OwnedFd>,
    /// A Linux pidfd: readable when the child exits (GNU's SIGCHLD wakeup).
    pub pidfd: Option<OwnedFd>,
    /// The child's pid, also its process group (POSIX_SPAWN_SETSID).
    pub pid: i32,
    /// GNU decoding_buf: undecoded bytes held over to the next read.
    pub carryover: Vec<u8>,
    /// GNU write_queue: bytes not yet written.
    pub pending: Vec<u8>,
    /// add_process_read_fd: the wait polls `infd'.
    pub reading: bool,
    /// CODING_MODE_LAST_BLOCK: end of file was seen and the carryover flushed.
    pub last_block: bool,
}

#[derive(Default)]
pub(crate) struct Table {
    channels: BTreeMap<u64, Channel>,
    next: u64,
    /// GNU process_tick.
    pub tick: u64,
    /// GNU update_tick.
    pub update_tick: u64,
    /// GNU deleted_pid_list: killed children not yet reaped, with a file to
    /// unlink once they are.
    pub deleted: Vec<(i32, Option<std::path::PathBuf>)>,
    /// GNU last_read_channel, as a serial.
    pub last_read: u64,
}

impl Table {
    pub(crate) fn insert(&mut self, channel: Channel) -> u64 {
        self.next += 1;
        self.channels.insert(self.next, channel);
        self.next
    }
    pub(crate) fn get(&self, serial: u64) -> Option<&Channel> {
        self.channels.get(&serial)
    }
    pub(crate) fn get_mut(&mut self, serial: u64) -> Option<&mut Channel> {
        self.channels.get_mut(&serial)
    }
    /// deactivate_process: close every descriptor.
    pub(crate) fn remove(&mut self, serial: u64) -> Option<Channel> {
        self.channels.remove(&serial)
    }
    /// Reap killed children recorded by `delete-process' and `call-process'.
    pub(crate) fn reap_deleted(&mut self) {
        self.deleted.retain(|(pid, file)| {
            let mut status = 0;
            let done = unsafe { libc::waitpid(*pid, &mut status, libc::WNOHANG) } != 0;
            if done && let Some(file) = file {
                let _ = std::fs::remove_file(file);
            }
            !done
        });
    }
}

impl Drop for Table {
    /// shut_down_emacs's kill_buffer_processes (Qnil): SIGHUP to the group of
    /// each child not yet reaped (its pidfd still open, so its pid is not
    /// reused).
    fn drop(&mut self) {
        for channel in self.channels.values() {
            if channel.pidfd.is_some() && channel.pid > 0 {
                unsafe { libc::kill(-channel.pid, libc::SIGHUP) };
            }
        }
    }
}

pub(crate) fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// emacs_pipe: a close-on-exec pipe (read end, write end).
pub(crate) fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(last_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

pub(crate) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(last_error());
    }
    Ok(())
}

pub(crate) fn dup(fd: &OwnedFd) -> io::Result<OwnedFd> {
    fd.try_clone()
}

/// allocate_pty with posix_openpt: the master and the slave's name.
pub(crate) fn allocate_pty() -> io::Result<(OwnedFd, Vec<u8>)> {
    let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(last_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
        return Err(last_error());
    }
    let mut name = [0u8; 64];
    if unsafe { libc::ptsname_r(fd, name.as_mut_ptr().cast(), name.len()) } != 0 {
        return Err(last_error());
    }
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    Ok((master, name[..len].to_vec()))
}

pub(crate) fn open_tty(name: &[u8]) -> io::Result<OwnedFd> {
    let path = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(last_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// sysdep.c child_setup_tty: the terminal modes a pty child starts with.
pub(crate) fn child_setup_tty(fd: RawFd) {
    let mut s: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut s) } != 0 {
        return;
    }
    s.c_oflag |= libc::OPOST;
    s.c_oflag &= !libc::ONLCR;
    s.c_oflag &= !(libc::NLDLY | libc::CRDLY | libc::TABDLY | libc::BSDLY | libc::VTDLY | libc::FFDLY);
    s.c_lflag &= !libc::ECHO;
    s.c_lflag |= libc::ISIG;
    s.c_iflag &= !(libc::IUCLC | libc::ISTRIP);
    s.c_oflag &= !(libc::OLCUC | libc::TAB3);
    s.c_cflag = (s.c_cflag & !libc::CSIZE) | libc::CS8;
    s.c_cc[libc::VERASE] = 0xff;
    s.c_cc[libc::VKILL] = 0xff;
    // GNU keeps ICANON on and sets VEOF to C-d (sysdep.c:588-606).
    s.c_lflag |= libc::ICANON;
    s.c_cc[libc::VEOF] = 4;
    s.c_cc[libc::VEOL] = 0xff;
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &s) };
}

/// The pty's end-of-file character (process_send_eof sends it).
pub(crate) fn tty_eof_char(fd: RawFd) -> u8 {
    let mut s: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut s) } != 0 {
        return 4;
    }
    s.c_cc[libc::VEOF]
}

/// What a spawned child gets: GNU emacs_spawn's arguments.
pub(crate) struct Spawn<'a> {
    pub argv: &'a [Vec<u8>],
    pub env: &'a [Vec<u8>],
    pub dir: &'a [u8],
    pub stdin: RawFd,
    pub stdout: RawFd,
    pub stderr: RawFd,
    /// pty_in: make STDIN the controlling terminal.
    pub controlling_tty: bool,
}

/// emacs_spawn: POSIX_SPAWN_SETSID, default SIGINT/SIGQUIT/SIGPROF, an
/// empty signal mask, the given descriptors and environment.
pub(crate) fn spawn(spec: &Spawn<'_>) -> io::Result<i32> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;
    let os = |b: &[u8]| std::ffi::OsStr::from_bytes(b).to_os_string();
    let mut command = std::process::Command::new(os(&spec.argv[0]));
    command.args(spec.argv[1..].iter().map(|a| os(a)));
    command.env_clear();
    for entry in spec.env {
        if let Some(eq) = entry.iter().position(|&b| b == b'=') {
            command.env(os(&entry[..eq]), os(&entry[eq + 1..]));
        }
    }
    command.current_dir(os(spec.dir));
    let stdio = |fd: RawFd| -> io::Result<std::process::Stdio> {
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 {
            return Err(last_error());
        }
        Ok(std::process::Stdio::from(unsafe { OwnedFd::from_raw_fd(duplicate) }))
    };
    command.stdin(stdio(spec.stdin)?).stdout(stdio(spec.stdout)?).stderr(stdio(spec.stderr)?);
    let tty = spec.controlling_tty;
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if tty {
                libc::ioctl(0, libc::TIOCSCTTY, 0);
            }
            for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGPROF] {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    // The Child handle is dropped without waiting: the table reaps the pid.
    Ok(child.id() as i32)
}

/// file_accessible_directory_p: DIR is a directory that can be searched.
pub(crate) fn accessible_directory(dir: &[u8]) -> io::Result<()> {
    let path = std::ffi::CString::new(dir).map_err(|_| io::Error::from_raw_os_error(libc::ENOENT))?;
    use std::os::unix::ffi::OsStrExt;
    if !std::fs::metadata(std::ffi::OsStr::from_bytes(dir))?.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
    }
    if unsafe { libc::access(path.as_ptr(), libc::X_OK) } != 0 {
        return Err(last_error());
    }
    Ok(())
}

pub(crate) fn pidfd_open(pid: i32) -> Option<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// child_status_changed: a nonblocking waitpid, or None if unchanged.
pub(crate) fn wait_status(pid: i32, options: i32) -> Option<i32> {
    let mut status = 0;
    loop {
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG | options) };
        if r == pid {
            return Some(status);
        }
        if r < 0 && last_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return None;
    }
}

/// wait_for_termination: block until PID ends; its wait status.
pub(crate) fn wait_blocking(pid: i32) -> io::Result<i32> {
    let mut status = 0;
    loop {
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r == pid {
            return Ok(status);
        }
        let error = last_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// emacs_read: Ok(0) at end of file.
pub(crate) fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let error = last_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// One nonblocking write: bytes written, 0 when it would block.
pub(crate) fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { libc::send_or_write(fd, buf) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let error = last_error();
        match error.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock => return Ok(0),
            _ => return Err(error),
        }
    }
}

mod libc {
    pub use ::libc::*;
    /// write(2), except that a socket never raises SIGPIPE.
    pub unsafe fn send_or_write(fd: c_int, buf: &[u8]) -> isize {
        let n = unsafe { send(fd, buf.as_ptr().cast(), buf.len(), MSG_NOSIGNAL) };
        if n < 0 && unsafe { *__errno_location() } == ENOTSOCK {
            return unsafe { ::libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        }
        n
    }
}

/// Poll FDS (descriptor, events) for at most TIMEOUT; the revents of each.
pub(crate) fn poll(fds: &[(RawFd, i16)], timeout: Option<std::time::Duration>) -> io::Result<Vec<i16>> {
    let mut set: Vec<libc::pollfd> = fds.iter().map(|&(fd, events)| libc::pollfd { fd, events, revents: 0 }).collect();
    let millis = match timeout {
        None => -1,
        // Round up, so a wait never returns before its deadline.
        Some(d) => i32::try_from(d.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX),
    };
    let n = unsafe { libc::poll(set.as_mut_ptr(), set.len() as libc::nfds_t, millis) };
    if n < 0 {
        let error = last_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(vec![0; fds.len()]);
        }
        return Err(error);
    }
    Ok(set.iter().map(|p| p.revents).collect())
}

pub(crate) fn kill(pid: i32, signal: i32) -> io::Result<()> {
    if unsafe { libc::kill(pid, signal) } != 0 {
        return Err(last_error());
    }
    Ok(())
}

/// emacs_get_tty_pgrp: the foreground group of the pty on FD.
pub(crate) fn tty_pgrp(fd: RawFd) -> Option<i32> {
    let mut group: libc::pid_t = -1;
    (unsafe { libc::ioctl(fd, libc::TIOCGPGRP, &mut group) } != -1).then_some(group)
}

/// GNU's strsignal text for SIGNAL (glibc's English table).
pub(crate) fn signal_description(signal: i32) -> Option<String> {
    let text = unsafe { libc::strsignal(signal) };
    if text.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy().into_owned())
}

/// Signal names GNU's `signal-process' accepts (sysdep.c sys_siglist order).
pub(crate) fn signal_number(name: &str) -> Option<i32> {
    let name = name.strip_prefix("SIG").or_else(|| name.strip_prefix("sig")).unwrap_or(name);
    Some(match name.to_ascii_uppercase().as_str() {
        "HUP" => libc::SIGHUP, "INT" => libc::SIGINT, "QUIT" => libc::SIGQUIT, "ILL" => libc::SIGILL,
        "ABRT" | "IOT" => libc::SIGABRT, "TRAP" => libc::SIGTRAP, "EMT" => return None,
        "FPE" => libc::SIGFPE, "KILL" => libc::SIGKILL, "BUS" => libc::SIGBUS, "SEGV" => libc::SIGSEGV,
        "SYS" => libc::SIGSYS, "PIPE" => libc::SIGPIPE, "ALRM" => libc::SIGALRM, "TERM" => libc::SIGTERM,
        "URG" => libc::SIGURG, "STOP" => libc::SIGSTOP, "TSTP" => libc::SIGTSTP, "CONT" => libc::SIGCONT,
        "CHLD" | "CLD" => libc::SIGCHLD, "TTIN" => libc::SIGTTIN, "TTOU" => libc::SIGTTOU, "IO" | "POLL" => libc::SIGIO,
        "XCPU" => libc::SIGXCPU, "XFSZ" => libc::SIGXFSZ, "VTALRM" => libc::SIGVTALRM, "PROF" => libc::SIGPROF,
        "WINCH" => libc::SIGWINCH, "USR1" => libc::SIGUSR1, "USR2" => libc::SIGUSR2, "PWR" => libc::SIGPWR,
        _ => return None,
    })
}

