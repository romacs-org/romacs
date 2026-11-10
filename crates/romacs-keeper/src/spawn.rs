//! Spawning and reaping, as a library.
//!
//! One spawn mechanism with two hosts: the keeper calls it for children
//! that must outlive a session, and a process that keeps no children
//! (`--batch`, a test, a worker VM) calls the same code itself. A [`Spec`]
//! carries everything the child's start depends on, whole: nothing is
//! inherited from the spawner but what the spec names.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use romacs_record::{Error, Reader, Writer};

use crate::proto::count;

/// What a child's descriptor is, and so what the spawner keeps of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// `/dev/null`; nothing is kept.
    Null,
    /// The read end of a pipe; the spawner keeps the write end.
    Reads,
    /// The write end of a pipe; the spawner keeps the read end.
    Writes,
    /// One end of a stream socketpair; the spawner keeps the other.
    Socket,
    /// The spec's one pty, as the child's controlling terminal when the
    /// spec asks for a session. The spawner keeps the master, once, at the
    /// first such slot.
    Pty,
}

/// Everything a child's start depends on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Spec {
    /// The program (looked up in no path) and its arguments.
    pub argv: Vec<Vec<u8>>,
    /// The whole environment, `NAME=value` each.
    pub env: Vec<Vec<u8>>,
    pub cwd: Vec<u8>,
    pub umask: u32,
    /// `(resource, soft, hard)` for `setrlimit`.
    pub limits: Vec<(u32, u64, u64)>,
    /// The blocked signals: bit N-1 for signal N.
    pub sigmask: u64,
    /// Whether the child leads a new session (and takes the pty as its
    /// controlling terminal).
    pub setsid: bool,
    /// The child's descriptors, by number. Every other one is closed.
    pub fds: Vec<(u32, End)>,
    /// The pty's columns and rows.
    pub pty_size: (u16, u16),
}

impl Spec {
    #[must_use]
    pub fn write(&self, mut w: Writer) -> Writer {
        let list = |mut w: Writer, items: &[Vec<u8>]| {
            w = w.u32(count(items.len()));
            items.iter().fold(w, |w, item| w.bytes(item))
        };
        w = list(w, &self.argv);
        w = list(w, &self.env);
        w = w.bytes(&self.cwd).u32(self.umask).u32(count(self.limits.len()));
        for &(resource, soft, hard) in &self.limits {
            w = w.u32(resource).u64(soft).u64(hard);
        }
        w = w.u64(self.sigmask).u8(u8::from(self.setsid)).u32(count(self.fds.len()));
        for &(fd, end) in &self.fds {
            w = w.u32(fd).u8(end as u8);
        }
        w.u16(self.pty_size.0).u16(self.pty_size.1)
    }

    /// # Errors
    /// [`Error::Truncated`], also for an end this build does not know.
    pub fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        fn list(r: &mut Reader<'_>) -> Result<Vec<Vec<u8>>, Error> {
            (0..r.u32()?).map(|_| r.bytes().map(<[u8]>::to_vec)).collect()
        }
        let (argv, env) = (list(r)?, list(r)?);
        let (cwd, umask) = (r.bytes()?.to_vec(), r.u32()?);
        let limits = (0..r.u32()?).map(|_| Ok((r.u32()?, r.u64()?, r.u64()?))).collect::<Result<_, Error>>()?;
        let (sigmask, setsid) = (r.u64()?, r.u8()? != 0);
        let fds = (0..r.u32()?).map(|_| {
            let (fd, end) = (r.u32()?, r.u8()?);
            let end = [End::Null, End::Reads, End::Writes, End::Socket, End::Pty].into_iter().find(|e| *e as u8 == end).ok_or(Error::Truncated)?;
            Ok((fd, end))
        }).collect::<Result<_, Error>>()?;
        Ok(Self { argv, env, cwd, umask, limits, sigmask, setsid, fds, pty_size: (r.u16()?, r.u16()?) })
    }
}

/// A spawned child: the spawner is its parent and holds these.
pub struct Child {
    pub pid: i32,
    /// Names the child without its pid: signals and waits go through it.
    pub pidfd: OwnedFd,
    /// The spawner's ends, in the order of the spec's kept slots.
    pub fds: Vec<OwnedFd>,
}

fn os<T: PartialOrd + Default>(result: T) -> io::Result<T> {
    if result < T::default() { Err(io::Error::last_os_error()) } else { Ok(result) }
}

