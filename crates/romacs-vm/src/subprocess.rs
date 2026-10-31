//! GNU process.c and callproc.c operating-system mechanics: process objects'
//! native state, descriptors, spawning, reaping and polling. Nothing here
//! calls Lisp; the wait loop that runs filters, sentinels and timers is Lisp
//! (rlisp/vm/subprocess.el) over the leaf primitives in prims/process.rs.
use crate::Value;
use std::collections::BTreeMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

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
romacs_snapshot::snapshot_struct!(Process { slots, serial, pid, alive, tick, update_tick, raw_status, kill_without_query, inherit_coding, pty_in, pty_out, nbytes_read, readmax });

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
    /// A :nowait connection in progress (GNU's NON_BLOCKING_CONNECT_FD): the
    /// addresses still to try if this one fails (GNU's addrinfos).
    pub connecting: Option<Vec<std::net::SocketAddr>>,
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
    /// How many selects found something ready: the command loop's idle
    /// wait asks whether its pass handled anything.
    pub activity: u64,
    /// GNU connect_counter: how many connections servers have accepted,
    /// naming those whose peer has no host.
    pub connect_counter: u64,
}

impl Table {
    /// No channel and no child left to reap.
    pub(crate) fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.deleted.is_empty()
    }
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
                let _ = crate::host::remove_file(file);
            }
            !done
        });
    }
}

impl Drop for Table {
    /// shut_down_emacs's kill_buffer_processes (Qnil): SIGHUP to the group of
    /// each child not yet reaped (unreaped, so its pid is not reused). The
    /// host outlives the VM, so these and the deleted children then pass to
    /// the host's reaper where GNU leaves them to init.
    fn drop(&mut self) {
        let deleted = std::mem::take(&mut self.deleted).into_iter().map(|(pid, file)| Orphan { pid, pidfd: crate::host::pidfd_open(pid), file });
        let live = self.channels.values_mut().filter(|c| c.pidfd.is_some() && c.pid > 0).map(|c| {
            let _ = crate::host::kill(-c.pid, libc::SIGHUP);
            Orphan { pid: c.pid, pidfd: c.pidfd.take(), file: None }
        });
        let orphans: Vec<Orphan> = deleted.chain(live).collect();
        REAPER.adopt(orphans, spawn_reaper);
    }
}

/// A child no VM owns any more: its pid, a pidfd readable once it exits, and
/// a file to unlink once it is reaped.
struct Orphan {
    pid: i32,
    pidfd: Option<OwnedFd>,
    file: Option<std::path::PathBuf>,
}

/// The host's one reaper thread, shared by every VM. An orphan's pid stays
/// registered here, its only owner, until it is waited for exactly once:
/// unreaped, the pid cannot be reused, and no second waiter exists to reap a
/// reused one.
#[derive(Default)]
struct Reaper {
    state: std::sync::Mutex<Reaping>,
}

#[derive(Default)]
struct Reaping {
    orphans: Vec<Orphan>,
    /// The running thread's eventfd, None until a start succeeds.
    wake: Option<OwnedFd>,
}

static REAPER: Reaper = Reaper { state: std::sync::Mutex::new(Reaping { orphans: Vec::new(), wake: None }) };

/// Starts the reaper's thread: production's [`spawn_reaper`], or a test's.
type Start = fn(Box<dyn FnOnce() + Send>) -> io::Result<()>;

fn spawn_reaper(run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    std::thread::Builder::new().name("romacs-reaper".into()).stack_size(64 * 1024).spawn(run).map(drop)
}

impl Reaper {
    fn lock(&self) -> std::sync::MutexGuard<'_, Reaping> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Register ORPHANS and wake the thread, or start it. A start that fails
    /// keeps every orphan registered; the next handoff, a VM's drop or a
    /// process creation, tries again.
    fn adopt(&'static self, orphans: Vec<Orphan>, start: Start) {
        let mut state = self.lock();
        let fresh = !orphans.is_empty();
        for orphan in orphans {
            match state.orphans.iter_mut().find(|o| o.pid == orphan.pid) {
                Some(known) => known.file = known.file.take().or(orphan.file),
                None => state.orphans.push(orphan),
            }
        }
        if let Some(wake) = &state.wake {
            if !fresh {
                return;
            }
            let one = 1u64;
            unsafe { libc::write(wake.as_raw_fd(), (&one as *const u64).cast(), 8) };
        } else if !state.orphans.is_empty() {
            state.wake = self.start(start).ok();
        }
    }

