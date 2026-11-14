//! The C library calls host.rs makes that no crate of the workspace covers
//! safely: rustix has no user database, posix_spawn, getaddrinfo, signal
//! handler, fork or sysconf, and its kill and pidfd_send_signal take only
//! named signals and no flags. Each is a small safe function around one
//! unsafe call, so this file is the whole of the host boundary's unsafe
//! code. host.rs records, refuses and replays; these only call.
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::time::Duration;

/// A C call's 0-or-error result.
fn status(result: libc::c_int) -> io::Result<()> {
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// close(2) of FD, with its error, which dropping an OwnedFd discards.
/// FD is the caller's to give up: an OwnedFd's, or a descriptor no one
/// else will use again.
pub(super) fn close(fd: RawFd) -> io::Result<()> {
    // SAFETY: close takes only a number; the caller owns FD and never uses
    // it again, so no other owner's descriptor is closed under it.
    status(unsafe { libc::close(fd) })
}

/// close(2) of an owned descriptor, with its error.
pub(super) fn close_owned(fd: OwnedFd) -> io::Result<()> {
    close(fd.into_raw_fd())
}

/// ioprio_set(2) of the calling thread to IO (class << 13 | level); best
/// effort. rustix has no ioprio call.
pub(super) fn set_io_priority(io: libc::c_long) {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    // SAFETY: ioprio_set takes three integers and reads no memory.
    unsafe { libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0 as libc::c_long, io) };
}

/// sysconf(3) of NAME: the C library's answer, which for the processor
/// counts and ARG_MAX no kernel interface rustix wraps gives.
pub(super) fn sysconf(name: libc::c_int) -> libc::c_long {
    // SAFETY: sysconf takes an integer and reads no memory of ours.
    unsafe { libc::sysconf(name) }
}

// User database: NSS is the C library's, so these go through it.

/// A passwd entry's fields, as the C library's bytes.
pub struct Passwd {
    pub uid: u32,
    /// pw_name, absent when the entry has none.
    pub name: Option<Vec<u8>>,
    pub gecos: Vec<u8>,
    /// pw_dir, absent when the entry has none.
    pub dir: Option<Vec<u8>>,
}

/// The fields of ENTRY, copied out of the buffer its strings point into.
fn passwd_fields(entry: &libc::passwd) -> Passwd {
    let field = |p: *const libc::c_char| {
        // SAFETY: a non-null field of an entry the lookup filled points at
        // a NUL-terminated string in its buffer, which outlives ENTRY's use.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    };
    Passwd { uid: entry.pw_uid, name: field(entry.pw_name), gecos: field(entry.pw_gecos).unwrap_or_default(), dir: field(entry.pw_dir) }
}

/// getpwuid_r (KEY a uid) or getpwnam_r (a name) into a BUFFER-byte
/// buffer, so NSS sources count: the entry, or the status, 0 when there is
/// none.
pub(super) fn passwd(key: Result<u32, &CStr>, buffer: usize) -> Result<Passwd, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: ENTRY, BUFFER (with its length) and FOUND are live locals the
    // lookup writes; NAME is NUL-terminated. FOUND is ENTRY or null after.
    let status = unsafe {
        match key {
            Ok(uid) => libc::getpwuid_r(uid, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found),
            Err(name) => libc::getpwnam_r(name.as_ptr(), entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found),
        }
    };
    if status != 0 || found.is_null() {
        return Err(status);
    }
    // SAFETY: a non-null FOUND is ENTRY, which the lookup filled.
    Ok(passwd_fields(unsafe { entry.assume_init_ref() }))
}

