//! The keeper's command line, and how a session finds and starts one.
//!
//! `romacs-keeper --socket PATH [--foreground]` starts a keeper listening
//! at PATH. Without `--foreground` it detaches first: it starts itself
//! again (`--detached`) in its own session, with no controlling terminal,
//! directory `/` and an empty environment, and the starting process
//! returns once the socket listens. Under systemd the
//! session starts it `--foreground` in a transient user unit of its own, so
//! the end of the session's unit does not take it along.
//!
//! One keeper serves one socket: a lock beside the socket is held for the
//! keeper's life, and a second keeper for the same path exits at once.

use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use crate::server::{fresh, Build, Keeper};
use crate::table::Table;

/// Where the keeper of session NAME listens, under the runtime directory.
/// Any depth will do: [`crate::address`] binds and connects to it.
#[must_use]
pub fn socket_path(runtime: &Path, name: &str) -> PathBuf { runtime.join("romacs").join(name).join("keeper.sock") }

/// Take the lock that makes this the one keeper of SOCKET; `None` when
/// another holds it. The table keeps the descriptor, across an exec too.
fn lock(socket: &Path) -> std::io::Result<Option<OwnedFd>> {
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(socket.with_extension("lock"))?;
    // An exclusive flock, without waiting.
    Ok(file.try_lock().is_ok().then(|| file.into()))
}

/// Leave the starting process: start this program again with ARGS and
/// `--detached`, with no environment, directory `/` and no standard
/// streams but its output, a pipe on which it reports that it listens.
/// Returns once it has, or has died trying.
fn detach(args: &[String]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let program = std::env::args_os().next().unwrap_or_else(|| "romacs-keeper".into());
    let mut child = std::process::Command::new("/proc/self/exe").arg0(program).args(args).arg("--detached")
        .env_clear().current_dir("/").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("cannot start a detached keeper: {e}"))?;
    let mut answer = String::new();
    let _ = child.stdout.take().map(|mut report| report.read_to_string(&mut answer));
    if answer == "ready" { Ok(()) } else { Err("the detached keeper did not start".into()) }
}

/// In the process [`detach`] started: lead a new session, without a
/// terminal, and take the standard output as the report pipe, leaving
/// `/dev/null` in its place.
fn detached() -> std::io::Result<std::fs::File> {
    let _ = rustix::process::setsid();
    let report = std::io::stdout().as_fd().try_clone_to_owned()?;
    let null = std::fs::OpenOptions::new().read(true).write(true).open("/dev/null")?;
    rustix::stdio::dup2_stdout(&null)?;
    Ok(std::fs::File::from(report))
}

fn serve(table: Table, build: Build) -> Result<(), String> {
    let path = PathBuf::from(String::from_utf8_lossy(&table.socket_path).into_owned());
    let mut keeper = Keeper::new(table, build).map_err(|e| e.to_string())?;
    keeper.run();
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn start(socket: &Path, detaching: bool, idle_ms: Option<u64>, build: Build) -> Result<(), String> {
    let mut report = if detaching { Some(detached().map_err(|e| e.to_string())?) } else { None };
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

/// Run the keeper as its arguments say. The first thing a keeper binary's
/// `main` does: a resumed keeper adopts what it inherited before this
/// image opens a descriptor of its own ([`crate::exec`]).
#[must_use]
pub fn main(build: Build) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let fd = |name: &str| value(name).and_then(|v| v.parse::<RawFd>().ok());
    let result = if args.first().is_some_and(|a| a == "--check-table") {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map_err(|e| e.to_string()).and_then(|_| Table::check(&bytes, build.table))
    } else if let Some(table) = fd("--resume") {
        crate::exec::resume(table, fd("--fallback"), build).and_then(|table| serve(table, build))
    } else if let Some(socket) = value("--socket") {
        let flag = |name: &str| args.iter().any(|a| a == name);
        if flag("--foreground") || flag("--detached") {
            start(Path::new(&socket), flag("--detached"), value("--idle-ms").and_then(|v| v.parse().ok()), build)
        } else {
            detach(&args)
        }
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
