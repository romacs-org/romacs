//! Landlock confinement of a world build (DESIGN-M1 §5 step 2): the build
//! process may read and execute only beneath its recipe's roots and write
//! only the files it produces. It is a backstop to the host boundary, not
//! the mechanism: a read outside the roots fails with EACCES, which a traced
//! read lists as a refusal (`host::note_confined`).
//!
//! Landlock confines the calling thread and the threads and processes it
//! starts afterwards; stat and access checks stay free.
use std::io;
use std::path::{Path, PathBuf};

const CREATE_RULESET_VERSION: libc::c_uint = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;
const EXECUTE: u64 = 1 << 0;
const WRITE_FILE: u64 = 1 << 1;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;
const TRUNCATE: u64 = 1 << 14;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

/// The filesystem rights Landlock ABI version ABI handles: each version adds
/// one (REFER at 2, TRUNCATE at 3, IOCTL_DEV at 5).
fn handled(abi: i64) -> u64 {
    let rights = match abi { 1 => 13, 2 => 14, 3 | 4 => 15, _ => 16 };
    (1 << rights) - 1
}

/// Confine this thread: read and execute beneath each of READ, write each
/// file of WRITE, nothing else. A root that does not exist is skipped.
pub fn confine(read: &[PathBuf], write: &[PathBuf]) -> io::Result<()> {
    let abi = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<RulesetAttr>(), 0, CREATE_RULESET_VERSION) };
    if abi < 1 {
        return Err(io::Error::last_os_error());
    }
    let handled = handled(abi);
    let attr = RulesetAttr { handled_access_fs: handled };
    let ruleset = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr, size_of::<RulesetAttr>(), 0) };
    if ruleset < 0 {
        return Err(io::Error::last_os_error());
    }
    let ruleset = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(ruleset as i32) };
    for path in read {
        allow(&ruleset, path, EXECUTE | READ_FILE | READ_DIR)?;
    }
    for path in write {
        allow(&ruleset, path, WRITE_FILE | (handled & TRUNCATE))?;
    }
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    use std::os::fd::AsRawFd;
    if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    crate::host::note_confined();
    Ok(())
}

/// Allow ACCESS beneath PATH, a file's own rights only on a file.
fn allow(ruleset: &std::os::fd::OwnedFd, path: &Path, access: u64) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return Ok(()) };
    let fd = unsafe { libc::open(name.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Ok(());
    }
    let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let directory = crate::host::metadata(path).is_ok_and(|m| m.file_type().is_dir());
    let access = if directory { access } else { access & !READ_DIR };
    let rule = PathBeneath { allowed_access: access, parent_fd: fd.as_raw_fd() };
    if unsafe { libc::syscall(libc::SYS_landlock_add_rule, ruleset.as_raw_fd(), RULE_PATH_BENEATH, &rule, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The roots this binary itself reads at run time: its build tree (etc/,
/// the source directory) and `ROMACS_RUNTIME_ROOT`, the directories of the
/// shared objects it maps (glibc loads NSS modules from its own), the
/// process's /proc entries, the time zones and the user and host databases.
pub fn runtime_roots() -> Vec<PathBuf> {
    let mut roots = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")];
    roots.extend(crate::host::getenv("ROMACS_RUNTIME_ROOT").map(PathBuf::from));
    let maps = crate::host::read_to_string("/proc/self/maps").unwrap_or_default();
    for path in maps.lines().filter_map(|line| line.split_whitespace().nth(5)).filter(|p| p.starts_with('/')) {
        if let Some(parent) = Path::new(path).parent().filter(|p| !roots.iter().any(|r| r == p)) {
            roots.push(parent.to_owned());
        }
    }
    roots.extend([
        "/proc/self", "/proc/sys/kernel/hostname", "/usr/share/zoneinfo", "/etc/localtime",
        "/etc/passwd", "/etc/group", "/etc/hostname", "/etc/nsswitch.conf",
    ].map(PathBuf::from));
    roots
}
