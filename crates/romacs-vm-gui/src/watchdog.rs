//! The main VM's one-second rule, watched.  The evaluator says when it
//! leaves its wait for input and when it goes back to it
//! (`terminal_wait::wait` calls [`working`] and [`waiting`]): whatever it
//! runs in between (a command, a timer, process output, a server's
//! request, startup itself) is one span of work.  A span past the
//! threshold (`ROMACS_STALL_MS`, one second) is a stall: [`waiting`] keeps
//! its start and end, so none is missed however short its excess.  A thread
//! of its own looks ten times a second: while a stall lasts it asks the
//! evaluator for its call stack once a second, through the debugger's
//! inspection (`romacs_vm::host::inspect_*`: answered at the next safe
//! point, native loops included, read with no Lisp run); once it ended, its
//! record is one JSON line, on stderr at once (a server's journal) and kept
//! for the usage log, which takes it with its counts (`romacs--take-stalls`).
//! A stall still going is kept as an `ongoing` record the usage log takes
//! too (so a session that ends inside one still says it), and is said on
//! stderr at [`ONGOING`], so a wedged session leaves a trace.
//!
//!   {"kind":"stall","ms":MS,"during":"startup"|"session","buffer":B,
//!    "samples":[[FUNCTION...],...]}
//!
//! MS is how long the span ran; each sample the stack's functions,
//! innermost first, a second apart from the first second.
use std::sync::Mutex;
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
/// The threshold in nanoseconds, once the watchdog runs.
static THRESHOLD: AtomicU64 = AtomicU64::new(u64::MAX);
/// The stalls that ended, (start, end), for the watchdog to record.
static ENDED: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());
/// The stall being watched, and startup's span's start: the watchdog's
/// thread looks at them, and so does the session's last take of the
/// records (`look_now`).
static WATCHED: Mutex<(Option<Stall>, u64)> = Mutex::new((None, 0));

fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now() -> u64 { u64::try_from(epoch().elapsed().as_nanos()).unwrap_or(u64::MAX - 1) + 1 }

/// The evaluator left its wait: its span of work begins.
pub fn working() { SINCE.store(now(), Ordering::Relaxed); }

/// The evaluator waits for input: its span of work ended, a stall kept when
/// it ran past the threshold.
pub fn waiting() {
    let since = SINCE.swap(0, Ordering::Relaxed);
    if since == 0 { return; }
    let end = now();
    if end - since >= THRESHOLD.load(Ordering::Relaxed) {
        ENDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((since, end));
    }
}

/// The span past which work is a stall: `ROMACS_STALL_MS` milliseconds, by
/// default one second.
pub fn threshold() -> Duration {
    Duration::from_millis(std::env::var("ROMACS_STALL_MS").ok().and_then(|ms| ms.parse().ok()).unwrap_or(1000))
}

/// A stall as it is watched: its span's start, the stacks taken so far and
/// the buffer they were taken in, the inspection asked and not yet
/// answered, and when the last was asked.
#[derive(Debug, Default)]
struct Stall {
    since: u64,
    samples: Vec<Vec<String>>,
    buffer: Option<String>,
    asked: Option<u64>,
    last: Option<Instant>,
    said_ongoing: bool,
}

