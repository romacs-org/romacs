//! The host boundary: the VM's and its embedders' host queries and effects
//! (files, processes, sockets) go through this module, so a world builder can
//! later record and classify each one. `tests/host_boundary_lint.rs` keeps
//! other sources from calling the host APIs it names directly.
//!
//! No read descriptor leaves this module (`Input`). Output files, pipes,
//! ptys, sockets and children are made only here, so a refusal at these entry
//! points also covers the descriptor calls made on them elsewhere.
//!
//! Only `Mode::Off` exists, in which each function is a direct passthrough:
//! it returns exactly what the host call returns.
use std::cell::Cell;
use std::ffi::{CStr, OsStr, OsString};
use std::fs::{File, FileType, Metadata};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

pub use std::fs::OpenOptions;

/// How the boundary treats host access. Per thread: tests run VMs on
/// parallel threads, and some callers have no `Vm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
}

thread_local! {
    static MODE: Cell<Mode> = const { Cell::new(Mode::Off) };
}

pub fn mode() -> Mode {
    MODE.with(Cell::get)
}

// Environment.

pub fn getenv(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The process environment, in the host's order.
pub fn env_snapshot() -> Vec<(OsString, OsString)> {
    std::env::vars_os().collect()
}

/// Which Lisp list a Rust environment lookup walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvList {
    /// `process-environment`.
    Process,
    /// `initial-environment`.
    Initial,
    /// A list the caller passed.
    Explicit,
}

/// A Rust lookup of NAME in a Lisp environment list. The lookup itself reads
/// Lisp data; this marks it as a host fact. A no-op in `Mode::Off`.
pub fn lisp_env(_list: EnvList, _name: &[u8]) {}

// Files.

pub fn read_file(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// The boolean path probes of `std::path::Path`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Exists,
    IsFile,
    IsDir,
}

pub fn probe(kind: Probe, path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match kind {
        Probe::Exists => path.exists(),
        Probe::IsFile => path.is_file(),
        Probe::IsDir => path.is_dir(),
    }
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    std::fs::metadata(path)
}

pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    std::fs::symlink_metadata(path)
}

/// fstat on an output file this module opened.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    file.metadata()
}

/// A file opened for reading. Its reads and queries go through this module,
/// so a recorder sees the bytes a caller consumes and each error; the
/// descriptor never leaves it.
pub struct Input(File);

/// open(2) of PATH for reading, with FLAGS (`O_NONBLOCK`, `O_NOFOLLOW` …).
pub fn open_input(path: impl AsRef<Path>, flags: i32) -> io::Result<Input> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new().read(true).custom_flags(flags).open(path).map(Input)
}

/// The kernel's random source, `/dev/urandom`.
pub fn random_source() -> io::Result<Input> {
    File::open("/dev/urandom").map(Input)
}

impl Input {
    /// fstat.
    pub fn stat(&self) -> io::Result<Metadata> {
        self.0.metadata()
    }

    /// At most LIMIT bytes from OFFSET: a seek, then reads until LIMIT or the
    /// end. An error from either discards what was read.
    pub fn read_range(&mut self, offset: u64, limit: u64) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        self.0.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        (&mut self.0).take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// One read into BUFFER: the count, 0 at the end.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, buffer)
    }

    pub fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        io::Read::read_exact(&mut self.0, buffer)
    }

    /// Whether the file carries an extended (POSIX or NFSv4) ACL.
    pub fn has_acl(&self) -> bool {
        use std::os::fd::AsRawFd;
        [c"system.posix_acl_access", c"system.nfs4_acl"].iter().any(|attribute| {
            let size = unsafe { libc::fgetxattr(self.0.as_raw_fd(), attribute.as_ptr(), std::ptr::null_mut(), 0) };
            size >= 0
        })
    }

    /// close(2), with its error.
    pub fn close(self) -> io::Result<()> {
        use std::os::fd::IntoRawFd;
        if unsafe { libc::close(self.0.into_raw_fd()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// A file a child process reads as its standard input: this process reads
/// none of it.
pub fn open_child_input(path: impl AsRef<Path>) -> io::Result<OwnedFd> {
    File::open(path).map(OwnedFd::from)
}

pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::read_link(path)
}

/// The entries of PATH, as the host yields them.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    std::fs::read_dir(path).map(ReadDir)
}

/// A directory listing whose entries answer their queries here.
pub struct ReadDir(std::fs::ReadDir);

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|entry| entry.map(DirEntry))
    }
}