fn owned(fd: RawFd) -> OwnedFd {
    // SAFETY: every caller passes a descriptor a system call just returned
    // to it, open and owned by nothing else.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn pair(make: impl FnOnce(&mut [RawFd; 2]) -> libc::c_int) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    os(make(&mut fds))?;
    Ok((owned(fds[0]), owned(fds[1])))
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    // SAFETY: pipe2 writes two descriptors into the array it is given.
    pair(|fds| unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })
}

fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    // SAFETY: socketpair writes two descriptors into the array it is given.
    pair(|fds| unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) })
}

/// A pty's master and slave, sized.
fn pty(size: (u16, u16)) -> io::Result<(OwnedFd, OwnedFd)> {
    let window = libc::winsize { ws_col: size.0, ws_row: size.1, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: openpty writes two descriptors through the first two
    // pointers and only reads the window size; name and termios are null.
    let (master, slave) = pair(|fds| unsafe {
        let (first, second) = fds.split_at_mut(1);
        libc::openpty(first.as_mut_ptr(), second.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null(), &raw const window)
    })?;
    for fd in [&master, &slave] {
        // SAFETY: F_SETFD sets a flag on a descriptor owned here.
        os(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    Ok((master, slave))
}

/// The child's ends by descriptor number, and the spawner's.
type Ends = (Vec<(RawFd, OwnedFd)>, Vec<OwnedFd>);

/// The child's and the spawner's ends for each slot of SPEC.
fn ends(spec: &Spec) -> io::Result<Ends> {
    let (mut theirs, mut ours, mut slave) = (Vec::new(), Vec::new(), None::<OwnedFd>);
    for &(fd, end) in &spec.fds {
        let fd = RawFd::try_from(fd).map_err(|_| io::ErrorKind::InvalidInput)?;
        let (child, parent) = match end {
            End::Null => (std::fs::OpenOptions::new().read(true).write(true).open("/dev/null")?.into(), None),
            End::Reads => { let (read, write) = pipe()?; (read, Some(write)) }
            End::Writes => { let (read, write) = pipe()?; (write, Some(read)) }
            End::Socket => { let (a, b) = socketpair()?; (a, Some(b)) }
            End::Pty => if let Some(slave) = &slave {
                (slave.try_clone()?, None)
            } else {
                let (master, s) = pty(spec.pty_size)?;
                slave = Some(s.try_clone()?);
                (s, Some(master))
            },
        };
        theirs.push((fd, child));
        ours.extend(parent);
    }
    Ok((theirs, ours))
}

fn c_strings(items: &[Vec<u8>]) -> io::Result<Vec<CString>> {
    items.iter().map(|item| CString::new(item.as_slice()).map_err(|_| io::ErrorKind::InvalidInput.into())).collect()
}

/// Start the child SPEC describes. The caller becomes its parent.
///
/// # Errors
/// A malformed spec (`InvalidInput`), a refused pipe, pty or fork, or the
/// child's own failure before or at its `execve`, with the system's error.
pub fn spawn(spec: &Spec) -> io::Result<Child> {
    let argv = c_strings(&spec.argv)?;
    let env = c_strings(&spec.env)?;
    let cwd = CString::new(spec.cwd.as_slice()).map_err(|_| io::ErrorKind::InvalidInput)?;
    if argv.is_empty() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let pointers = |items: &[CString]| items.iter().map(|s| s.as_ptr()).chain([std::ptr::null()]).collect::<Vec<_>>();
    let (argv, env) = (pointers(&argv), pointers(&env));
    let (theirs, ours) = ends(spec)?;
    let (report, failure) = pipe()?;
    // The child moves its descriptors above every number it must fill.
    let floor = theirs.iter().map(|(fd, _)| *fd).max().unwrap_or(2).max(2) + 1;
    let pty = spec.fds.iter().zip(&theirs).find(|((_, end), _)| *end == End::Pty).map(|(_, (_, fd))| fd.as_raw_fd());
    let plan = Plan { spec, cwd: &cwd, argv: &argv, env: &env, theirs: &theirs, failure: failure.as_raw_fd(), floor, pty };
    // SAFETY: fork in a possibly threaded process: the child runs only
    // `Plan::child`, which makes system calls alone (no allocation, no
    // lock, no unwinding) on memory prepared above, and ends in execve or
    // _exit.
    let pid = os(unsafe { libc::fork() })?;
    if pid == 0 {
        plan.child();
    }
    drop((failure, theirs));
    // SAFETY: pidfd_open takes a pid and flags. PID is this process's
    // unreaped child, so the number cannot have been reused.
    let pidfd = os(unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) }).and_then(|fd| RawFd::try_from(fd).map(owned).map_err(io::Error::other));
    // The pipe closes at the exec: end of file is success, four bytes are
    // the child's errno, and anything else is a failure of the report.
    let mut errno = [0u8; 4];
    let reported = loop {
        // SAFETY: reading up to four bytes into a four-byte buffer.
        match unsafe { libc::read(report.as_raw_fd(), errno.as_mut_ptr().cast(), 4) } {
            0 => break Ok(()),
            4 => break Err(io::Error::from_raw_os_error(i32::from_ne_bytes(errno))),
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => {}
            -1 => break Err(io::Error::last_os_error()),
            _ => break Err(io::ErrorKind::UnexpectedEof.into()),
        }
    };
    match (reported, pidfd) {
        (Ok(()), Ok(pidfd)) => Ok(Child { pid, pidfd, fds: ours }),
        (reported, pidfd) => {
            // SAFETY: killing and reaping this process's own child; a null
            // status pointer is allowed, and the wait is retried when a
            // signal interrupts it.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                while libc::waitpid(pid, std::ptr::null_mut(), 0) < 0 && *libc::__errno_location() == libc::EINTR {}
            }
            Err(reported.err().or(pidfd.err()).unwrap_or_else(|| io::ErrorKind::Other.into()))
        }
    }
}