    fn start(&'static self, start: Start) -> io::Result<OwnedFd> {
        let wake = crate::host::eventfd()?;
        let fd = wake.as_raw_fd();
        start(Box::new(move || self.run(fd)))?;
        Ok(wake)
    }

    /// Poll the orphans' pidfds and WAKE, then reap whichever ended. Only
    /// this thread removes orphans, so the polled descriptors stay open. An
    /// orphan without a pidfd is checked every 100ms.
    fn run(&self, wake: RawFd) {
        let watch = |fd: RawFd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        loop {
            let (mut fds, timeout) = {
                let state = self.lock();
                let fds: Vec<_> = std::iter::once(wake).chain(state.orphans.iter().filter_map(|o| o.pidfd.as_ref().map(|fd| fd.as_raw_fd()))).map(watch).collect();
                (fds, if state.orphans.iter().any(|o| o.pidfd.is_none()) { 100 } else { -1 })
            };
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            let mut count = 0u64;
            unsafe { libc::read(wake, (&mut count as *mut u64).cast(), 8) };
            self.lock().orphans.retain(|o| !reap(o));
        }
    }
}

/// Wait for ORPHAN without blocking, then unlink its file: true once it is
/// no longer this host's child to wait for.
fn reap(orphan: &Orphan) -> bool {
    let mut status = 0;
    let r = loop {
        let r = unsafe { libc::waitpid(orphan.pid, &mut status, libc::WNOHANG) };
        if r >= 0 || last_error().kind() != io::ErrorKind::Interrupted {
            break r;
        }
    };
    if r == 0 {
        return false;
    }
    if let Some(file) = &orphan.file {
        let _ = crate::host::remove_file(file);
    }
    true
}