pub struct DirEntry(std::fs::DirEntry);

impl DirEntry {
    pub fn file_name(&self) -> OsString {
        self.0.file_name()
    }

    pub fn path(&self) -> PathBuf {
        self.0.path()
    }

    /// The type the listing gave, else an lstat's.
    pub fn file_type(&self) -> io::Result<FileType> {
        self.0.file_type()
    }
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// PATH made absolute against the working directory, lexically.
pub fn absolute(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::path::absolute(path)
}

/// Whose ids an access check uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ids {
    Effective,
    Real,
}

/// faccessat: whether IDS may access PATH in MODE.
pub fn access(path: &CStr, mode: i32, ids: Ids) -> io::Result<()> {
    let flags = match ids { Ids::Effective => libc::AT_EACCESS, Ids::Real => 0 };
    status(unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, flags) })
}

/// A C call's 0-or-error result.
fn status(result: libc::c_int) -> io::Result<()> {
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn statvfs(path: &CStr) -> io::Result<libc::statvfs> {
    let mut usage = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), usage.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { usage.assume_init() })
}

// Process facts.

pub fn current_dir() -> io::Result<PathBuf> {
    std::env::current_dir()
}

pub fn current_exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}

pub fn pid() -> u32 {
    std::process::id()
}

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn euid() -> u32 {
    unsafe { libc::geteuid() }
}

// User database.

/// A passwd entry's fields, as the C library's bytes.
pub struct Passwd {
    /// pw_name, absent when the entry has none.
    pub name: Option<Vec<u8>>,
    pub gecos: Vec<u8>,
    /// pw_dir, absent when the entry has none.
    pub dir: Option<Vec<u8>>,
}

/// The C library's lookup into a BUFFER-byte buffer (so NSS sources count):
/// the entry, or the status, 0 when there is none.
fn passwd(buffer: usize, lookup: impl Fn(*mut libc::passwd, *mut libc::c_char, usize, *mut *mut libc::passwd) -> libc::c_int) -> Result<Passwd, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = lookup(&mut entry, buffer.as_mut_ptr(), buffer.len(), &mut found);
    if status != 0 || found.is_null() {
        return Err(status);
    }
    let bytes = |p: *const libc::c_char| if p.is_null() { Vec::new() } else { unsafe { CStr::from_ptr(p) }.to_bytes().to_vec() };
    let field = |p: *const libc::c_char| (!p.is_null()).then(|| bytes(p));
    Ok(Passwd { name: field(entry.pw_name), gecos: bytes(entry.pw_gecos), dir: field(entry.pw_dir) })
}

pub fn user_by_uid(uid: u32, buffer: usize) -> Result<Passwd, i32> {
    passwd(buffer, |entry, buffer, size, found| unsafe { libc::getpwuid_r(uid, entry, buffer, size, found) })
}

pub fn user_by_name(name: &CStr, buffer: usize) -> Result<Passwd, i32> {
    passwd(buffer, |entry, buffer, size, found| unsafe { libc::getpwnam_r(name.as_ptr(), entry, buffer, size, found) })
}

/// getgrgid_r into a BUFFER-byte buffer: the group's name, or the status, 0
/// when there is none.
pub fn group_by_gid(gid: u32, buffer: usize) -> Result<Vec<u8>, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut found = std::ptr::null_mut();
    let status = unsafe { libc::getgrgid_r(gid, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found) };
    if status != 0 || found.is_null() || unsafe { (*found).gr_name }.is_null() {
        return Err(status);
    }
    Ok(unsafe { CStr::from_ptr((*found).gr_name) }.to_bytes().to_vec())
}