/// getgrgid_r (KEY a gid) or getgrnam_r (a name) into a BUFFER-byte
/// buffer: the group's name, if it has one, and gid, or the status, 0 when
/// there is none.
pub(super) fn group(key: Result<u32, &CStr>, buffer: usize) -> Result<(Option<Vec<u8>>, u32), i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut found: *mut libc::group = std::ptr::null_mut();
    // SAFETY: as in `passwd`: live locals the lookup writes, a
    // NUL-terminated name; a non-null FOUND is the filled ENTRY, whose
    // non-null name points into BUFFER.
    unsafe {
        let status = match key {
            Ok(gid) => libc::getgrgid_r(gid, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found),
            Err(name) => libc::getgrnam_r(name.as_ptr(), entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found),
        };
        if status != 0 || found.is_null() {
            return Err(status);
        }
        let name = (*found).gr_name;
        Ok(((!name.is_null()).then(|| CStr::from_ptr(name).to_bytes().to_vec()), (*found).gr_gid))
    }
}

/// The names getpwent (users) or getgrent (groups) walks the whole database
/// for, in its order, between setpwent and endpwent (setgrent, endgrent).
/// The cursor is the C library's, shared by the process: the caller holds
/// one walk at a time.
pub(super) fn account_names(groups: bool) -> Vec<Vec<u8>> {
    let mut names = Vec::new();
    // SAFETY: the walk's calls take no arguments; each entry's name is read
    // before the next call reuses the C library's static entry, and the
    // caller serializes walks.
    unsafe {
        if groups { libc::setgrent() } else { libc::setpwent() }
        loop {
            let name = if groups { libc::getgrent().as_ref().map(|g| g.gr_name) } else { libc::getpwent().as_ref().map(|p| p.pw_name) };
            let Some(name) = name else { break };
            if !name.is_null() { names.push(CStr::from_ptr(name).to_bytes().to_vec()); }
        }
        if groups { libc::endgrent() } else { libc::endpwent() }
    }
    names
}

// Counters and clocks.

/// A pinned counter of the calling thread's user-space instructions
/// (perf_event_open, which rustix lacks); None where the kernel refuses.
pub(super) fn open_instruction_counter() -> Option<OwnedFd> {
    // perf_event_attr, PERF_ATTR_SIZE_VER0: type HARDWARE (0) and size,
    // config INSTRUCTIONS (1), flags pinned | exclude_kernel | exclude_hv.
    const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 8;
    let mut attr = [0u64; 8];
    attr[0] = 64 << 32;
    attr[1] = 1;
    attr[5] = 1 << 2 | 1 << 5 | 1 << 6;
    // SAFETY: ATTR is a PERF_ATTR_SIZE_VER0 struct that outlives the call;
    // a non-negative result is a new descriptor this thread owns.
    unsafe {
        let fd = libc::syscall(libc::SYS_perf_event_open, attr.as_ptr(), 0, -1, -1, PERF_FLAG_FD_CLOEXEC);
        (fd >= 0).then(|| OwnedFd::from_raw_fd(fd as RawFd))
    }
}

/// getrusage(RUSAGE_SELF)'s user and system times: rustix reads usage only
/// of a waited child.
pub(super) fn self_usage() -> io::Result<(Duration, Duration)> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage writes the whole struct it is given, which is
    // initialized once it returns 0.
    let usage = unsafe {
        if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) < 0 {
            return Err(io::Error::last_os_error());
        }
        usage.assume_init()
    };
    let part = |t: libc::timeval| Duration::new(t.tv_sec.max(0) as u64, (t.tv_usec.clamp(0, 999_999) as u32) * 1000);
    Ok((part(usage.ru_utime), part(usage.ru_stime)))
}

/// The CPU-time clock of the thread that made it, readable from any thread
/// (pthread_getcpuclockid): rustix reads only the caller's own.
#[derive(Clone, Copy)]
pub(super) struct ThreadCpuClock(libc::clockid_t);

impl ThreadCpuClock {
    pub(super) fn of_this_thread() -> io::Result<Self> {
        let mut clock: libc::clockid_t = 0;
        // SAFETY: CLOCK is a live local; pthread_self is the calling thread.
        match unsafe { libc::pthread_getcpuclockid(libc::pthread_self(), &mut clock) } {
            0 => Ok(Self(clock)),
            error => Err(io::Error::from_raw_os_error(error)),
        }
    }

