//! The keeper's command line, and how a session finds and starts one.
//!
//! `romacs-keeper --socket PATH [--foreground]` starts a keeper listening
//! at PATH. Without `--foreground` it detaches first: its own session, no
//! controlling terminal, directory `/`, an empty environment, and the
//! starting process returns once the socket listens. Under systemd the
//! session starts it `--foreground` in a transient user unit of its own, so
//! the end of the session's unit does not take it along.
//!
//! One keeper serves one socket: a lock beside the socket is held for the
//! keeper's life, and a second keeper for the same path exits at once.

use std::io::{Read, Write};
use std::os::fd::{OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::server::{fresh, Build, Keeper};
use crate::table::Table;

/// Where the keeper of session NAME listens, under the runtime directory.
///
/// # Errors
/// A path too long for a socket address, named.
pub fn socket_path(runtime: &Path, name: &str) -> Result<PathBuf, String> {
    let path = runtime.join("romacs").join(name).join("keeper.sock");
    if path.as_os_str().len() > 107 {
        return Err(format!("keeper socket path {} is over 107 bytes", path.display()));
    }
    Ok(path)
}

/// Take the lock that makes this the one keeper of SOCKET; `None` when
/// another holds it. The table keeps the descriptor, across an exec too.
fn lock(socket: &Path) -> std::io::Result<Option<OwnedFd>> {
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(socket.with_extension("lock"))?;
    // An exclusive flock, without waiting.
    Ok(file.try_lock().is_ok().then(|| file.into()))
}

/// Leave the starting process: a new session, no terminal, directory `/`,
/// no environment, no standard streams. Returns in the detached child with
/// the pipe on which it reports that it listens; the parent waits for that
/// report and exits.
///
/// `fork`, `clearenv` and `_exit` are direct calls: rustix has them only
/// as unsafe runtime functions; std's `remove_var` is unsafe too, and
/// std's `exit` runs the C library's exit handlers in a parent that
/// shares them with the child.
fn detach() -> std::io::Result<std::fs::File> {
    let (mut ready, report) = std::io::pipe()?;
    // SAFETY: fork at startup, before any thread exists; the child goes on
    // as the only thread of an ordinary process.
    match unsafe { libc::fork() } {
        -1 => Err(std::io::Error::last_os_error()),
        0 => {
            drop(ready);
            let null = std::fs::OpenOptions::new().read(true).write(true).open("/dev/null")?;
            let _ = rustix::process::setsid();
            let _ = rustix::stdio::dup2_stdin(&null);
            let _ = rustix::stdio::dup2_stdout(&null);
            let _ = rustix::stdio::dup2_stderr(&null);
            // SAFETY: clearenv in a single-threaded process: nothing else
            // reads the environment while it is emptied.
            unsafe { libc::clearenv() };
            std::env::set_current_dir("/")?;
            Ok(std::fs::File::from(OwnedFd::from(report)))
        }
        _ => {
            drop(report);
            let mut answer = String::new();
            let _ = ready.read_to_string(&mut answer);
            // SAFETY: _exit takes a status; the parent has nothing to flush.
            unsafe { libc::_exit(i32::from(answer != "ready")) }
        }
    }
}

fn serve(table: Table, build: Build) -> Result<(), String> {
    let path = PathBuf::from(String::from_utf8_lossy(&table.socket_path).into_owned());
    let mut keeper = Keeper::new(table, build).map_err(|e| e.to_string())?;
    keeper.run();
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn start(socket: &Path, foreground: bool, idle_ms: Option<u64>, build: Build) -> Result<(), String> {
    let mut report = if foreground { None } else { Some(detach().map_err(|e| e.to_string())?) };
    let mut ready = || { if let Some(mut report) = report.take() { let _ = report.write_all(b"ready"); } };
    if let Some(parent) = socket.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let Some(lock) = lock(socket).map_err(|e| e.to_string())? else {
        // Another keeper serves this socket, or is about to.
        ready();
        return Ok(());
    };
    let mut table = fresh(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    table.lock = Some(lock);
    if let Some(idle_ms) = idle_ms { table.idle_ms = idle_ms; }
    ready();
    serve(table, build)
}

/// Run the keeper as its arguments say.
#[must_use]
pub fn main(build: Build) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let fd = |name: &str| value(name).and_then(|v| v.parse::<RawFd>().ok());
    let result = if args.first().is_some_and(|a| a == "--check-table") {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map_err(|e| e.to_string()).and_then(|_| Table::check(&bytes, build.table))
    } else if let Some(table) = fd("--resume") {
        // SAFETY: `--resume` is passed only by `exec::exec`, which leaves
        // exactly the table's descriptors, the table and the fallback open.
        // They are inherited by number, which only an unsafe adoption can
        // own (`exec::resume`).
        unsafe { crate::exec::resume(table, fd("--fallback"), build) }.and_then(|table| serve(table, build))
    } else if let Some(socket) = value("--socket") {
        start(Path::new(&socket), args.iter().any(|a| a == "--foreground"), value("--idle-ms").and_then(|v| v.parse().ok()), build)
    } else {
        Err("usage: romacs-keeper --socket PATH [--foreground] [--idle-ms N]".into())
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("romacs-keeper: {error}");
            ExitCode::FAILURE
        }
    }
}