/// What the forked child needs, prepared before the fork.
struct Plan<'a> {
    spec: &'a Spec,
    cwd: &'a CString,
    argv: &'a [*const libc::c_char],
    env: &'a [*const libc::c_char],
    theirs: &'a [(RawFd, OwnedFd)],
    failure: RawFd,
    floor: RawFd,
    pty: Option<RawFd>,
}

impl Plan<'_> {
    /// Become the program, or write the errno of what failed to the report
    /// pipe and exit.
    fn child(&self) -> ! {
        let mut report = self.failure;
        macro_rules! fail { ($errno:expr) => {{
            let errno: i32 = $errno;
            libc::write(report, (&raw const errno).cast(), 4);
            libc::_exit(127)
        }}; }
        macro_rules! check { ($call:expr) => { if $call < 0 { fail!(*libc::__errno_location()) } }; }
        // SAFETY: after fork, in the child: system calls only, on values
        // prepared before the fork and descriptors this process owns. The
        // pointers given to execve are NUL-terminated strings and
        // null-terminated arrays that outlive the call.
        unsafe {
            for signal in 1..=64 {
                libc::signal(signal, libc::SIG_DFL);
            }
            if self.spec.setsid { check!(libc::setsid()); }
            // Above every target, so no dup2 below overwrites a source.
            let failure = libc::fcntl(self.failure, libc::F_DUPFD_CLOEXEC, self.floor);
            check!(failure);
            report = failure;
            let mut moved = [0; 64];
            if self.theirs.len() > moved.len() { fail!(libc::E2BIG) }
            for (index, (_, fd)) in self.theirs.iter().enumerate() {
                moved[index] = libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, self.floor);
                check!(moved[index]);
            }
            let mut pty = -1;
            for (index, (target, fd)) in self.theirs.iter().enumerate() {
                check!(libc::dup2(moved[index], *target));
                if Some(fd.as_raw_fd()) == self.pty && pty < 0 { pty = *target; }
            }
            if self.spec.setsid && pty >= 0 { check!(libc::ioctl(pty, libc::TIOCSCTTY, 0)); }
            // Close all but the targets (dup2 cleared their close-on-exec)
            // and the report pipe, which closes at the exec.
            let mut low = 0;
            loop {
                let mut next = failure;
                for (target, _) in self.theirs { if *target >= low && *target < next { next = *target; } }
                if next > low { libc::close_range(low.cast_unsigned(), (next - 1).cast_unsigned(), 0); }
                if next == failure { break; }
                low = next + 1;
            }
            libc::close_range((failure + 1).cast_unsigned(), u32::MAX, 0);
            check!(libc::chdir(self.cwd.as_ptr()));
            libc::umask(self.spec.umask);
            for &(resource, soft, hard) in &self.spec.limits {
                check!(libc::setrlimit(resource as _, &libc::rlimit { rlim_cur: soft, rlim_max: hard }));
            }
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut mask);
            for signal in 1..=64 {
                if self.spec.sigmask >> (signal - 1) & 1 == 1 { libc::sigaddset(&raw mut mask, signal); }
            }
            check!(libc::sigprocmask(libc::SIG_SETMASK, &raw const mask, std::ptr::null_mut()));
            libc::execve(self.argv[0], self.argv.as_ptr(), self.env.as_ptr());
            fail!(*libc::__errno_location())
        }
    }
}