    /// Seconds and nanoseconds; None once the thread is gone.
    pub(super) fn now(self) -> Option<(i64, i64)> {
        let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: clock_gettime fills TIME when it returns 0.
        unsafe { (libc::clock_gettime(self.0, time.as_mut_ptr()) == 0).then(|| { let t = time.assume_init(); (t.tv_sec, t.tv_nsec) }) }
    }
}

// Files.

/// fchmodat: PATH's mode bits, a symlink's own under NOFOLLOW. The C
/// library emulates NOFOLLOW where the kernel lacks fchmodat2; rustix
/// refuses it.
pub(super) fn chmod(path: &CStr, mode: u32, nofollow: bool) -> io::Result<()> {
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    // SAFETY: PATH is NUL-terminated and only read.
    status(unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, flags) })
}

/// mkostemp: a new close-on-exec file named TEMPLATE (NUL-free) with its
/// trailing XXXXXX replaced, and that name.
pub(super) fn temp_file(template: &[u8]) -> io::Result<(Vec<u8>, File)> {
    let mut name = CString::new(template).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?.into_bytes_with_nul();
    // SAFETY: NAME is a NUL-terminated buffer mkostemp rewrites in place;
    // a non-negative result is a new descriptor this process owns.
    let file = unsafe {
        let fd = libc::mkostemp(name.as_mut_ptr().cast(), libc::O_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        File::from_raw_fd(fd)
    };
    name.pop();
    Ok((name, file))
}

// Processes.

/// GNU's emacs_spawn by posix_spawn, as `host::spawn_child` describes:
/// ARGV run with ENV in DIR, FDS as its standard streams, in a session of
/// its own, with SIGINT, SIGQUIT, SIGPROF and SIGPIPE at their defaults and
/// no signal blocked. Its pid.
pub(super) fn spawn(argv: &[CString], env: &[CString], dir: &CStr, fds: [RawFd; 3]) -> io::Result<i32> {
    let invalid = || io::Error::from_raw_os_error(libc::EINVAL);
    let program = argv.first().ok_or_else(invalid)?;
    let pointers = |list: &[CString]| list.iter().map(|c| c.as_ptr().cast_mut()).chain([std::ptr::null_mut()]).collect::<Vec<_>>();
    let (argp, envp) = (pointers(argv), pointers(env));
    let ok = |r: libc::c_int| if r == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(r)) };
    // SAFETY: the actions and attributes are initialized before use and
    // destroyed after; every pointer passed outlives the call.
    unsafe {
        let mut actions = std::mem::MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
        ok(libc::posix_spawn_file_actions_init(actions.as_mut_ptr()))?;
        let mut attr = std::mem::MaybeUninit::<libc::posix_spawnattr_t>::uninit();
        if let Err(error) = ok(libc::posix_spawnattr_init(attr.as_mut_ptr())) {
            libc::posix_spawn_file_actions_destroy(actions.as_mut_ptr());
            return Err(error);
        }
        let (actions, attr) = (actions.as_mut_ptr(), attr.as_mut_ptr());
        let prepared = (|| -> io::Result<()> {
            for (target, fd) in fds.into_iter().enumerate() {
                ok(libc::posix_spawn_file_actions_adddup2(actions, fd, target as libc::c_int))?;
            }
            ok(libc::posix_spawn_file_actions_addchdir_np(actions, dir.as_ptr()))?;
            let mut defaults: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut defaults);
            for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGPROF, libc::SIGPIPE] {
                libc::sigaddset(&mut defaults, signal);
            }
            ok(libc::posix_spawnattr_setsigdefault(attr, &defaults))?;
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            ok(libc::posix_spawnattr_setsigmask(attr, &mask))?;
            let flags = libc::POSIX_SPAWN_SETSID as libc::c_int | libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK;
            ok(libc::posix_spawnattr_setflags(attr, flags as libc::c_short))
        })();
        let mut pid: libc::pid_t = 0;
        let spawned = prepared.and_then(|()| ok(libc::posix_spawn(&mut pid, program.as_ptr(), actions, attr, argp.as_ptr(), envp.as_ptr())));
        libc::posix_spawnattr_destroy(attr);
        libc::posix_spawn_file_actions_destroy(actions);
        spawned.map(|()| pid)
    }
}

