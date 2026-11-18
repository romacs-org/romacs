//! GNU process.c and callproc.c operating-system mechanics: process objects'
//! native state, descriptors, spawning, reaping and polling. Nothing here
//! calls Lisp; the wait loop that runs filters, sentinels and timers is Lisp
//! (lisp/vm/subprocess.el) over the leaf primitives in prims/process.rs.
use crate::Value;
use std::collections::BTreeMap;
use std::io;
use rustix::termios::Termios;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};

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
    Thread,
}
pub(crate) const SLOTS: usize = Slot::Thread as usize + 1;

/// A process object. Its descriptors live in [`Table`] under `serial`, so a
/// moved or copied object never owns an OS resource.
#[derive(Clone, romacs_snapshot::Snapshot)]
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
    /// A :nowait connection in progress (GNU's NON_BLOCKING_CONNECT_FD): the
    /// addresses still to try if this one fails (GNU's addrinfos).
    pub connecting: Option<Vec<std::net::SocketAddr>>,
    /// CODING_MODE_LAST_BLOCK: end of file was seen and the carryover flushed.
    pub last_block: bool,
    /// The TLS session `gnutls-boot' started (GNU's gnutls_state): reads
    /// and writes go through it once it is READY.
    pub tls: Option<Box<crate::tls::Session>>,
    /// The stage `gnutls-deinit' left (GNUTLS_STAGE_INIT - 1 once a
    /// session was initialized), reported while no session is attached.
    pub tls_stage: u8,
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
    /// Whether PID, a child handed to the reaper of deleted children, is still
    /// owned there: false once it is reaped, the positive notice of its death.
    pub(crate) fn owns(&self, pid: i32) -> bool {
        self.deleted.iter().any(|&(owned, _)| owned == pid)
    }
    /// Reap killed children recorded by `delete-process' and `call-process'.
    pub(crate) fn reap_deleted(&mut self) {
        self.deleted.retain(|(pid, file)| {
            let done = reaped(*pid);
            if done && let Some(file) = file {
                let _ = crate::host::remove_file(file);
            }
            if done { crate::isolate::child_reaped(*pid); }
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
    // CELL(Plain)
    state: std::sync::Mutex<Reaping>,
}

#[derive(Default)]
struct Reaping {
    orphans: Vec<Orphan>,
    /// The running thread's eventfd, None until a start succeeds.
    wake: Option<std::fs::File>,
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
            let _ = std::io::Write::write(&mut &*wake, &1u64.to_ne_bytes());
        } else if !state.orphans.is_empty() {
            state.wake = self.start(start).ok();
        }
    }

    fn start(&'static self, start: Start) -> io::Result<std::fs::File> {
        let wake = std::fs::File::from(crate::host::eventfd()?);
        let read_end = wake.try_clone()?;
        start(Box::new(move || self.run(read_end)))?;
        Ok(wake)
    }

    /// Poll the orphans' pidfds and WAKE, then reap whichever ended. Only
    /// this thread removes orphans, so the polled descriptors stay open. An
    /// orphan without a pidfd is checked every 100ms.
    fn run(&self, wake: std::fs::File) {
        loop {
            let (fds, timeout) = {
                let state = self.lock();
                let fds: Vec<_> = std::iter::once(wake.as_raw_fd()).chain(state.orphans.iter().filter_map(|o| o.pidfd.as_ref().map(|fd| fd.as_raw_fd()))).map(|fd| (fd, libc::POLLIN)).collect();
                (fds, state.orphans.iter().any(|o| o.pidfd.is_none()).then(|| std::time::Duration::from_millis(100)))
            };
            let _ = poll(&fds, timeout);
            let _ = std::io::Read::read(&mut &wake, &mut [0; 8]);
            self.lock().orphans.retain(|o| !reap(o));
        }
    }
}

/// Wait for ORPHAN without blocking, then unlink its file: true once it is
/// no longer this host's child to wait for.
fn reap(orphan: &Orphan) -> bool {
    if !reaped(orphan.pid) {
        return false;
    }
    if let Some(file) = &orphan.file {
        let _ = crate::host::remove_file(file);
    }
    true
}

/// Wait for PID without blocking: true once it is no longer this host's
/// child to wait for (reaped now, or not a child).
fn reaped(pid: i32) -> bool {
    let Some(pid) = child(pid) else { return true };
    loop {
        match rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG) {
            Err(rustix::io::Errno::INTR) => {}
            Ok(None) => return false,
            Ok(Some(_)) | Err(_) => return true,
        }
    }
}

/// The child PID names: None for 0 or a negative number, which waitpid
/// would read as a process group.
fn child(pid: i32) -> Option<rustix::process::Pid> {
    if pid > 0 { rustix::process::Pid::from_raw(pid) } else { None }
}