/// What a parent last learned of a child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Exited(i32),
    /// Killed by this signal.
    Signaled(i32),
    /// Stopped by this signal.
    Stopped(i32),
    Continued,
}

impl Status {
    #[must_use]
    pub fn ended(self) -> bool { matches!(self, Status::Exited(_) | Status::Signaled(_)) }

    #[must_use]
    pub fn write(self, w: Writer) -> Writer {
        let (tag, value) = match self {
            Status::Running => (0, 0),
            Status::Exited(code) => (1, code),
            Status::Signaled(signal) => (2, signal),
            Status::Stopped(signal) => (3, signal),
            Status::Continued => (4, 0),
        };
        w.u8(tag).u32(value.cast_unsigned())
    }

    /// # Errors
    /// [`Error::Truncated`].
    pub fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let (tag, value) = (r.u8()?, r.u32()?.cast_signed());
        Ok(match tag {
            1 => Status::Exited(value),
            2 => Status::Signaled(value),
            3 => Status::Stopped(value),
            4 => Status::Continued,
            _ => Status::Running,
        })
    }
}

/// How `waitid` names whom to wait for.
#[derive(Clone, Copy)]
pub enum Whom<'a> {
    /// Any child, left waitable: a look at who changed.
    PeekAny,
    Child(&'a OwnedFd),
    Pid(i32),
}

/// A child's change of state, if one is pending: its pid and new status.
/// Never blocks. With [`Whom::Child`] and [`Whom::Pid`] the change is
/// consumed, and an exit reaps the child.
#[must_use]
pub fn wait(whom: Whom<'_>) -> Option<(i32, Status)> {
    let (kind, id, peek) = match whom {
        Whom::PeekAny => (libc::P_ALL, 0, libc::WNOWAIT),
        Whom::Child(pidfd) => (libc::P_PIDFD, pidfd.as_raw_fd().cast_unsigned(), 0),
        Whom::Pid(pid) => (libc::P_PID, pid.cast_unsigned(), 0),
    };
    // SAFETY: a zeroed siginfo is valid; waitid fills it or leaves si_pid 0.
    let info = unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let options = libc::WNOHANG | libc::WEXITED | libc::WSTOPPED | libc::WCONTINUED | peek;
        if libc::waitid(kind, id, &raw mut info, options) != 0 { return None; }
        info
    };
    // SAFETY: waitid succeeded, so the SIGCHLD fields of the union are set.
    let (pid, value) = unsafe { (info.si_pid(), info.si_status()) };
    let status = match info.si_code {
        _ if pid == 0 => return None,
        libc::CLD_EXITED => Status::Exited(value),
        libc::CLD_KILLED | libc::CLD_DUMPED => Status::Signaled(value),
        libc::CLD_STOPPED | libc::CLD_TRAPPED => Status::Stopped(value),
        _ => Status::Continued,
    };
    Some((pid, status))
}

/// Send SIGNAL to the process PIDFD names, or to its process group when
/// GROUP names the group's leader pid (the child of a spec with a session).
///
/// # Errors
/// The system's refusal; `ESRCH` once the process is gone.
pub fn signal(pidfd: &OwnedFd, group: Option<i32>, signal: i32) -> io::Result<()> {
    // SAFETY: both calls take integers and a null siginfo. The pidfd names
    // one process for good; the group call is made by the unreaped
    // leader's parent, so the pid still names that group.
    os(unsafe {
        match group {
            Some(leader) => libc::kill(-leader, signal),
            None => libc::c_int::from(libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), signal, std::ptr::null::<libc::siginfo_t>(), 0) < 0).wrapping_neg(),
        }
    }).map(drop)
}
