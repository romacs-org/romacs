//! Replacing the keeper by exec in place.
//!
//! Same pid, same children, same unit: the process image changes and
//! nothing else. The table goes through a memfd ([`Table::export`]); the
//! descriptors it names lose close-on-exec for the exec and get it back in
//! the new image, and no other descriptor is inherited.
//!
//! The exec is guarded twice. Before it, the new binary reads a copy of
//! the table in a check mode, as a child: a binary that cannot resume is
//! refused and nothing changes. After it, a new image that still fails to
//! resume execs back into the old executable, whose descriptor it was
//! passed, with the same table.
//!
//! Three calls stay direct C library calls, as no crate in the lock file
//! makes them safely: the exec itself (rustix's `execve` is an unsafe
//! runtime function, and std's `CommandExt::exec` resets the signal mask
//! and SIGPIPE, which an exec in place must carry over unchanged); the
//! fallback's `execveat`, likewise, given the C library's `environ`, which
//! std does not expose; and `close_range` over descriptors nothing owns,
//! which no safe API can express. The resumed image adopts what it
//! inherited by number ([`resume`]): only `from_raw_fd` can own a
//! descriptor this image did not open.

use std::io::{Read, Seek, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::server::{clear_cloexec, restore_cloexec, Build};
use crate::table::{Table, TABLE_VERSION};

fn memfd(bytes: &[u8]) -> std::io::Result<std::fs::File> {
    let mut file = std::fs::File::from(rustix::fs::memfd_create(c"romacs-keeper-table", rustix::fs::MemfdFlags::CLOEXEC)?);
    file.write_all(bytes)?;
    file.rewind()?;
    Ok(file)
}

fn export(table: &Table, build: Build) -> (Vec<u8>, Vec<BorrowedFd<'_>>) { table.export(build.table > TABLE_VERSION) }

/// Ask the binary at PATH whether it can resume from TABLE.
///
/// # Errors
/// Why it cannot: it does not start, takes too long, or refuses the table.
pub fn check(table: &Table, build: Build, path: &Path) -> Result<(), String> {
    let name = path.display();
    let file = memfd(&export(table, build).0).map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(path).arg("--check-table").stdin(file)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("{name}: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut why = String::new();
                let _ = child.stderr.take().map(|mut e| e.read_to_string(&mut why));
                return Err(format!("{name} refuses this keeper's table ({status}): {}", why.trim()));
            }
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(2)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} did not answer the table check"));
            }
        }
    }
}

// SAFETY: libc's own variable, declared with its C type.
unsafe extern "C" {
    static environ: *const *const libc::c_char;
}

fn c_string(bytes: &[u8]) -> std::ffi::CString { std::ffi::CString::new(bytes).unwrap_or_default() }

/// Exec PATH over this process, carrying TABLE. Returns only if the exec
/// failed, with every descriptor close-on-exec again.
pub fn exec(table: &Table, build: Build, path: &Path) {
    let (bytes, fds) = export(table, build);
    let (Ok(file), Ok(old)) = (memfd(&bytes), std::fs::File::open("/proc/self/exe")) else { return };
    let (carried, back) = (file.as_raw_fd(), old.as_raw_fd());
    let kept: Vec<BorrowedFd<'_>> = fds.into_iter().chain([file.as_fd(), old.as_fd()]).collect();
    clear_cloexec(&kept);
    let words = [c_string(path.as_os_str().as_bytes()), c_string(b"--resume"), c_string(carried.to_string().as_bytes()),
                c_string(b"--fallback"), c_string(back.to_string().as_bytes())];
    let argv: Vec<_> = words.iter().map(|a| a.as_ptr()).chain([std::ptr::null()]).collect();
    // SAFETY: argv is a null-terminated array of NUL-terminated strings
    // alive across the call; the environment is this process's own.
    unsafe { libc::execv(argv[0], argv.as_ptr()) };
    restore_cloexec(&kept);
}

/// Close every descriptor above the standard streams that is not in KEPT:
/// a resumed keeper holds what its table names and nothing else, whatever
/// the image before it left open.
fn close_all_but(mut kept: Vec<RawFd>) {
    kept.sort_unstable();
    let mut low = 3;
    for fd in kept.into_iter().filter(|fd| *fd >= 3).chain([RawFd::MAX]) {
        if fd > low {
            // SAFETY: closing descriptors this process holds and nothing
            // owns: the table's are excluded, and a keeper that has just
            // resumed has made no other.
            unsafe { libc::close_range(low.cast_unsigned(), (fd - 1).cast_unsigned(), 0) };
        }
        low = fd.saturating_add(1);
    }
}

/// In a freshly exec'd keeper: take over the table in descriptor `TABLE_FD`.
/// A table this build cannot read sends the process back into the old
/// executable (FALLBACK), which resumes from the same table.
///
/// # Errors
/// The table's fault, when there is no way back.
///
/// # Safety
/// The process must have been exec'd by [`exec`]: `TABLE_FD`, FALLBACK and
/// every descriptor the table names are then inherited, open, and owned by
/// nothing else.
pub unsafe fn resume(table_fd: RawFd, fallback: Option<RawFd>, build: Build) -> Result<Table, String> {
    // SAFETY: the caller's contract: an inherited descriptor owned here.
    let mut file = unsafe { std::fs::File::from_raw_fd(table_fd) };
    let mut bytes = Vec::new();
    let read = file.rewind().and_then(|()| file.read_to_end(&mut bytes)).map_err(|e| e.to_string());
    // SAFETY: the caller's contract covers the descriptors the table names.
    match read.and_then(|_| unsafe { Table::import(&bytes, build.table) }) {
        Ok(table) => {
            let kept: Vec<RawFd> = {
                let fds = table.export(false).1;
                restore_cloexec(&fds);
                fds.iter().map(AsRawFd::as_raw_fd).collect()
            };
            drop(file);
            close_all_but(kept);
            Ok(table)
        }
        Err(error) => {
            let Some(old) = fallback else { return Err(error) };
            let carried = file.into_raw_fd().to_string();
            let words = [c_string(b"romacs-keeper"), c_string(b"--resume"), c_string(carried.as_bytes())];
            let argv: Vec<_> = words.iter().map(|a| a.as_ptr()).chain([std::ptr::null()]).collect();
            // SAFETY: an empty path with AT_EMPTY_PATH execs the file OLD
            // names; argv is null-terminated and alive across the call, and
            // `environ` is read by this process's only thread.
            unsafe { libc::syscall(libc::SYS_execveat, old, c"".as_ptr(), argv.as_ptr(), environ, libc::AT_EMPTY_PATH) };
            Err(format!("{error}; and the old keeper could not be resumed: {}", std::io::Error::last_os_error()))
        }
    }
}
