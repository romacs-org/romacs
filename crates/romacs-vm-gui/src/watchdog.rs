//! The main VM's one-second rule, watched.  The evaluator says when it
//! leaves its wait for input and when it goes back to it
//! (`terminal_wait::wait` calls [`working`] and [`waiting`]): whatever it
//! runs in between (a command, a timer, process output, a server's
//! request, startup itself) is one span of work.  A thread of its own looks
//! ten times a second; a span past [`threshold`] is a stall.  While it lasts
//! the watchdog asks the evaluator for its call stack once a second, through
//! the debugger's inspection (`romacs_vm::host::inspect_*`: answered at the
//! next safe point, read with no Lisp run).  When it ends, the stall is one
//! JSON line: on stderr at once (a server's journal), and kept for the usage
//! log, which takes it with its counts (`romacs--take-stalls`).  A stall
//! still going at [`ONGOING`] is said once on stderr then, so a wedged
//! session leaves a trace.
//!
//!   {"kind":"stall","ms":MS,"during":"startup"|"session","buffer":B,
//!    "samples":[[FUNCTION...],...]}
//!
//! MS is how long the span ran; each sample the stack's functions,
//! innermost first, a second apart from the first second.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How often the watchdog looks.
const LOOK: Duration = Duration::from_millis(100);
/// How often a stall's stack is asked for.
const SAMPLE: Duration = Duration::from_secs(1);
/// The samples a stall keeps.
const SAMPLES: usize = 8;
/// The functions a sample keeps, innermost first.
const FRAMES: usize = 24;
/// A stall this long is said on stderr while it goes on.
const ONGOING: Duration = Duration::from_secs(10);

/// When the evaluator's span of work began, in nanoseconds past [`epoch`]
/// plus one; 0 while it waits for input.
static SINCE: AtomicU64 = AtomicU64::new(0);

fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now() -> u64 { u64::try_from(epoch().elapsed().as_nanos()).unwrap_or(u64::MAX - 1) + 1 }

/// The evaluator left its wait: its span of work begins.
pub fn working() { SINCE.store(now(), Ordering::Relaxed); }

/// The evaluator waits for input: its span of work ended.
pub fn waiting() { SINCE.store(0, Ordering::Relaxed); }

/// The span past which work is a stall: `ROMACS_STALL_MS` milliseconds, by
/// default one second.
pub fn threshold() -> Duration {
    Duration::from_millis(std::env::var("ROMACS_STALL_MS").ok().and_then(|ms| ms.parse().ok()).unwrap_or(1000))
}

/// A stall as it is watched.
#[derive(Debug, Default)]
struct Stall {
    /// Its span's start ([`SINCE`]).
    since: u64,
    /// The stacks taken so far.
    samples: Vec<Vec<String>>,
    buffer: Option<String>,
    /// The inspection asked for and not yet answered, and when the last was.
    asked: Option<u64>,
    last: Option<Instant>,
    said_ongoing: bool,
}

/// The functions of an inspection's JSON SNAPSHOT, innermost first, and its
/// buffer's name.
fn stack(snapshot: &serde_json::Value) -> (Vec<String>, Option<String>) {
    let frames = snapshot.get("frames").and_then(serde_json::Value::as_array).map(|frames| {
        frames.iter().take(FRAMES).map(|frame| match frame.get("fn") {
            Some(serde_json::Value::String(name)) => name.clone(),
            Some(other) => other.get("type").and_then(serde_json::Value::as_str).map_or_else(|| "?".to_owned(), |kind| format!("<{kind}>")),
            None => "?".to_owned(),
        }).collect()
    }).unwrap_or_default();
    let buffer = snapshot.get("buffer").and_then(|buffer| buffer.get("str").or(Some(buffer))).and_then(serde_json::Value::as_str).map(str::to_owned);
    (frames, buffer)
}

/// STALL's record, MS long; ONGOING while it still runs.
fn record(stall: &Stall, ms: u128, startup: bool, ongoing: bool) -> String {
    let mut line = serde_json::json!({
        "kind": "stall", "ms": ms, "during": if startup { "startup" } else { "session" },
        "buffer": stall.buffer, "samples": stall.samples,
    });
    if ongoing { line["ongoing"] = serde_json::Value::Bool(true); }
    line.to_string()
}

/// Watch the evaluator, whose thread this is called on: it answers the
/// watchdog's inspections, and its span of work begins now (startup).
/// Stalls past THRESHOLD are recorded.
pub fn start(threshold: Duration) {
    romacs_vm::host::inspect_target_unless_named();
    working();
    // The first span is startup's, until the evaluator first waits.
    let startup = SINCE.load(Ordering::Relaxed);
    let started = std::thread::Builder::new().name("romacs-watchdog".into()).spawn(move || {
        let mut stall: Option<Stall> = None;
        loop {
            std::thread::sleep(LOOK);
            let since = SINCE.load(Ordering::Relaxed);
            // The span the stall was in ended.
            if let Some(ended) = stall.take_if(|s| s.since != since) {
                let ms = Duration::from_nanos(now().saturating_sub(ended.since)).as_millis();
                let line = record(&ended, ms, ended.since == startup, false);
                eprintln!("romacs-stall: {line}");
                romacs_vm::host::note_stall(line);
            }
            if since == 0 { continue; }
            let ran = Duration::from_nanos(now().saturating_sub(since));
            if ran < threshold { continue; }
            let watched = stall.get_or_insert_with(|| Stall { since, ..Stall::default() });
            if let Some(generation) = watched.asked
                && let Some(answer) = romacs_vm::host::inspect_answer_since(generation)
                && let Ok(answer) = serde_json::from_str::<serde_json::Value>(&answer)
            {
                let (frames, buffer) = stack(&answer);
                watched.samples.push(frames);
                watched.buffer = watched.buffer.take().or(buffer);
                watched.asked = None;
            }
            if watched.asked.is_none() && watched.samples.len() < SAMPLES && watched.last.is_none_or(|last| last.elapsed() >= SAMPLE) {
                watched.asked = Some(romacs_vm::host::inspect_raise());
                watched.last = Some(Instant::now());
            }
            if ran >= ONGOING && !watched.said_ongoing {
                watched.said_ongoing = true;
                eprintln!("romacs-stall: {}", record(watched, ran.as_millis(), since == startup, true));
            }
        }
    });
    if let Err(error) = started { eprintln!("romacs: stall watchdog: {error}"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_names_its_stack_and_buffer() {
        let snapshot = serde_json::json!({"frames": [{"fn": "romacs-agent--read-journal", "argc": 2},
                                                     {"fn": {"type": "function"}}, {"fn": "romacs-server-start"}],
                                          "buffer": {"str": " *temp*", "len": 7}});
        let (frames, buffer) = stack(&snapshot);
        assert_eq!(frames, ["romacs-agent--read-journal", "<function>", "romacs-server-start"]);
        assert_eq!(buffer.as_deref(), Some(" *temp*"));
    }

    #[test]
    fn a_record_says_how_long_and_where() {
        let stall = Stall { samples: vec![vec!["notmuch-hello".into()]], buffer: Some("*notmuch-hello*".into()), ..Stall::default() };
        let line: serde_json::Value = serde_json::from_str(&record(&stall, 5159, false, false)).unwrap();
        assert_eq!(line["kind"], "stall");
        assert_eq!(line["ms"], 5159);
        assert_eq!(line["during"], "session");
        assert_eq!(line["samples"][0][0], "notmuch-hello");
        assert!(line.get("ongoing").is_none());
    }
}
