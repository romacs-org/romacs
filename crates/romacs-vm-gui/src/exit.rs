//! Why a GUI session's process ends. Every end says so in one stderr line
//! before the process exits, `romacs-exit: {"reason": R, "detail": D,
//! "status": N}`, which a launcher keeping stderr records (romacs-sessions
//! shows it). Once a session starts ([`install`]): a Rust panic on any
//! thread is said on stderr and written to a crash report in
//! $XDG_STATE_HOME/romacs/crashes, and an ERROR-level tracing event (winit's
//! lost Wayland connection, wgpu's lost device) is said on stderr as
//! `romacs-error: {"target": T, "message": M}`, the last one kept for the
//! detail of an end the display caused.
use romacs_display_runtime::render_thread::{RenderLoopError, RenderLoopExit, RenderShutdownReason};
use serde_json::json;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, PartialEq)]
pub struct Exit {
    pub reason: &'static str,
    pub detail: String,
    pub status: i32,
}

impl Exit {
    pub fn new(reason: &'static str, detail: impl Into<String>, status: i32) -> Self {
        Self { reason, detail: detail.into(), status }
    }

    pub fn line(&self) -> String {
        format!("romacs-exit: {}", json!({"reason": self.reason, "detail": self.detail, "status": self.status}))
    }
}

/// Say END's line and end the process with its status.
pub fn finish(end: Exit) -> ! {
    eprintln!("{}", end.line());
    std::process::exit(end.status)
}

/// What the evaluator thread made of the session, as the main thread finds it.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Ended(Exit),
    /// Its thread unwound without one: it panicked.
    Panicked,
    /// It has not ended.
    Running,
}

/// The session's end, from how the render loop on the main thread ended
/// (HOST) and the evaluator's VERDICT. The display's own failure is the
/// cause whatever the evaluator then did with the close it was sent; an
/// end the evaluator asked for is its own.
pub fn session(host: Result<RenderLoopExit, RenderLoopError>, verdict: Verdict) -> Exit {
    let evaluator = |otherwise: Exit| match verdict {
        Verdict::Ended(end) => end,
        Verdict::Panicked => panicked("evaluator"),
        Verdict::Running => otherwise,
    };
    match host {
        Err(RenderLoopError::EventLoop(error)) => Exit::new("display-lost", with_last_error(error), 1),
        Err(RenderLoopError::StartupInterrupted(error)) => Exit::new("display-startup-failed", with_last_error(error), 1),
        Err(RenderLoopError::Preparation(error)) => evaluator(Exit::new("display-preparation-failed", error, 1)),
        Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::NativeWindowDestroyed))) =>
            Exit::new("window-destroyed", with_last_error("the native window was destroyed".into()), 1),
        Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::StartupCancelled)) | RenderLoopExit::GpuStartupCancelled) =>
            evaluator(Exit::new("window-closed-during-startup", "", 0)),
        Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::EvaluatorShutdown) | None)) =>
            evaluator(Exit::new("render-loop-ended", with_last_error("the render loop ended under a running evaluator".into()), 1)),
    }
}

/// Whether HOST's end comes from the evaluator, whose verdict is then worth
/// waiting for: it sends it before it lets the loop end.
pub fn evaluator_ended(host: &Result<RenderLoopExit, RenderLoopError>) -> bool {
    matches!(host, Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::EvaluatorShutdown) | None)))
        || matches!(host, Err(RenderLoopError::Preparation(_)))
}

static INSTALLED: AtomicBool = AtomicBool::new(false);
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// ERROR events said on stderr; later ones are kept, not said.
static ERRORS_SAID: AtomicUsize = AtomicUsize::new(0);
const ERRORS_TO_SAY: usize = 200;

/// Whether a session has started, so that its error ends say why.
pub fn installed() -> bool { INSTALLED.load(Ordering::Relaxed) }

/// The session's panic report and ERROR events, from here on.
pub fn install() {
    if INSTALLED.swap(true, Ordering::Relaxed) { return; }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let site = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let payload = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        let report = write_report(&thread, &site, &std::backtrace::Backtrace::force_capture().to_string());
        eprintln!("romacs-panic: {}", json!({"thread": thread, "site": site,
            "report": report.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|e| format!("not written: {e}"))}));
        *LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(format!("thread {thread} panicked at {site}: {}", truncate(&payload, 300)));
    }));
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;
    let _ = tracing_subscriber::registry()
        .with(ErrorLines.with_filter(tracing_subscriber::filter::LevelFilter::ERROR))
        .try_init();
}

/// The end a panic on WHOSE thread made, from the last panic said.
pub fn panicked(whose: &str) -> Exit {
    let panic = LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Exit::new("panic", format!("{whose}: {}", panic.as_deref().unwrap_or("no panic report")), 101)
}

fn with_last_error(detail: String) -> String {
    match LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).as_deref() {
        Some(error) => format!("{detail}; last error: {error}"),
        None => detail,
    }
}

fn truncate(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The crash report romacs-sessions lists: $XDG_STATE_HOME/romacs/crashes/
/// panic-UNIXNS-PID.txt. The payload stays on stderr only.
fn write_report(thread: &str, site: &str, backtrace: &str) -> std::io::Result<std::path::PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".local/state")))
        .ok_or_else(|| std::io::Error::other("no XDG_STATE_HOME or HOME"))?;
    let dir = state.join("romacs/crashes");
    std::fs::create_dir_all(&dir)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("panic-{now}-{}.txt", std::process::id()));
    std::fs::write(&path, report_text(thread, site, backtrace, now))?;
    Ok(path)
}