/// Remove NAME from the environment. The caller runs before any other
/// thread starts.
pub(super) fn remove_env(name: &str) {
    // SAFETY: the process is single threaded here (the caller's contract),
    // so no other thread reads the environment while it changes.
    unsafe { std::env::remove_var(name) };
}

/// kill(2): SIGNAL, any number (0 only asks), to PID, a group when
/// negative. rustix takes only named signals.
pub(super) fn kill(pid: i32, signal: i32) -> io::Result<()> {
    // SAFETY: kill takes two integers and reads no memory.
    status(unsafe { libc::kill(pid, signal) })
}

/// pidfd_send_signal(2) with FLAGS, which rustix's takes none of.
pub(super) fn pidfd_send_signal(pidfd: BorrowedFd<'_>, signal: i32, flags: libc::c_uint) -> io::Result<()> {
    // SAFETY: a null siginfo asks for the kernel's default; the rest are
    // integers.
    status(unsafe { libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), signal, std::ptr::null::<libc::siginfo_t>(), flags) } as libc::c_int)
}

/// fork(2): 0 in the child, else its pid. The caller states what the
/// child may run, since it has only the calling thread.
pub(super) fn fork() -> io::Result<i32> {
    // SAFETY: fork takes no arguments; each caller (`host::fork_isolate`,
    // `host::fork_threaded`) limits its child to what a one-thread copy of
    // the process may do.
    let pid = unsafe { libc::fork() };
    if pid < 0 { Err(io::Error::last_os_error()) } else { Ok(pid) }
}

/// End this process now with CODE: no destructor, atexit handler or
/// buffered output of it runs.
pub(super) fn exit_at_once(code: i32) -> ! {
    // SAFETY: `_exit' takes only a status; it ends the process at once, and
    // nothing of this process (destructor, atexit handler, other thread) runs
    // after it. No safe std call exits without running atexit handlers.
    unsafe { libc::_exit(code) }
}

// Signals.

/// Install HANDLER for SIGNAL with FLAGS (`SA_RESTART` or 0) and an empty
/// mask. HANDLER may make only async-signal-safe calls.
pub(super) fn catch_signal(signal: i32, handler: extern "C" fn(libc::c_int), flags: libc::c_int) -> io::Result<()> {
    // SAFETY: an all-zero sigaction is valid, its mask is then emptied, and
    // HANDLER is a plain C function, as sa_sigaction expects without
    // SA_SIGINFO.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        action.sa_flags = flags;
        libc::sigemptyset(&mut action.sa_mask);
        status(libc::sigaction(signal, &action, std::ptr::null_mut()))
    }
}

/// One byte to FD, a nonblocking pipe end a signal handler holds as a raw
/// number; async-signal-safe, and a full pipe already wakes its reader.
pub(super) fn wake(fd: RawFd) {
    // SAFETY: write reads one byte of a live array; FD stays open for the
    // process's life or until its owner swaps it out first.
    let _ = unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) };
}

/// Read FD, a nonblocking pipe end held as a raw number, until empty.
pub(super) fn drain(fd: RawFd) {
    let mut bytes = [0u8; 64];
    // SAFETY: read writes at most the length of the live array it is given.
    while unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) } > 0 {}
}

/// alarm(2), async-signal-safe: SIGALRM in SECONDS, whose default action
/// ends the process.
pub(super) fn alarm(seconds: u32) {
    // SAFETY: alarm takes an integer and reads no memory.
    unsafe { libc::alarm(seconds) };
}

