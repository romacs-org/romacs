//! The screen vocabulary the interactive package suites share.
//!
//! Each package suite boots a GNU Emacs and a Romacs into a PTY pair and
//! drives them by screen state, so the same few primitives -- wait for a row,
//! type a command, run one step against both editors and keep both failures --
//! were copied into every suite's own `harness.rs`. They live here instead:
//! one implementation, so a fix to a wait or a panic message reaches the whole
//! family rather than one package.
//!
//! A suite's `harness.rs` keeps what is genuinely its own: readiness markers,
//! package-specific capture, and the values it waits on. A wait's tolerance
//! stays at its call site -- `invoke_with_prompt_timeout` for a prompt that
//! follows a heavy screen -- so a suite can be patient where it has reason to
//! be without forking the helper.

use crate::TuiSession;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

/// Re-read `session` until `predicate` holds, or fail with the final screen.
///
/// The timeout is explicit at each call site: how long a row may take is a
/// property of what is being waited for, not of the helper.
pub fn wait_for(
    session: &mut TuiSession,
    timeout: Duration,
    description: &str,
    predicate: impl Fn(&[String]) -> bool,
) {
    session.read_until(timeout, |grid| predicate(grid));
    let grid = session.text_grid();
    assert!(
        predicate(&grid),
        "{} timed out waiting for {description}:\n{}",
        session.name,
        grid.join("\n")
    );
}

/// Type `M-x <command> RET` into `session` and wait for the `ready` marker.
pub fn invoke(session: &mut TuiSession, command: &str, ready: &str) {
    invoke_with_prompt_timeout(session, command, ready, Duration::from_secs(8))
}

/// As [`invoke`], but with an explicit budget for the `M-x` prompt.
///
/// The prompt opens when the editor next reads keys, and an editor still
/// working through a large asynchronous screen -- Magit inserting a status
/// buffer runs several git processes and fontifies the result -- can take
/// longer than the default to get there, however quickly the screen itself
/// settles.
pub fn invoke_with_prompt_timeout(
    session: &mut TuiSession,
    command: &str,
    ready: &str,
    prompt_timeout: Duration,
) {
    session.send_keys("M-x");
    wait_for(session, prompt_timeout, "M-x prompt", |grid| {
        grid.iter().any(|row| row.contains("M-x"))
    });
    session.send(command.as_bytes());
    session.send_keys("RET");
    wait_for(session, Duration::from_secs(20), ready, |grid| {
        grid.iter().any(|row| row.contains(ready))
    });
}

/// The message text of a caught panic payload.
pub fn panic_text(payload: Box<dyn Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|value| (*value).to_owned())
        })
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Run `phase`, turning a panic into a `String` labelled with `label`.
///
/// A suite reports every failed step of a scenario rather than dying on the
/// first one, which needs the panic as a value.
pub fn catch_phase<T>(label: &str, phase: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(phase))
        .map_err(|payload| format!("{label}: {}", panic_text(payload)))
}

/// The single row holding `marker`, trimmed.
pub fn exact_row(session: &TuiSession, marker: &str) -> String {
    session
        .text_grid()
        .into_iter()
        .find(|row| row.contains(marker))
        .unwrap_or_else(|| panic!("{} did not render {marker:?}", session.name))
        .trim()
        .to_owned()
}