pub(crate) fn last_error() -> io::Error {
    io::Error::last_os_error()
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
    // A process creation retries a reaper that failed to start.
    REAPER.adopt(Vec::new(), spawn_reaper);
    let os = |b: &[u8]| std::ffi::OsStr::from_bytes(b).to_os_string();
    let mut command = crate::host::command(os(&spec.argv[0]))?;
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
    if !crate::host::metadata(std::ffi::OsStr::from_bytes(dir))?.file_type().is_dir() {
        return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
    }
    crate::host::access(&path, libc::X_OK, crate::host::Ids::Real)
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
        let n = unsafe { send_or_write(fd, buf) };
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

/// write(2), except that a socket never raises SIGPIPE.
unsafe fn send_or_write(fd: RawFd, buf: &[u8]) -> isize {
    let n = unsafe { libc::send(fd, buf.as_ptr().cast(), buf.len(), libc::MSG_NOSIGNAL) };
    if n < 0 && unsafe { *libc::__errno_location() } == libc::ENOTSOCK {
        return unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    }
    n
}

/// Poll FDS (descriptor, events) for at most TIMEOUT; the revents of each.
/// A thread waiting, for a host's blocking wait, until one of a set of
/// process descriptors (`Vm::process_wait_fds') is ready, which it reports
/// once through READY; dropping it ends the wait.  The descriptors must
/// stay open meanwhile, as they do while the VM runs no Lisp.
pub struct ProcessWatch {
    cancel: std::os::unix::net::UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProcessWatch {
    pub fn start(mut fds: Vec<(RawFd, i16)>, ready: impl FnOnce() + Send + 'static) -> io::Result<Self> {
        let (cancel, stop) = std::os::unix::net::UnixStream::pair()?;
        let thread = std::thread::Builder::new().name("process-watch".into()).spawn(move || {
            use std::os::fd::AsRawFd;
            fds.push((stop.as_raw_fd(), libc::POLLIN));
            loop {
                let Ok(revents) = poll(&fds, None) else { return };
                let (watched, stopped) = revents.split_at(revents.len() - 1);
                if stopped[0] != 0 { return; }
                if watched.iter().any(|&r| r != 0) {
                    ready();
                    return;
                }
            }
        })?;
        Ok(Self { cancel, thread: Some(thread) })
    }
}

impl Drop for ProcessWatch {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = (&self.cancel).write_all(&[0]);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

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

/// A socket address as conv_sockaddr_to_lisp reads it: IPv4 and IPv6 with
/// their port, or a local socket's path (empty for an unnamed socket).
pub(crate) enum Address {
    V4([u8; 4], u16),
    V6([u16; 8], u16),
    Local(Vec<u8>),
}

/// The address LEN bytes of STORAGE hold, in GNU's families; None for another.
fn decode_sockaddr(storage: &libc::sockaddr_storage, len: libc::socklen_t) -> Option<Address> {
    let base = (storage as *const libc::sockaddr_storage).cast::<u8>();
    match i32::from(storage.ss_family) {
        libc::AF_INET => {
            let sin = unsafe { &*(storage as *const libc::sockaddr_storage).cast::<libc::sockaddr_in>() };
            Some(Address::V4(sin.sin_addr.s_addr.to_ne_bytes(), u16::from_be(sin.sin_port)))
        }
        libc::AF_INET6 => {
            let sin6 = unsafe { &*(storage as *const libc::sockaddr_storage).cast::<libc::sockaddr_in6>() };
            let octets = sin6.sin6_addr.s6_addr;
            let segments = std::array::from_fn(|i| u16::from_be_bytes([octets[2 * i], octets[2 * i + 1]]));
            Some(Address::V6(segments, u16::from_be(sin6.sin6_port)))
        }
        libc::AF_UNIX => {
            // The name is what fits in LEN past the family; a NUL ends a
            // path, while a Linux abstract name (leading NUL) may hold NULs.
            let offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
            let available = (len as usize).saturating_sub(offset).min(std::mem::size_of::<libc::sockaddr_un>() - offset);
            let path = unsafe { std::slice::from_raw_parts(base.add(offset), available) };
            let name = if path.first().is_some_and(|&b| b != 0) { path.iter().position(|&b| b == 0).map_or(path, |end| &path[..end]) } else { path };
            Some(Address::Local(name.to_vec()))
        }
        _ => None,
    }
}

/// getsockname (LOCAL) or getpeername of the socket FD.
pub(crate) fn socket_address(fd: RawFd, local: bool) -> io::Result<Option<Address>> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let sa = (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr>();
    let result = if local { unsafe { libc::getsockname(fd, sa, &mut len) } } else { unsafe { libc::getpeername(fd, sa, &mut len) } };
    if result != 0 {
        return Err(last_error());
    }
    Ok(decode_sockaddr(&storage, len))
}

/// server_accept_connection's accept4: the connection, close-on-exec, with
/// the peer's address.
pub(crate) fn accept(listener: RawFd) -> io::Result<(OwnedFd, Option<Address>)> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let sa = (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr>();
    let fd = unsafe { libc::accept4(listener, sa, &mut len, libc::SOCK_CLOEXEC) };
    if fd < 0 {
        return Err(last_error());
    }
    Ok((unsafe { OwnedFd::from_raw_fd(fd) }, decode_sockaddr(&storage, len)))
}

/// SO_ERROR of the socket FD: how its nonblocking connect ended.
pub(crate) fn socket_error(fd: RawFd) -> i32 {
    let mut error: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    if unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, (&mut error as *mut libc::c_int).cast(), &mut len) } != 0 {
        return last_error().raw_os_error().unwrap_or(libc::ECONNREFUSED);
    }
    error
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The child's /proc state letter, or None once no such process exists.
    fn state(pid: i32) -> Option<char> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat.rsplit_once(") ").and_then(|(_, rest)| rest.chars().next())
    }

    fn poll_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::yield_now();
        }
    }

    fn refuse(_: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::EAGAIN))
    }

    // A refused thread start keeps the orphan registered, once, unreaped;
    // the next handoff starts the thread, which reaps it and unlinks its file.
    #[test]
    fn reaper_keeps_orphans_until_a_later_start() {
        let reaper: &'static Reaper = Box::leak(Box::default());
        let dir = std::env::temp_dir().join(format!("romacs-reaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("orphan");
        std::fs::write(&file, "").unwrap();
        let pid = std::process::Command::new("true").spawn().unwrap().id() as i32;
        reaper.adopt(vec![Orphan { pid, pidfd: crate::host::pidfd_open(pid), file: None }], refuse);
        reaper.adopt(vec![Orphan { pid, pidfd: crate::host::pidfd_open(pid), file: Some(file.clone()) }], refuse);
        assert!(reaper.lock().wake.is_none());
        assert_eq!(reaper.lock().orphans.iter().map(|o| o.pid).collect::<Vec<_>>(), [pid]);
        poll_until("the orphan to exit", || state(pid) == Some('Z'));
        assert!(file.exists());
        reaper.adopt(Vec::new(), spawn_reaper);
        assert!(reaper.lock().wake.is_some());
        poll_until("the orphan to be reaped", || reaper.lock().orphans.is_empty());
        assert_eq!(unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) }, -1);
        assert_eq!(last_error().raw_os_error(), Some(libc::ECHILD));
        assert!(!file.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