/// F with every signal blocked on this thread, the mask put back however F
/// ends: rustix's sigprocmask is for C runtimes only.
pub(super) fn with_signals_blocked<T>(f: impl FnOnce() -> T) -> T {
    /// The mask before, put back however F ends.
    struct Restore(libc::sigset_t);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: the mask pthread_sigmask filled in below, set back on the
            // thread it came from; a null old set asks for nothing back.
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut()) };
        }
    }
    let mut all = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    let mut old = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: both write only the locals they are given; SIG_SETMASK is a valid
    // `how', so pthread_sigmask has filled OLD before it is read.
    let _restore = unsafe {
        libc::sigfillset(all.as_mut_ptr());
        libc::pthread_sigmask(libc::SIG_SETMASK, all.as_ptr(), old.as_mut_ptr());
        Restore(old.assume_init())
    };
    f()
}

// Sockets.

/// getaddrinfo for NAME, datagram sockets of FAMILY, numeric only with
/// NUMERIC: its IPv4 and IPv6 addresses in its order, or gai_strerror's
/// reason. std's resolver takes no family or flags.
pub(super) fn lookup_addresses(name: &CStr, family: i32, numeric: bool) -> Result<Vec<std::net::IpAddr>, String> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    // SAFETY: an all-zero addrinfo is the documented "no hints" value;
    // NAME and HINTS outlive the call; each node of the list getaddrinfo
    // returns holds a sockaddr of its family; the list is freed once.
    unsafe {
        let mut hints: libc::addrinfo = std::mem::zeroed();
        hints.ai_family = family;
        hints.ai_socktype = libc::SOCK_DGRAM;
        if numeric {
            hints.ai_flags = libc::AI_NUMERICHOST;
        }
        let mut res: *mut libc::addrinfo = std::ptr::null_mut();
        let ret = libc::getaddrinfo(name.as_ptr(), std::ptr::null(), &hints, &mut res);
        if ret != 0 {
            return Err(CStr::from_ptr(libc::gai_strerror(ret)).to_string_lossy().into_owned());
        }
        let mut addresses = Vec::new();
        let mut entry = res;
        while let Some(ai) = entry.as_ref() {
            match ai.ai_family {
                libc::AF_INET => {
                    let sin = &*(ai.ai_addr as *const libc::sockaddr_in);
                    addresses.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))));
                }
                libc::AF_INET6 => {
                    let sin6 = &*(ai.ai_addr as *const libc::sockaddr_in6);
                    addresses.push(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr)));
                }
                _ => {}
            }
            entry = ai.ai_next;
        }
        libc::freeaddrinfo(res);
        Ok(addresses)
    }
}

/// accept4 on the listening socket LISTENER: the connection, close-on-exec,
/// and the peer's raw address with its length.
pub(super) fn accept(listener: RawFd) -> io::Result<(OwnedFd, libc::sockaddr_storage, libc::socklen_t)> {
    let mut storage = std::mem::MaybeUninit::<libc::sockaddr_storage>::zeroed();
    let mut len = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: an all-zero sockaddr_storage is valid; accept4 writes at most
    // LEN bytes of it and LEN back; a non-negative result is a new
    // descriptor this process owns.
    unsafe {
        let fd = libc::accept4(listener, storage.as_mut_ptr().cast(), &mut len, libc::SOCK_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((OwnedFd::from_raw_fd(fd), storage.assume_init(), len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_keeps_absent_fields_absent() {
        let null = std::ptr::null_mut();
        let entry = libc::passwd { pw_name: null, pw_passwd: null, pw_uid: 0, pw_gid: 0, pw_gecos: c"Full Name".as_ptr().cast_mut(), pw_dir: null, pw_shell: null };
        let entry = passwd_fields(&entry);
        assert_eq!((entry.name, entry.gecos, entry.dir), (None, b"Full Name".to_vec(), None));
    }
}