impl Stall {
    /// Take the answer to the inspection asked, if it came.
    fn collect(&mut self) {
        if let Some(generation) = self.asked
            && let Some(answer) = romacs_vm::host::inspect_answer_since(generation)
            && let Ok(answer) = serde_json::from_str::<serde_json::Value>(&answer)
        {
            let (frames, buffer) = stack(&answer);
            self.samples.push(frames);
            self.buffer = self.buffer.take().or(buffer);
            self.asked = None;
        }
    }
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

/// STALL's record, NS long, during startup when STARTUP; ONGOING while it
/// still runs.
fn record(stall: &Stall, ns: u64, startup: bool, ongoing: bool) -> String {
    let mut line = serde_json::json!({
        "kind": "stall", "ms": Duration::from_nanos(ns).as_millis(), "during": if startup { "startup" } else { "session" },
        "buffer": stall.buffer, "samples": stall.samples,
    });
    if ongoing { line["ongoing"] = serde_json::Value::Bool(true); }
    line.to_string()
}

/// One look: record the stalls that ended, then watch the span running.
/// STALL is the one being watched, STARTUP startup's span's start.
fn look(stall: &mut Option<Stall>, startup: u64) {
    // An answer asked during a span that has ended belongs to that span.
    if let Some(watched) = stall.as_mut() { watched.collect(); }
    let ended = std::mem::take(&mut *ENDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for (since, end) in ended {
        let done = stall.take_if(|s| s.since == since).unwrap_or(Stall { since, ..Stall::default() });
        let line = record(&done, end - since, since == startup, false);
        crate::stderr::line(&format!("romacs-stall: {line}"));
        romacs_vm::host::note_stall(line);
    }
    let since = SINCE.load(Ordering::Relaxed);
    let running = (since != 0).then(|| now().saturating_sub(since)).filter(|&ran| ran >= THRESHOLD.load(Ordering::Relaxed));
    let Some(ran) = running else {
        stall.take();
        romacs_vm::host::note_ongoing_stall(None);
        return;
    };
    if stall.as_ref().is_some_and(|s| s.since != since) { stall.take(); }
    let watched = stall.get_or_insert_with(|| Stall { since, ..Stall::default() });
    if watched.asked.is_none() && watched.samples.len() < SAMPLES && watched.last.is_none_or(|last| last.elapsed() >= SAMPLE) {
        watched.asked = Some(romacs_vm::host::inspect_raise());
        watched.last = Some(Instant::now());
    }
    let line = record(watched, ran, since == startup, true);
    if Duration::from_nanos(ran) >= ONGOING && !watched.said_ongoing {
        watched.said_ongoing = true;
        crate::stderr::line(&format!("romacs-stall: {line}"));
    }
    romacs_vm::host::note_ongoing_stall(Some(line));
}

/// One look now, from whichever thread: the stalls that ended recorded,
/// the one going kept as it stands.
fn look_now() {
    let mut watched = WATCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (stall, startup) = &mut *watched;
    look(stall, *startup);
}

/// Watch the evaluator, whose thread this is called on: it answers the
/// watchdog's inspections, and its span of work begins now (startup).
/// Stalls past THRESHOLD are recorded.
pub fn start(threshold: Duration) {
    THRESHOLD.store(u64::try_from(threshold.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    romacs_vm::host::inspect_watch();
    working();
    WATCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).1 = SINCE.load(Ordering::Relaxed);
    romacs_vm::host::on_stall_take(look_now);
    let started = std::thread::Builder::new().name("romacs-watchdog".into()).spawn(|| loop {
        std::thread::sleep(LOOK);
        look_now();
    });
    if let Err(error) = started { crate::stderr::line(&format!("romacs: stall watchdog: {error}")); }
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
        let line: serde_json::Value = serde_json::from_str(&record(&stall, 5_159_000_000, false, false)).unwrap();
        assert_eq!(line["kind"], "stall");
        assert_eq!(line["ms"], 5159);
        assert_eq!(line["during"], "session");
        assert_eq!(line["samples"][0][0], "notmuch-hello");
        assert!(line.get("ongoing").is_none());
    }

    /// A span past the threshold that ends between two looks is kept by
    /// its wait, exactly as long as it ran; a shorter one is not.
    #[test]
    fn a_wait_keeps_a_stall_however_briefly_it_overran() {
        THRESHOLD.store(1_000_000, Ordering::Relaxed);
        working();
        std::thread::sleep(Duration::from_millis(2));
        waiting();
        working();
        waiting();
        let ended = std::mem::take(&mut *ENDED.lock().unwrap());
        assert_eq!(ended.len(), 1);
        assert!(ended[0].1 - ended[0].0 >= 2_000_000);
        THRESHOLD.store(u64::MAX, Ordering::Relaxed);
    }

    /// A stall that ended just before the session does, with no look since
    /// and the last look's record of it still kept as going: the last take
    /// records it as it ended, and says nothing is going.
    #[test]
    fn the_last_take_records_a_stall_that_ended_since_the_last_look() {
        THRESHOLD.store(1_000_000, Ordering::Relaxed);
        romacs_vm::host::on_stall_take(look_now);
        working();
        std::thread::sleep(Duration::from_millis(2));
        look_now();
        assert!(romacs_vm::host::take_stalls(false).0.is_empty(), "nothing ended yet");
        waiting();
        let (lines, _) = romacs_vm::host::take_stalls(true);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(!lines[0].contains("ongoing"), "{}", lines[0]);
        THRESHOLD.store(u64::MAX, Ordering::Relaxed);
    }
}
