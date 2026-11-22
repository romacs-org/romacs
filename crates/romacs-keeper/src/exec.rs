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
//! Both execs are std's `CommandExt::exec`, which keeps the signal mask
//! (SIGCHLD stays blocked, and a change pending at the exec reaches the
//! new image's signalfd). It sets SIGPIPE to its default before the exec;
//! the new image ignores it again as std starts, and a keeper whose exec
//! failed writes to sockets only with `MSG_NOSIGNAL`. The fallback execs
//! the old executable through `/proc/self/fd`, as the file its path named
//! may be gone.
//!
//! One function keeps direct C library calls, as no crate in the lock
//! file makes them safely: [`inherit`], with `from_raw_fd` and
//! `close_range`: only they own or close a descriptor this image did not
//! open. Everything around them is checked first ([`unfit`]), so
//! [`resume`] is safe to call.

use std::io::{Read, Seek, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
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

/// Exec PATH over this process, carrying TABLE. Returns only if the exec
/// failed, with every descriptor close-on-exec again.
pub fn exec(table: &Table, build: Build, path: &Path) {
    let (bytes, fds) = export(table, build);
    let (Ok(file), Ok(old)) = (memfd(&bytes), std::fs::File::open("/proc/self/exe")) else { return };
    let kept: Vec<BorrowedFd<'_>> = fds.into_iter().chain([file.as_fd(), old.as_fd()]).collect();
    clear_cloexec(&kept);
    let _ = std::process::Command::new(path).arg("--resume").arg(file.as_raw_fd().to_string())
        .arg("--fallback").arg(old.as_raw_fd().to_string()).exec();
    restore_cloexec(&kept);
}

/// Why NUMBERS cannot be the descriptors a resumed keeper adopts: each
/// must be above the standard streams, named once, open, and neither
/// the table's carrier nor the fallback.
fn unfit(numbers: &[RawFd], carriers: &[RawFd]) -> Option<String> {
    let mut seen = std::collections::BTreeSet::new();
    numbers.iter().find_map(|&fd| {
        let open = std::fs::symlink_metadata(format!("/proc/self/fd/{fd}")).is_ok();
        (fd < 3 || carriers.contains(&fd) || !seen.insert(fd) || !open).then(|| format!("keeper table names descriptor {fd}, which cannot be adopted"))
    })
}

/// Own the descriptors NUMBERS name, inherited across the exec, and close
/// every other one above the standard streams: a resumed keeper holds what
/// its table names and nothing else, whatever the image before it left
/// open. Returns them by number.
///
/// Called once, by [`resume`], which [`crate::run::main`] calls before
/// this image opens a descriptor of its own, on NUMBERS [`unfit`] passed.
#[expect(unsafe_code, reason = "adopting inherited descriptors by number; see the SAFETY comment")]
fn inherit(numbers: &[RawFd]) -> std::collections::HashMap<RawFd, OwnedFd> {
    let mut sorted = numbers.to_vec();
    sorted.sort_unstable();
    let mut low = 3;
    // SAFETY: this image has opened no descriptor above the standard
    // streams (`run::main` resumes first), so every one open there was
    // inherited and nothing here owns it. NUMBERS are open, distinct and
    // above 2 (`unfit`); each becomes owned once, and `close_range` closes
    // only the gaps between them.
    // UNSAFE(only `from_raw_fd` owns an inherited descriptor; no safe API closes by number)
    unsafe {
        for &fd in sorted.iter().chain([&RawFd::MAX]) {
            if fd > low { libc::close_range(low.cast_unsigned(), (fd - 1).cast_unsigned(), 0); }
            low = fd.saturating_add(1);
        }
        numbers.iter().map(|&fd| (fd, OwnedFd::from_raw_fd(fd))).collect()
    }
}

/// In a freshly exec'd keeper: take over the table in descriptor `TABLE_FD`.
/// A table this build cannot read sends the process back into the old
/// executable (FALLBACK), which resumes from the same table.
///
/// Called by [`crate::run::main`] before this image opens a descriptor:
/// every descriptor above the standard streams is then one the image
/// before it left open.
///
/// # Errors
/// The table's fault, when there is no way back.
pub(crate) fn resume(table_fd: RawFd, fallback: Option<RawFd>, build: Build) -> Result<Table, String> {
    // Read through a fresh open, so the carrier itself stays unowned.
    let bytes = std::fs::read(format!("/proc/self/fd/{table_fd}")).map_err(|e| e.to_string());
    let carriers: Vec<RawFd> = [table_fd].into_iter().chain(fallback).collect();
    let numbers = bytes.and_then(|bytes| {
        let numbers = Table::descriptors(&bytes, build.table)?;
        unfit(&numbers, &carriers).map_or(Ok((bytes, numbers)), Err)
    });
    match numbers {
        Ok((bytes, numbers)) => {
            let mut fds = inherit(&numbers);
            let table = Table::import(&bytes, build.table, |fd| fds.remove(&fd))?;
            restore_cloexec(&table.export(false).1);
            Ok(table)
        }
        Err(error) => {
            let Some(old) = fallback else { return Err(error) };
            // The old executable, by the descriptor it was passed: the
            // file its path named may be gone or replaced.
            let back = std::process::Command::new(format!("/proc/self/fd/{old}")).arg0("romacs-keeper")
                .arg("--resume").arg(table_fd.to_string()).exec();
            Err(format!("{error}; and the old keeper could not be resumed: {back}"))
        }
    }
}