// Machine.

/// The kernel nodename, which GNU's gethostname returns, read without an
/// unsafe syscall. HOSTNAME and DNS are not consulted.
pub fn system_name() -> io::Result<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
}

/// gnulib nproc's processor sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Processors {
    /// NPROC_CURRENT: those this process may run on.
    Current,
    /// The online processors.
    Online,
    /// NPROC_ALL: the configured processors.
    All,
}

pub fn processors(set: Processors) -> u64 {
    let name = match set {
        Processors::Current => {
            let mut cpus: libc::cpu_set_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut cpus) } == 0 {
                let count = unsafe { libc::CPU_COUNT(&cpus) };
                if count > 0 { return count as u64; }
            }
            libc::_SC_NPROCESSORS_ONLN
        }
        Processors::Online => libc::_SC_NPROCESSORS_ONLN,
        Processors::All => libc::_SC_NPROCESSORS_CONF,
    };
    u64::try_from(unsafe { libc::sysconf(name) }).ok().filter(|&n| n > 0).unwrap_or(1)
}

// Time.

/// A time-zone file's bytes: `/etc/localtime`, a TZ file or a zoneinfo entry.
pub fn zone_file(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// The wall clock.
pub fn clock() -> SystemTime {
    SystemTime::now()
}

/// The monotonic clock, for deadlines and measurement.
pub fn monotonic() -> Instant {
    Instant::now()
}

// Effects.

/// Four bytes of kernel entropy.
pub fn random_seed() -> io::Result<[u8; 4]> {
    let mut bytes = [0u8; 4];
    random_source()?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// PATH opened with OPTIONS for writing only: reads go through `Input`.
pub fn open_output(path: impl AsRef<Path>, options: &OpenOptions) -> io::Result<File> {
    options.clone().read(false).write(true).open(path)
}

/// GNU copy-file's fast paths: share SOURCE's extents with DEST, else
/// copy_file_range until it stops. False leaves the rest to read+write.
pub fn clone_file(dest: &File, source: &Input) -> bool {
    use std::os::fd::AsRawFd;
    if unsafe { libc::ioctl(dest.as_raw_fd(), libc::FICLONE, source.0.as_raw_fd()) } == 0 {
        return true;
    }
    const COPY_MAX: usize = (isize::MAX as usize) >> 30 << 30;
    while unsafe { libc::copy_file_range(source.0.as_raw_fd(), std::ptr::null_mut(), dest.as_raw_fd(), std::ptr::null_mut(), COPY_MAX, 0) } > 0 {}
    false
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    std::fs::write(path, bytes)
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    std::fs::rename(from, to)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_file(path)
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::remove_dir(path)
}

/// mkdir with MODE before the umask.
pub fn create_dir(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    std::fs::hard_link(original, link)
}

pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

pub fn fchown(file: &File, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    std::os::unix::fs::fchown(file, uid, gid)
}

/// fchmodat: PATH's mode bits, a symlink's own under NOFOLLOW.
pub fn chmod(path: &CStr, mode: u32, nofollow: bool) -> io::Result<()> {
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    status(unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, flags) })
}

/// renameat2 with RENAME_NOREPLACE: EEXIST rather than replace TO.
pub fn rename_noreplace(from: &CStr, to: &CStr) -> io::Result<()> {
    status(unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) })
}

/// utimensat: both of PATH's times set to TIME (seconds, nanoseconds), or
/// now; a symlink's own under NOFOLLOW.
pub fn set_times(path: &CStr, time: Option<(i64, i64)>, nofollow: bool) -> io::Result<()> {
    let stamp = match time {
        Some((seconds, nanos)) => libc::timespec { tv_sec: seconds as libc::time_t, tv_nsec: nanos as _ },
        None => libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_NOW },
    };
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    status(unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), [stamp, stamp].as_ptr(), flags) })
}

