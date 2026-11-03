//! Background compiles: what fills the store ahead of use in an
//! interactive session (design note §4.8; rlisp/romacs-precompile.el is
//! the switch). A host thread owns the work: the files loads read from
//! source for want of an artifact (`note_miss`), then the files compiled
//! loads used before, then every file of the installation's Lisp trees, as
//! GNU's `make` compiles its `.elc` files. It holds paths and strings
//! only, never the Lisp heap, and starts a compile only once the session
//! has been idle long enough (`note_idle`): one compile process at a time,
//! at the idle CPU and I/O priority, which it spawns, waits for and reaps
//! itself. The evaluator does no work for it and is never woken by it.
use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::time::Duration;

/// When the session's present idleness began, as nanoseconds after
/// `base()` plus one; 0 while it is not idle.
static IDLE: AtomicU64 = AtomicU64::new(0);
/// Whether the worker compiles: `start` and `stop`.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The idleness, in milliseconds, before a compile starts.
static DELAY_MS: AtomicU64 = AtomicU64::new(2000);
/// Files loaded from source for want of an artifact, oldest first, each
/// with the `load-path` of its load, and the set of them, so a file waits
/// once however often it misses.
static MISSES: Mutex<Option<(VecDeque<(PathBuf, Vec<String>)>, std::collections::HashSet<PathBuf>)>> = Mutex::new(None);

fn base() -> std::time::Instant {
    static BASE: OnceLock<std::time::Instant> = OnceLock::new();
    *BASE.get_or_init(host::monotonic)
}

/// The session became idle at SINCE, or stopped being idle (None): one
/// store, whatever the worker does.
pub(crate) fn note_idle(since: Option<std::time::Instant>) {
    let value = since.map_or(0, |since| since.saturating_duration_since(base()).as_nanos() as u64 + 1);
    IDLE.store(value, Ordering::Relaxed);
}

/// How long the session has been idle, if it is.
fn idleness() -> Option<Duration> {
    match IDLE.load(Ordering::Relaxed) {
        0 => None,
        value => Some(host::monotonic().saturating_duration_since(base() + Duration::from_nanos(value - 1))),
    }
}

/// PATH was loaded from source on LOAD_PATH for want of an artifact: the
/// worker compiles it first.
pub(super) fn note_miss(path: &Path, load_path: &[String]) {
    let mut misses = MISSES.lock().unwrap();
    let (queue, waiting) = misses.get_or_insert_default();
    if waiting.insert(path.to_owned()) {
        queue.push_back((path.to_owned(), load_path.to_vec()));
    }
}

/// Compile in the background from now on, once the session has been idle
/// DELAY, with LOAD_PATH (the session's) for the files no load asked for.
/// The thread is started once per process; a later call only enables it.
pub(crate) fn start(load_path: Vec<String>, delay: Duration) {
    DELAY_MS.store(delay.as_millis() as u64, Ordering::Relaxed);
    ENABLED.store(true, Ordering::Relaxed);
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("romacs-precompile".into()).spawn(move || run(load_path));
    });
}

/// Start no compile from now on; one running finishes.
pub(crate) fn stop() {
    ENABLED.store(false, Ordering::Relaxed);
}

/// The worker: the misses, then the candidates, each compiled unless its
/// key has an artifact already, a compile started only while enabled and
/// the session idle. With nothing left it looks again now and then: a
/// later load may miss.
fn run(load_path: Vec<String>) {
    let Some(configured) = configured() else { return };
    let mut queue: VecDeque<PathBuf> = candidates(&configured.config).into();
    loop {
        let delay = Duration::from_millis(DELAY_MS.load(Ordering::Relaxed));
        let waited = idleness().filter(|idle| ENABLED.load(Ordering::Relaxed) && *idle >= delay);
        if waited.is_none() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        // A miss is compiled whatever the store holds: the load found no
        // artifact whose inputs hold. A candidate only when its key has none.
        let missed = MISSES.lock().unwrap().as_mut().and_then(|(queue, waiting)| {
            let next = queue.pop_front()?;
            waiting.remove(&next.0);
            Some(next)
        });
        let next = missed.map(|(path, load_path)| (path, load_path, true))
            .or_else(|| queue.pop_front().map(|path| (path, load_path.clone(), false)));
        let Some((path, load_path, confirmed)) = next else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        if confirmed || !compiled(configured, &path) {
            spawn_compile(configured, &path, &load_path);
        }
    }
}

/// The files compiled loads used before, most recent first, then every
/// file of the installation's Lisp trees.
fn candidates(config: &Config) -> Vec<PathBuf> {
    if config.compiler.is_empty() {
        return vec![];
    }
    let mut files = store::history(&config.store);
    let mut seen: std::collections::HashSet<PathBuf> = files.iter().cloned().collect();
    for root in &config.runtime {
        files.extend(lisp_files(root).into_iter().filter(|p| seen.insert(p.clone())));
    }
    files
}

/// Whether PATH's key has an artifact, whatever its inputs: a load checks
/// them, and misses, if they changed.
fn compiled(configured: &Configured, path: &Path) -> bool {
    let config = &configured.config;
    let Some(identity) = configured.identity() else { return false };
    let Ok(digest) = store::source_digest(&config.store, path) else { return true };
    store::has_artifact(&config.store, &store::key(identity, path, &digest))
}

/// Compile PATH on LOAD_PATH in a compile process at the idle priority,
/// and wait for it. Its standard error goes to the generation's compile
/// log.
fn spawn_compile(configured: &Configured, path: &Path, load_path: &[String]) {
    let Some((program, args, mut environment)) = compile_command(configured, path, load_path) else { return };
    environment.push((BACKGROUND.into(), "1".into()));
    let store = &configured.config.store;
    let _ = host::create_dir_all(store::generation(store));
    let stderr = host::open_output(&store::generation(store).join("compile.log"), host::OpenOptions::new().append(true).create(true))
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());
    let Ok(mut command) = host::command(&program) else { return };
    command.args(args).env_clear().envs(environment)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(stderr);
    // One wake when it ends; the compile process keeps its own deadline
    // and dies with this session (`PARENT`).
    if let Ok(mut child) = command.spawn() {
        let _ = child.wait();
    }
}