pub(crate) fn last_error() -> io::Error {
    io::Error::last_os_error()
}

pub(crate) fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd)?;
    Ok(rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?)
}

pub(crate) fn dup(fd: &OwnedFd) -> io::Result<OwnedFd> {
    fd.try_clone()
}

/// The terminal modes of the pty on FD, or None if it is not one.
pub(crate) fn terminal_modes(fd: BorrowedFd<'_>) -> Option<Termios> {
    rustix::termios::tcgetattr(fd).ok()
}

/// sysdep.c child_setup_tty: the terminal modes a pty child starts with.
pub(crate) fn child_setup_tty(fd: BorrowedFd<'_>) {
    use rustix::termios::{ControlModes, InputModes, LocalModes, OptionalActions, OutputModes, SpecialCodeIndex};
    let Some(mut s) = terminal_modes(fd) else { return };
    s.output_modes |= OutputModes::OPOST;
    s.output_modes &= !OutputModes::ONLCR;
    s.output_modes &= !(OutputModes::NLDLY | OutputModes::CRDLY | OutputModes::TABDLY | OutputModes::BSDLY
        | OutputModes::VTDLY | OutputModes::FFDLY);
    s.local_modes &= !LocalModes::ECHO;
    s.local_modes |= LocalModes::ISIG;
    s.input_modes &= !(InputModes::IUCLC | InputModes::ISTRIP);
    s.output_modes &= !(OutputModes::OLCUC | OutputModes::TAB3);
    s.control_modes = (s.control_modes & !ControlModes::CSIZE) | ControlModes::CS8;
    s.special_codes[SpecialCodeIndex::VERASE] = 0xff;
    s.special_codes[SpecialCodeIndex::VKILL] = 0xff;
    // GNU keeps ICANON on and sets VEOF to C-d (sysdep.c:588-606).
    s.local_modes |= LocalModes::ICANON;
    s.special_codes[SpecialCodeIndex::VEOF] = 4;
    s.special_codes[SpecialCodeIndex::VEOL] = 0xff;
    let _ = rustix::termios::tcsetattr(fd, OptionalActions::Now, &s);
}

/// The pty's end-of-file character (process_send_eof sends it).
pub(crate) fn tty_eof_char(fd: BorrowedFd<'_>) -> u8 {
    terminal_modes(fd).map_or(4, |s| s.special_codes[rustix::termios::SpecialCodeIndex::VEOF])
}

/// A close-on-exec duplicate of FD numbered 3 or above, so placing a child's
/// standard descriptors cannot close it first.
fn duplicate_above_stdio(fd: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    Ok(rustix::io::fcntl_dupfd_cloexec(fd, 3)?)
}