/// mkostemp: a new close-on-exec file named TEMPLATE with its trailing
/// XXXXXX replaced, and that name.
pub fn temp_file(template: &Path) -> io::Result<(PathBuf, File)> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut name = template.as_os_str().as_bytes().to_vec();
    name.push(0);
    let fd = unsafe { libc::mkostemp(name.as_mut_ptr().cast(), libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    name.pop();
    Ok((PathBuf::from(OsString::from_vec(name)), file))
}

// Processes. A child, and every descriptor this process holds besides its
// standard streams, comes from a function here: the descriptor and child
// calls elsewhere (read, write, poll, fcntl, ioctl, the termios calls, fsync,
// close, waitpid, shutdown, accept) act only on what these returned.

/// Every child process starts from a Command made here.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

/// kill(2): SIGNAL to PID, a process group when negative. Signal 0 asks
/// only whether PID exists and may be signalled.
pub fn kill(pid: i32, signal: i32) -> io::Result<()> {
    status(unsafe { libc::kill(pid, signal) })
}

/// A pidfd for PID, readable once it exits.
pub fn pidfd_open(pid: i32) -> Option<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}

/// emacs_pipe: a close-on-exec pipe (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    status(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// A close-on-exec, nonblocking eventfd: a wakeup counter no file names.
pub fn eventfd() -> io::Result<OwnedFd> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// allocate_pty with posix_openpt: the master and the slave's name.
pub fn allocate_pty() -> io::Result<(OwnedFd, Vec<u8>)> {
    let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name = [0u8; 64];
    if unsafe { libc::ptsname_r(fd, name.as_mut_ptr().cast(), name.len()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    Ok((master, name[..len].to_vec()))
}

/// The pty slave NAME, opened without becoming the controlling terminal.
pub fn open_tty(name: &[u8]) -> io::Result<OwnedFd> {
    let path = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

// Sockets.

/// HOST:PORT's addresses, from the C library's resolver.
pub fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    use std::net::ToSocketAddrs;
    (host, port).to_socket_addrs().map(Iterator::collect)
}

pub fn listen(addr: SocketAddr) -> io::Result<TcpListener> {
    TcpListener::bind(addr)
}

pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    TcpStream::connect(addr)
}

/// connect_network_socket's nonblocking connect: a close-on-exec stream
/// socket to ADDR, connected or (EINPROGRESS) connecting.
pub fn connect_nonblocking(addr: SocketAddr) -> io::Result<OwnedFd> {
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        SocketAddr::V4(a) => {
            let sin = unsafe { &mut *(&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in>() };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_port = a.port().to_be();
            sin.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(a.ip().octets()) };
            size_of::<libc::sockaddr_in>()
        }
        SocketAddr::V6(a) => {
            let sin6 = unsafe { &mut *(&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in6>() };
            sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sin6.sin6_port = a.port().to_be();
            sin6.sin6_flowinfo = a.flowinfo();
            sin6.sin6_addr = libc::in6_addr { s6_addr: a.ip().octets() };
            sin6.sin6_scope_id = a.scope_id();
            size_of::<libc::sockaddr_in6>()
        }
    };
    let s = unsafe { libc::socket(i32::from(storage.ss_family), libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if s < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(s) };
    if unsafe { libc::connect(s, (&storage as *const libc::sockaddr_storage).cast(), len as libc::socklen_t) } != 0 {
        let error = io::Error::last_os_error();
        // EINTR leaves a nonblocking connect in progress, as EINPROGRESS does.
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EINTR)) {
            return Err(error);
        }
    }
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_keeps_absent_fields_absent() {
        let gecos = c"Full Name";
        let entry = passwd(16, |entry, _, _, found| {
            unsafe {
                (*entry).pw_name = std::ptr::null_mut();
                (*entry).pw_gecos = gecos.as_ptr().cast_mut();
                (*entry).pw_dir = std::ptr::null_mut();
                *found = entry;
            }
            0
        }).ok().unwrap();
        assert_eq!((entry.name, entry.gecos, entry.dir), (None, b"Full Name".to_vec(), None));
    }
}
