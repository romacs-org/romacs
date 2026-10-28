//! The host boundary: the VM's and its embedders' host queries and file
//! effects go through this module, so a world builder can later record and
//! classify each one. `tests/host_boundary_lint.rs` keeps other sources from
//! calling the host APIs it names directly.
//!
//! Only `Mode::Off` exists, in which each function is a direct passthrough:
//! it returns exactly what the host call returns.
use std::cell::Cell;
use std::ffi::{CStr, OsString};
use std::fs::{File, Metadata, ReadDir};
use std::io;
use std::path::{Path, PathBuf};
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

/// fstat on a file this module opened.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    file.metadata()
}

pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::read_link(path)
}

/// The entries of PATH, as the host yields them.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    std::fs::read_dir(path)
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// PATH made absolute against the working directory, lexically.
pub fn absolute(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    std::path::absolute(path)
}

/// faccessat with the effective ids: whether MODE access to PATH is allowed.
pub fn access(path: &CStr, mode: i32) -> io::Result<()> {
    if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) } == 0 { Ok(()) }
    else { Err(io::Error::last_os_error()) }
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
    use std::io::Read;
    let mut bytes = [0u8; 4];
    File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes))?;
    Ok(bytes)
}

pub fn open(path: impl AsRef<Path>, options: &OpenOptions) -> io::Result<File> {
    options.open(path)
}

pub fn open_read(path: impl AsRef<Path>) -> io::Result<File> {
    File::open(path)
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