/// What a spawned child gets: GNU emacs_spawn's arguments.
pub(crate) struct Spawn<'a> {
    pub argv: &'a [Vec<u8>],
    pub env: &'a [Vec<u8>],
    pub dir: &'a [u8],
    pub stdin: BorrowedFd<'a>,
    pub stdout: BorrowedFd<'a>,
    pub stderr: BorrowedFd<'a>,
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
    // A program named by a path and no controlling terminal: posix_spawn,
    // which a fork's page-table copy would make cost what the heap weighs.
    if !spec.controlling_tty && spec.argv.first().is_some_and(|p| p.contains(&b'/')) {
        let fds = [duplicate_above_stdio(spec.stdin)?, duplicate_above_stdio(spec.stdout)?, duplicate_above_stdio(spec.stderr)?];
        let env: Vec<Vec<u8>> = spec.env.iter().filter(|e| e.contains(&b'=')).cloned().collect();
        let pid = crate::host::spawn_child(spec.argv, &env, spec.dir, fds.each_ref().map(|fd| fd.as_raw_fd()))?;
        crate::isolate::child_spawned(pid);
        return Ok(pid);
    }
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
    let stdio = |fd: BorrowedFd<'_>| duplicate_above_stdio(fd).map(std::process::Stdio::from);
    command.stdin(stdio(spec.stdin)?).stdout(stdio(spec.stdout)?).stderr(stdio(spec.stderr)?);
    let tty = spec.controlling_tty;
    // UNSAFE(std runs code between fork and exec only through the unsafe pre_exec)
    // SAFETY: the hook runs in the forked child before exec, and calls only
    // async-signal-safe functions (setsid, ioctl, signal), allocating nothing.
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
    let pid = child.id() as i32;
    crate::isolate::child_spawned(pid);
    Ok(pid)
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
pub(crate) fn wait_status(pid: i32, options: rustix::process::WaitOptions) -> Option<i32> {
    let child = child(pid)?;
    loop {
        match rustix::process::waitpid(Some(child), rustix::process::WaitOptions::NOHANG | options) {
            Ok(Some((_, status))) => {
                if status.exited() || status.signaled() { crate::isolate::child_reaped(pid); }
                return Some(status.as_raw());
            }
            Err(rustix::io::Errno::INTR) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Whether child PID has ended, left unreaped for the wait that takes its
/// status (waitid's WNOWAIT): true too once it is no child of this host's.
/// A waiter without a pidfd asks it between ticks.
pub(crate) fn exited(pid: i32) -> bool {
    use rustix::process::WaitIdOptions as W;
    let Some(child) = child(pid) else { return true };
    loop {
        match rustix::process::waitid(rustix::process::WaitId::Pid(child), W::EXITED | W::NOHANG | W::NOWAIT) {
            Err(rustix::io::Errno::INTR) => {}
            Ok(found) => return found.is_some(),
            Err(_) => return true,
        }
    }
}

/// wait_for_termination: block until PID ends; its wait status.
pub(crate) fn wait_blocking(pid: i32) -> io::Result<i32> {
    let child = child(pid).ok_or(rustix::io::Errno::CHILD)?;
    loop {
        match rustix::process::waitpid(Some(child), rustix::process::WaitOptions::empty()) {
            Ok(Some((_, status))) => {
                crate::isolate::child_reaped(pid);
                return Ok(status.as_raw());
            }
            Err(rustix::io::Errno::INTR) => {}
            Ok(None) => return Err(rustix::io::Errno::CHILD.into()),
            Err(error) => return Err(error.into()),
        }
    }
}

/// Whether child PID has exited, without reaping it: its number stays pinned
/// until a wait takes it.
pub(crate) fn child_exited(pid: i32) -> io::Result<bool> {
    let child = child(pid).ok_or(rustix::io::Errno::CHILD)?;
    let options = rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOHANG | rustix::process::WaitIdOptions::NOWAIT;
    loop {
        // NOHANG answers None while the child runs.
        match rustix::process::waitid(rustix::process::WaitId::Pid(child), options) {
            Ok(status) => return Ok(status.is_some()),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// emacs_read: Ok(0) at end of file.
pub(crate) fn read(fd: BorrowedFd<'_>, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match rustix::io::read(fd, &mut *buf) {
            Err(rustix::io::Errno::INTR) => {}
            read => return Ok(read?),
        }
    }
}

/// One nonblocking write: bytes written, 0 when it would block.
pub(crate) fn write(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    loop {
        match send_or_write(fd, buf) {
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::WOULDBLOCK) => return Ok(0),
            written => return Ok(written?),
        }
    }
}

/// write(2), except that a socket never raises SIGPIPE.
fn send_or_write(fd: BorrowedFd<'_>, buf: &[u8]) -> rustix::io::Result<usize> {
    match rustix::net::send(fd, buf, rustix::net::SendFlags::NOSIGNAL) {
        Err(rustix::io::Errno::NOTSOCK) => rustix::io::write(fd, buf),
        sent => sent,
    }
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
        // HOST(std::net): ProcessWatch's cancel pair: a socket pair private to this process that
        // stops its own poll thread; it reaches no host fact
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
    // UNSAFE(rustix's poll takes borrowed descriptors; these are numbers polled across threads)
    // SAFETY: poll reads and writes the `set.len()' pollfds `set' owns. A
    // descriptor is only a number to it: one closed meanwhile answers
    // POLLNVAL, and no memory is reached through it.
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

/// ADDRESS in GNU's families; None for another.
fn decode_sockaddr(address: rustix::net::SocketAddrAny) -> Option<Address> {
    match address.address_family() {
        rustix::net::AddressFamily::INET => {
            let v4 = std::net::SocketAddrV4::try_from(address).ok()?;
            Some(Address::V4(v4.ip().octets(), v4.port()))
        }
        rustix::net::AddressFamily::INET6 => {
            let v6 = std::net::SocketAddrV6::try_from(address).ok()?;
            Some(Address::V6(v6.ip().segments(), v6.port()))
        }
        rustix::net::AddressFamily::UNIX => Some(Address::Local(local_name(&address))),
        _ => None,
    }
}

/// A local socket's name, as the LEN bytes of ADDRESS hold it past the
/// family: a NUL ends a path; a Linux abstract name keeps its leading NUL
/// and may hold NULs; an unnamed socket's is empty. Read from the bytes,
/// not through rustix's `SocketAddrUnix`, which reads an unnamed socket as
/// an empty abstract name and indexes past `sun_path` for a path that
/// fills it.
fn local_name(address: &rustix::net::SocketAddrAny) -> Vec<u8> {
    let offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    let len = (address.addr_len() as usize).min(size_of::<libc::sockaddr_un>());
    // UNSAFE(rustix's SocketAddrAny has no safe view of its bytes, and its unix decoding misreads these)
    // SAFETY: the storage is a sockaddr_storage the kernel filled, with
    // its first ADDR_LEN bytes initialised; LEN is at most that and at
    // most a sockaddr_un, which the storage is sized for.
    let bytes = unsafe { std::slice::from_raw_parts(address.as_ptr().cast::<u8>(), len) };
    let path = bytes.get(offset..).unwrap_or_default();
    match path.first() {
        Some(&b) if b != 0 => path.split(|&b| b == 0).next().unwrap_or(path).to_vec(),
        _ => path.to_vec(),
    }
}

/// getsockname (LOCAL) or getpeername of the socket FD.
pub(crate) fn socket_address(fd: BorrowedFd<'_>, local: bool) -> io::Result<Option<Address>> {
    let address = if local { Some(rustix::net::getsockname(fd)?) } else { rustix::net::getpeername(fd)? };
    Ok(address.and_then(decode_sockaddr))
}

/// server_accept_connection's accept: the connection, close-on-exec, with
/// the peer's address.
pub(crate) fn accept(listener: BorrowedFd<'_>) -> io::Result<(OwnedFd, Option<Address>)> {
    let (fd, address) = crate::host::accept(listener)?;
    Ok((fd, address.and_then(decode_sockaddr)))
}

/// SO_ERROR of the socket FD: how its nonblocking connect ended.
pub(crate) fn socket_error(fd: BorrowedFd<'_>) -> i32 {
    match rustix::net::sockopt::socket_error(fd) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) | Err(error) => error.raw_os_error(),
    }
}

/// emacs_get_tty_pgrp: the foreground group of the pty on FD.
pub(crate) fn tty_pgrp(fd: BorrowedFd<'_>) -> Option<i32> {
    rustix::termios::tcgetpgrp(fd).ok().map(rustix::process::Pid::as_raw_nonzero).map(std::num::NonZeroI32::get)
}

/// GNU's strsignal text for SIGNAL (glibc's English table).
pub(crate) fn signal_description(signal: i32) -> Option<String> {
    crate::host::signal_text(signal)
}

/// gnulib sig2str's name for SIGNUM on GNU/Linux: the first name its table
/// gives the number, or RTMIN+N / RTMAX-N in the real-time range.
pub(crate) fn signal_name(signum: i32) -> Option<String> {
    const NAMES: [&str; 32] = ["EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "BUS", "FPE", "KILL", "USR1", "SEGV",
        "USR2", "PIPE", "ALRM", "TERM", "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN", "TTOU", "URG", "XCPU", "XFSZ",
        "VTALRM", "PROF", "WINCH", "POLL", "PWR", "SYS"];
    if let Some(name) = usize::try_from(signum).ok().and_then(|i| NAMES.get(i)) { return Some((*name).into()); }
    let (min, max) = (libc::SIGRTMIN(), libc::SIGRTMAX());
    if !(min..=max).contains(&signum) { return None; }
    Some(if signum <= min + (max - min) / 2 {
        if signum == min { "RTMIN".into() } else { format!("RTMIN+{}", signum - min) }
    } else if signum == max { "RTMAX".into() } else { format!("RTMAX-{}", max - signum) })
}

/// SIGNUM_BOUND: the largest signal number.
pub(crate) fn signal_bound() -> i32 { libc::SIGRTMAX() }

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

    /// A bound path, an abstract name with a NUL in it, and an unnamed
    /// socket each read back as GNU's process.c names them.
    #[test]
    fn local_socket_names_keep_their_shape() {
        use std::os::unix::net::{SocketAddr, UnixDatagram};
        use std::os::linux::net::SocketAddrExt;
        use std::os::fd::AsFd;
        let dir = std::env::temp_dir().join(format!("romacs-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s");
        let named = UnixDatagram::bind(&path).unwrap();
        let name = |fd: &UnixDatagram| match socket_address(fd.as_fd(), true).unwrap() {
            Some(Address::Local(name)) => name,
            _ => panic!("not a local address"),
        };
        assert_eq!(name(&named), path.as_os_str().as_encoded_bytes());
        let abstract_name = format!("romacs\0names-{}", std::process::id()).into_bytes();
        let hidden = UnixDatagram::bind_addr(&SocketAddr::from_abstract_name(&abstract_name).unwrap()).unwrap();
        assert_eq!(name(&hidden), [&[0], &abstract_name[..]].concat());
        assert_eq!(name(&UnixDatagram::unbound().unwrap()), b"");
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
        let pid = rustix::process::Pid::from_raw(pid).unwrap();
        assert_eq!(rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).err(), Some(rustix::io::Errno::CHILD));
        assert!(!file.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