fn report_text(thread: &str, site: &str, backtrace: &str, now: u128) -> String {
    format!("Romacs native panic report v1\npid: {}\nlogging-target: romacs-vm-gui\nversion: {}\nexecutable: {:?}\n\n\
             === PANIC ===\nunix-ns: {now}\nthread: {thread}\nsite: {site}\npayload: [on stderr only]\nbacktrace:\n{backtrace}\n",
            std::process::id(), env!("CARGO_PKG_VERSION"), std::env::current_exe().ok())
}

/// ERROR events on stderr, the last kept (`with_last_error').
struct ErrorLines;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ErrorLines {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut message = Fields(String::new());
        event.record(&mut message);
        let target = event.metadata().target();
        if ERRORS_SAID.fetch_add(1, Ordering::Relaxed) < ERRORS_TO_SAY {
            eprintln!("romacs-error: {}", json!({"target": target, "message": message.0}));
        }
        *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{target}: {}", truncate(&message.0, 500)));
    }
}

/// An event's fields as text: its message, then NAME=VALUE for the rest.
struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let gap = if self.0.is_empty() { "" } else { " " };
        let _ = if field.name() == "message" { write!(self.0, "{gap}{value:?}") } else { write!(self.0, "{gap}{}={value:?}", field.name()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(reason: &'static str, status: i32) -> Verdict { Verdict::Ended(Exit::new(reason, "", status)) }

    #[test]
    fn the_line_is_one_json_object() {
        let line = Exit::new("kill-emacs", "a \"quoted\"\nline", 3).line();
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(line.strip_prefix("romacs-exit: ").unwrap()).unwrap();
        assert_eq!(value, json!({"reason": "kill-emacs", "detail": "a \"quoted\"\nline", "status": 3}));
    }

    /// A lost display is the cause, even when the evaluator has since
    /// handled the close it was sent and ended cleanly.
    #[test]
    fn a_lost_display_wins_over_the_evaluators_end() {
        let end = session(Err(RenderLoopError::EventLoop("Exit Failure: 71".into())), ended("window-close", 0));
        assert_eq!((end.reason, end.status), ("display-lost", 1));
        assert!(end.detail.starts_with("Exit Failure: 71"), "{}", end.detail);
        let end = session(Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::NativeWindowDestroyed))), ended("kill-emacs", 0));
        assert_eq!((end.reason, end.status), ("window-destroyed", 1));
    }

    #[test]
    fn an_evaluator_end_is_its_own() {
        let host = || Ok(RenderLoopExit::Finished(Some(RenderShutdownReason::EvaluatorShutdown)));
        assert_eq!(session(host(), ended("kill-emacs", 3)), Exit::new("kill-emacs", "", 3));
        assert_eq!(session(host(), Verdict::Panicked).reason, "panic");
        assert_eq!(session(host(), Verdict::Panicked).status, 101);
        let silent = session(host(), Verdict::Running);
        assert_eq!((silent.reason, silent.status), ("render-loop-ended", 1), "never a silent 0");
        assert_eq!(session(Ok(RenderLoopExit::Finished(None)), Verdict::Running).status, 1);
        let failed = session(Err(RenderLoopError::Preparation("no frame".into())), ended("evaluator-error", 1));
        assert_eq!(failed.reason, "evaluator-error");
        assert_eq!(session(Err(RenderLoopError::Preparation("no frame".into())), Verdict::Running).reason,
                   "display-preparation-failed");
    }

    #[test]
    fn a_close_during_startup_is_a_clean_end() {
        assert_eq!(session(Ok(RenderLoopExit::GpuStartupCancelled), Verdict::Running),
                   Exit::new("window-closed-during-startup", "", 0));
    }

    /// In a child process of this test binary, whose hook is the session's:
    /// a helper thread's panic is said on stderr and written to a report.
    #[test]
    fn a_panic_on_any_thread_reaches_stderr_and_a_report() {
        const CHILD: &str = "ROMACS_EXIT_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            install();
            let _ = std::thread::Builder::new().name("helper".into()).spawn(|| panic!("helper failed")).unwrap().join();
            return;
        }
        let state = std::env::temp_dir().join(format!("romacs-exit-test-{}", std::process::id()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "exit::tests::a_panic_on_any_thread_reaches_stderr_and_a_report", "--nocapture"])
            .env(CHILD, "1").env("XDG_STATE_HOME", &state).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reports: Vec<_> = std::fs::read_dir(state.join("romacs/crashes")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        let report = reports.first().map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&state);
        assert!(stderr.contains("thread 'helper'") && stderr.contains("helper failed"), "{stderr}");
        assert!(stderr.contains("romacs-panic: {\"report\":"), "{stderr}");
        assert_eq!(reports.len(), 1, "{stderr}");
        assert!(report.contains("thread: helper\n") && !report.contains("helper failed"), "{report}");
    }

    #[test]
    fn the_report_names_its_thread_and_site_but_not_the_payload() {
        let text = report_text("romacs-evaluator", "src/main.rs:1", "0: frame", 7);
        assert!(text.starts_with("Romacs native panic report v1\n"));
        assert!(text.contains("thread: romacs-evaluator\nsite: src/main.rs:1\n"));
        assert!(text.contains("payload: [on stderr only]"));
    }
}
