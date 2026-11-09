//! The flight recorder: a session's host results, logged in order, and its
//! replay in a fresh process from the same world: for bug reports, and
//! for twins that run the session again (batch sessions only, for now).
//!
//! The VM is deterministic given its host inputs: fresh objects take the
//! same table slots, hash tables and the obarray keep the same orders, and
//! collections fall at the same points. Every host input goes through
//! `host.rs`, so logging what it returns is enough to run a session again.
//!
//! - **Recording** writes one line per host result. A value no two
//!   processes share (the clocks, the process id, entropy, the machine's
//!   load) is logged whole; any other result (a file's bytes, a stat, a
//!   directory listing, an effect's outcome) as a digest.
//! - **Replaying** returns each logged value in place of the live one, and
//!   checks every other result against its digest. A session therefore
//!   replays where its files are as they were.
//! - **Steps**: each top-level form of the outermost file a session loads,
//!   and each command its command loop runs, ends with a fingerprint of the
//!   heap and the buffers (`Vm::flight_fingerprint`). A replay compares
//!   each step's fingerprint with the log's.
//! - **Files the session writes** get the kernel's clock for their times,
//!   which no replay shares: a replay that stats a file it wrote, or its
//!   directory, gives it the recorded times first (`stat`).
//!
//! The first difference, a host result or a step, is the divergence, named
//! by the step it falls in. A replay keeps running after it, live from the
//! first misaligned host call, so the session still ends.
//!
//! Not logged: the asynchronous flags a host thread raises (quit, signals,
//! inspection requests), which a batch session never sees and a GUI
//! recorder must turn into logged input events at safe points; and host
//! calls of other threads, whose results reach the evaluator through a
//! call that is logged.
//!
//! Limits, each a decision:
//! - A stat's change and access times are neither checked nor replayed: no
//!   unprivileged restore can set a change time, and the recording's own
//!   reads move access times, so checking either would make every replay
//!   diverge. Nor is the inode and device of a file the session makes,
//!   which is new in each run. Lisp sees those live (`file-attributes`);
//!   a later loggable stat would carry them.
//! - A collection's phase times (`romacs--gc-phases`) are not logged.
//! - A step looks at every live buffer's tick, O(buffers) per step (probe
//!   `flight-buffer-steps`, ticket flight-steps), and hashes only the texts
//!   that changed. A set of dirty buffers kept by the text layer would
//!   remove the scan; the GUI recorder needs it.
use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

/// Where this thread's session is: off, recording, or replaying.
enum Flight {
    Record(Recorder),
    Replay(Player),
}

struct Recorder {
    out: std::io::BufWriter<std::fs::File>,
    report: PathBuf,
    base: std::time::Instant,
    calls: u64,
    steps: u64,
}

struct Player {
    log: Vec<String>,
    at: usize,
    report: PathBuf,
    base: std::time::Instant,
    calls: u64,
    steps: u64,
    /// The last step ended, the one a divergence falls in.
    step: String,
    divergence: Option<Divergence>,
    /// Host calls since the log stopped matching: answered live.
    live: bool,
    /// The files this replay wrote (absolute), which `stat` may retime.
    written: std::collections::HashSet<PathBuf>,
}

/// The first point where a replay left its log.
#[derive(Debug)]
struct Divergence {
    kind: &'static str,
    /// The host call's or the step's index in the replay.
    index: u64,
    /// The step it follows: the command or form running when it happened.
    after: String,
    expected: String,
    found: String,
}

thread_local! {
    static FLIGHT: RefCell<Option<Flight>> = const { RefCell::new(None) };
    /// Whether FLIGHT holds a flight: the one load an unrecorded host call
    /// pays.
    static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set FLIGHT to FLIGHT_, and ACTIVE with it.
fn install(flight: Option<Flight>) -> Option<Flight> {
    ACTIVE.with(|active| active.set(flight.is_some()));
    FLIGHT.with(|f| std::mem::replace(&mut *f.borrow_mut(), flight))
}

/// Start recording to, or replaying from, the log ROMACS_FLIGHT names
/// (`record:PATH` or `replay:PATH`), on this thread, the evaluator's. The
/// variable is removed, so no process this one starts records too. A
/// report of the run is written beside the log when it ends (`finish`).
pub fn start_from_environment() -> Result<(), String> {
    let Some(spec) = crate::host::getenv("ROMACS_FLIGHT") else { return Ok(()) };
    crate::host::unset_env("ROMACS_FLIGHT");
    let spec = spec.to_string_lossy().into_owned();
    let (role, path) = spec.split_once(':').ok_or_else(|| format!("ROMACS_FLIGHT={spec}: want record:PATH or replay:PATH"))?;
    let path = PathBuf::from(path);
    let report = path.with_extension(format!("{role}.json"));
    let flight = match role {
        "record" => Flight::Record(Recorder {
            out: std::io::BufWriter::new(crate::host::open_output(&path, crate::host::OpenOptions::new().write(true).create(true).truncate(true))
                .map_err(|e| format!("flight log {}: {e}", path.display()))?),
            report, base: crate::host::flight_base(), calls: 0, steps: 0,
        }),
        "replay" => Flight::Replay(Player {
            log: crate::host::read_to_string(&path).map_err(|e| format!("flight log {}: {e}", path.display()))?
                .lines().map(str::to_owned).collect(),
            at: 0, report, base: crate::host::flight_base(), calls: 0, steps: 0, step: "start".into(), divergence: None, live: false,
            written: Default::default(),
        }),
        _ => return Err(format!("ROMACS_FLIGHT={spec}: want record:PATH or replay:PATH")),
    };
    install(Some(flight));
    Ok(())
}

/// Whether this thread records or replays.
pub fn active() -> bool {
    ACTIVE.with(std::cell::Cell::get)
}

/// F's host calls left out of the log and answered live, in recording
/// and replay alike: for the launcher's own bookkeeping (a world store's
/// locks and use stamps), which every launch changes and no session reads.
pub fn without<T>(f: impl FnOnce() -> T) -> T {
    let saved = install(None);
    let result = f();
    install(saved);
    result
}

/// The session ends: the log flushed and the report written. Called
/// before the process exits, which runs no destructor.
pub fn finish() {
    let Some(flight) = install(None) else { return };
    let (report, json) = match flight {
        Flight::Record(mut r) => {
            let _ = r.out.flush();
            (r.report, serde_json::json!({"role": "record", "host_calls": r.calls, "steps": r.steps}))
        }
        Flight::Replay(p) => {
            // A log with steps or calls this run never reached diverged too.
            let left = p.log.len() - p.at.min(p.log.len());
            let divergence = p.divergence.or_else(|| (left > 0).then(|| Divergence {
                kind: "end", index: p.steps, after: p.step.clone(),
                expected: p.log[p.at].clone(), found: "the session ended".into(),
            }));
            (p.report, serde_json::json!({"role": "replay", "host_calls": p.calls, "steps": p.steps,
                "divergence": divergence.map(|d| serde_json::json!({"kind": d.kind, "index": d.index, "after": d.after,
                    "expected": d.expected, "found": d.found}))}))
        }
    };
    let _ = crate::host::write(&report, format!("{json}\n"));
}

/// A host result no two processes share, NAME's: logged whole when
/// recording, the logged one when replaying, else LIVE's.
pub(crate) fn value<T: Logged>(name: &'static str, live: impl FnOnce() -> T) -> T {
    /// What this call does, decided in one look at the flight.
    enum Now<T> { Live, Record, Replayed(T) }
    if !active() {
        return live();
    }
    let now = FLIGHT.with(|f| match f.borrow_mut().as_mut() {
        None => Now::Live,
        Some(Flight::Record(_)) => Now::Record,
        Some(Flight::Replay(p)) => match p.take(name, String::new) {
            None => Now::Live,
            Some(payload) => match T::load(&payload, p.base) {
                Some(value) => Now::Replayed(value),
                None => {
                    p.diverge("host", p.calls, format!("{name} {payload}"), format!("{name}, a payload it cannot load"));
                    p.live = true;
                    Now::Live
                }
            },
        },
    });
    match now {
        Now::Live => live(),
        Now::Replayed(value) => value,
        Now::Record => {
            let value = live();
            FLIGHT.with(|f| if let Some(Flight::Record(r)) = f.borrow_mut().as_mut() { r.put(name, &value.save(r.base)); });
            value
        }
    }
}

/// A host result any process gets again from the same files, NAME's:
/// LIVE's, its WITNESS logged when recording and checked when replaying.
/// WHAT names its input (a path) in a divergence.
pub(crate) fn checked<T>(name: &'static str, what: impl Fn() -> String, live: impl FnOnce() -> T, witness: impl FnOnce(&T) -> u64) -> T {
    let value = live();
    if !active() {
        return value;
    }
    FLIGHT.with(|f| match f.borrow_mut().as_mut() {
        None => {}
        Some(Flight::Record(r)) => r.put(name, &format!("{:016x}", witness(&value))),
        Some(Flight::Replay(p)) => {
            if let Some(logged) = p.take(name, &what) {
                let found = format!("{:016x}", witness(&value));
                if logged != found {
                    let what = what();
                    p.diverge("host", p.calls, format!("{name} {logged}"), format!("{name} {found} {what}"));
                }
            }
        }
    });
    value
}

/// PATH, absolute, is changed by this session: a replay gives it the times
/// the recording saw when it is next stat'ed (`stat`).
pub(crate) fn wrote(path: PathBuf) {
    FLIGHT.with(|f| if let Some(Flight::Replay(p)) = f.borrow_mut().as_mut() { p.written.insert(path); });
}

/// A stat, NAME's, of the file WHAT names: logged with its mode, size and
/// modification time. A file this session writes has the kernel's clock
/// for that time, which no replay can share; so a replay that finds a file
/// it wrote (PATH, among `wrote`'s, or an OUTPUT file of its own) differing
/// from the log in that time alone gives it the logged time (RETIME) and
/// stats it again. Any other difference is a divergence.
pub(crate) fn stat(name: &'static str, what: impl Fn() -> String, path: Option<&std::path::Path>, output: bool,
                   mut live: impl FnMut() -> std::io::Result<std::fs::Metadata>,
                   retime: impl FnOnce(i64, i64) -> std::io::Result<()>) -> std::io::Result<std::fs::Metadata> {
    let value = live();
    let logged = FLIGHT.with(|f| match f.borrow_mut().as_mut() {
        None => None,
        Some(Flight::Record(r)) => { r.put(name, &stat_fields(&value)); None }
        Some(Flight::Replay(p)) => {
            let logged = p.take(name, &what)?;
            let ours = output || path.is_some_and(|path| p.written.contains(path));
            Some((logged, ours))
        }
    });
    let Some((logged, ours)) = logged else { return value };
    let found = stat_fields(&value);
    if found == logged {
        return value;
    }
    let retimed = match (ours, logged.split(' ').collect::<Vec<_>>().as_slice(), found.split(' ').collect::<Vec<_>>().as_slice()) {
        // A file the session made is new in each run: its inode differs too.
        (true, [mode, len, s, ns, nlink, uid, gid, ..], [mode2, len2, _, _, nlink2, uid2, gid2, ..])
            if (mode, len, nlink, uid, gid) == (mode2, len2, nlink2, uid2, gid2) => {
            match (s.parse(), ns.parse()) {
                (Ok(s), Ok(ns)) if retime(s, ns).is_ok() => Some(live()),
                _ => None,
            }
        }
        _ => None,
    };
    match retimed {
        Some(value) if made_alike(&stat_fields(&value), &logged) => value,
        _ => {
            let what = what();
            FLIGHT.with(|f| if let Some(Flight::Replay(p)) = f.borrow_mut().as_mut() {
                p.diverge("host", p.calls, format!("{name} {logged}"), format!("{name} {found} {what}"));
            });
            value
        }
    }
}

/// Whether FOUND and LOGGED stat fields agree but for the inode and device,
/// which a file the session made gets anew in each run.
fn made_alike(found: &str, logged: &str) -> bool {
    found.split(' ').take(7).eq(logged.split(' ').take(7))
}

/// A stat's mode, size and modification time, then its link count,
/// owners, inode and device, or its error, as a log field. A replay's
/// inputs are restored in place, so they keep their inodes. Not the change
/// or access time: no unprivileged restore can set a change time, and a
/// read during the recording moves the access time.
fn stat_fields(result: &std::io::Result<std::fs::Metadata>) -> String {
    use std::os::unix::fs::MetadataExt;
    match result {
        Ok(m) => format!("{:o} {} {} {} {} {} {} {} {}", m.mode(), m.len(), m.mtime(), m.mtime_nsec(), m.nlink(), m.uid(), m.gid(), m.ino(), m.dev()),
        Err(error) => format!("!{}", error.raw_os_error().unwrap_or(-1)),
    }
}

/// A step ends: LABEL names it (a command, or a form's file and line), and
/// FINGERPRINT is the VM's state after it.
pub(crate) fn step(label: impl FnOnce() -> String, fingerprint: impl FnOnce() -> String) {
    if !active() {
        return;
    }
    let label = clean(&label());
    let line = format!("s\t{label}\t{}", fingerprint());
    FLIGHT.with(|f| {
        let mut flight = f.borrow_mut();
        let Some(flight) = flight.as_mut() else { return };
        match flight {
            Flight::Record(r) => { r.steps += 1; let _ = writeln!(r.out, "{line}"); }
            Flight::Replay(p) => {
                p.steps += 1;
                if !p.live {
                    match p.log.get(p.at) {
                        Some(logged) if *logged == line => p.at += 1,
                        Some(logged) if logged.starts_with("s\t") => {
                            p.at += 1;
                            let logged = logged.clone();
                            p.diverge("step", p.steps, logged, line);
                        }
                        logged => {
                            let logged = logged.cloned().unwrap_or_else(|| "the log's end".into());
                            p.diverge("step", p.steps, logged, line);
                            p.live = true;
                        }
                    }
                }
                p.step = label;
            }
        }
    });
}

impl Recorder {
    fn put(&mut self, name: &str, payload: &str) {
        self.calls += 1;
        let _ = writeln!(self.out, "h\t{name}\t{payload}");
    }
}

impl Player {
    /// The logged payload of the next host call, when it is NAME's; a
    /// divergence and live answers from then on when it is not.
    /// WHAT names the call's input, for a divergence.
    fn take(&mut self, name: &str, what: impl Fn() -> String) -> Option<String> {
        self.calls += 1;
        if self.live {
            return None;
        }
        let entry = self.log.get(self.at).and_then(|line| line.strip_prefix("h\t")).and_then(|rest| rest.split_once('\t'));
        match entry {
            Some((logged, payload)) if logged == name => {
                self.at += 1;
                Some(payload.to_owned())
            }
            _ => {
                let expected = self.log.get(self.at).cloned().unwrap_or_else(|| "the log's end".into());
                self.diverge("host", self.calls, expected, format!("h\t{name} {}", what()));
                self.live = true;
                None
            }
        }
    }

    fn diverge(&mut self, kind: &'static str, index: u64, expected: String, found: String) {
        if self.divergence.is_none() {
            self.divergence = Some(Divergence { kind, index, after: self.step.clone(), expected, found });
        }
    }
}

/// LABEL on one log field: no tab or line break.
fn clean(label: &str) -> String {
    label.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).take(120).collect()
}

/// A host value the log carries whole.
pub(crate) trait Logged: Sized {
    /// As a log field; times relative to BASE, the run's start.
    fn save(&self, base: std::time::Instant) -> String;
    fn load(field: &str, base: std::time::Instant) -> Option<Self>;
}

impl Logged for u64 {
    fn save(&self, _: std::time::Instant) -> String { self.to_string() }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> { field.parse().ok() }
}

impl Logged for u32 {
    fn save(&self, _: std::time::Instant) -> String { self.to_string() }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> { field.parse().ok() }
}

impl Logged for i32 {
    fn save(&self, _: std::time::Instant) -> String { self.to_string() }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> { field.parse().ok() }
}

impl Logged for bool {
    fn save(&self, _: std::time::Instant) -> String { u8::from(*self).to_string() }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> { Some(field == "1") }
}

impl Logged for Duration {
    fn save(&self, _: std::time::Instant) -> String { format!("{}.{:09}", self.as_secs(), self.subsec_nanos()) }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> {
        let (secs, nanos) = field.split_once('.')?;
        Some(Duration::new(secs.parse().ok()?, nanos.parse().ok()?))
    }
}

impl Logged for std::time::SystemTime {
    fn save(&self, base: std::time::Instant) -> String {
        match self.duration_since(std::time::SystemTime::UNIX_EPOCH) {
            Ok(after) => after.save(base),
            Err(before) => format!("-{}", before.duration().save(base)),
        }
    }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> {
        match field.strip_prefix('-') {
            Some(before) => std::time::SystemTime::UNIX_EPOCH.checked_sub(Duration::load(before, base)?),
            None => std::time::SystemTime::UNIX_EPOCH.checked_add(Duration::load(field, base)?),
        }
    }
}

/// An instant as its distance from the run's start, which every instant
/// of a run follows.
impl Logged for std::time::Instant {
    fn save(&self, base: std::time::Instant) -> String { self.saturating_duration_since(base).save(base) }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> { base.checked_add(Duration::load(field, base)?) }
}

impl Logged for f64 {
    fn save(&self, _: std::time::Instant) -> String { format!("{:016x}", self.to_bits()) }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> { u64::from_str_radix(field, 16).ok().map(f64::from_bits) }
}

impl<T: Logged> Logged for Option<T> {
    fn save(&self, base: std::time::Instant) -> String {
        self.as_ref().map_or_else(|| "-".into(), |value| format!("+{}", value.save(base)))
    }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> {
        if field == "-" { Some(None) } else { T::load(field.strip_prefix('+')?, base).map(Some) }
    }
}

impl<T: Logged> Logged for std::io::Result<T> {
    fn save(&self, base: std::time::Instant) -> String {
        match self {
            Ok(value) => format!("={}", value.save(base)),
            Err(error) => format!("!{}", error.raw_os_error().unwrap_or(libc::EIO)),
        }
    }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> {
        match field.strip_prefix('!') {
            Some(errno) => Some(Err(std::io::Error::from_raw_os_error(errno.parse().ok()?))),
            None => T::load(field.strip_prefix('=')?, base).map(Ok),
        }
    }
}

/// A list, comma-separated: its items must not save a comma.
impl<T: Logged> Logged for Vec<T> {
    fn save(&self, base: std::time::Instant) -> String {
        self.iter().map(|item| item.save(base)).collect::<Vec<_>>().join(",")
    }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> {
        if field.is_empty() { return Some(Vec::new()); }
        field.split(',').map(|item| T::load(item, base)).collect()
    }
}

impl<const N: usize> Logged for [u64; N] {
    fn save(&self, base: std::time::Instant) -> String { self.to_vec().save(base) }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> { Vec::<u64>::load(field, base)?.try_into().ok() }
}

impl Logged for crate::host::FsUsage {
    fn save(&self, base: std::time::Instant) -> String {
        [self.block, self.fragment, self.blocks, self.free, self.available].save(base)
    }
    fn load(field: &str, base: std::time::Instant) -> Option<Self> {
        let [block, fragment, blocks, free, available] = <[u64; 5]>::load(field, base)?;
        Some(crate::host::FsUsage { block, fragment, blocks, free, available })
    }
}

/// Bytes, as hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bytes(pub Vec<u8>);

impl Logged for Bytes {
    fn save(&self, _: std::time::Instant) -> String {
        self.0.iter().fold(String::with_capacity(self.0.len() * 2), |mut out, b| { let _ = write!(out, "{b:02x}"); out })
    }
    fn load(field: &str, _: std::time::Instant) -> Option<Self> {
        field.len().is_multiple_of(2).then_some(())?;
        (0..field.len()).step_by(2).map(|i| u8::from_str_radix(field.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>().map(Bytes)
    }
}

/// A digest of BYTES that stays the same across processes and builds.
pub(crate) fn digest(bytes: &[u8]) -> u64 {
    // FNV-1a: no per-process key, unlike std's RandomState.
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The witness of an outcome whose value is WITNESS's, or of its error.
pub(crate) fn outcome<T>(result: &std::io::Result<T>, witness: impl FnOnce(&T) -> u64) -> u64 {
    match result {
        Ok(value) => witness(value),
        Err(error) => 0xe000_0000_0000_0000 | error.raw_os_error().unwrap_or(-1) as u32 as u64,
    }
}

thread_local! {
    /// Each buffer's text digest, by its object (`Value` bits) and the
    /// box holding its text, with the `chars_modiff` it was taken at: only
    /// a buffer whose text changed since the last step is hashed again.
    static TEXTS: RefCell<std::collections::HashMap<(u64, usize), (u64, u64)>> = RefCell::new(Default::default());
}

/// The digest of the text in CHUNKS, read where they lie.
fn chunks_digest<'a>(chunks: impl Iterator<Item = &'a [u8]>) -> u64 {
    chunks.fold(0xcbf2_9ce4_8422_2325u64, |hash, chunk| {
        chunk.iter().fold(hash, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    })
}

impl crate::Vm {
    /// The state a step leaves: the heap tables' lengths and live counts
    /// (fresh objects' identities are their slots), the symbols, the
    /// collector's counts, and the live buffers' texts in buffer-list order
    /// with the current buffer. One field per part, so a difference names
    /// its part. A step costs a look at each buffer's tick, and a hash of
    /// only the texts that changed (`TEXTS`). An entry of the buffer order
    /// that is no buffer is an error, not a skipped entry.
    pub(crate) fn flight_fingerprint(&self) -> crate::Result<String> {
        let mut text = 0u64;
        let mut buffers = 0usize;
        TEXTS.with(|texts| -> crate::Result<()> {
            let mut texts = texts.borrow_mut();
            for buffer in self.buffer_order_snapshot() {
                let state = self.buffer_state(buffer)?;
                if !state.live { continue; }
                let tick = state.modification.chars_modiff;
                let object = self.object_if(buffer);
                let key = match object {
                    Some(crate::data::Object::Buffer(contents)) => (buffer.0, &**contents as *const _ as usize),
                    Some(crate::data::Object::ByteBuffer(contents)) => (buffer.0, &**contents as *const _ as usize),
                    _ => continue,
                };
                let hash = match texts.get(&key) {
                    Some(&(at, hash)) if at == tick => hash,
                    _ => {
                        let hash = match object {
                            Some(crate::data::Object::Buffer(contents)) => chunks_digest(contents.chunks_in(0..usize::MAX)),
                            Some(crate::data::Object::ByteBuffer(contents)) => digest(&contents.bytes),
                            _ => 0,
                        };
                        texts.insert(key, (tick, hash));
                        hash
                    }
                };
                buffers += 1;
                text = text.rotate_left(5) ^ hash;
            }
            // Killed buffers' entries go once they are half the cache.
            if texts.len() > 2 * buffers {
                let order: std::collections::HashSet<u64> = self.buffer_order_snapshot().iter().map(|b| b.0).collect();
                texts.retain(|(object, _), _| order.contains(object));
            }
            Ok(())
        })?;
        let gc = &self.gc_stats;
        Ok(format!("conses={}/{} objects={}/{} closures={}/{} cells={}/{} symbols={} gc={}/{}/{}/{}/{} buffers={buffers} text={text:016x} current={:x}",
                self.conses.len(), self.conses.live(), self.objects.len(), self.objects.live(),
                self.closures.len(), self.closures.live(), self.cells.len(), self.cells.live(), self.slots.len(),
                gc.collections, gc.allocations, gc.reclaimed, gc.live_objects, gc.live_bytes,
                self.slots[self.buffer_slot].value.0))
    }

    /// The fingerprint a flight step logs: an error's text in its place.
    pub(crate) fn flight_step_fingerprint(&self) -> String {
        self.flight_fingerprint().unwrap_or_else(|error| format!("error {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each logged kind loads back as it was saved.
    #[test]
    fn logged_values_load_as_they_were_saved() {
        let base = std::time::Instant::now();
        let later = base + Duration::from_millis(1234);
        assert_eq!(std::time::Instant::load(&later.save(base), base), Some(later));
        let time = std::time::SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 5);
        assert_eq!(std::time::SystemTime::load(&time.save(base), base), Some(time));
        assert_eq!(Option::<u64>::load(&Some(7u64).save(base), base), Some(Some(7)));
        assert_eq!(Option::<u64>::load(&None::<u64>.save(base), base), Some(None));
        assert_eq!(Vec::<f64>::load(&vec![0.5, 1.25].save(base), base), Some(vec![0.5, 1.25]));
        assert_eq!(Bytes::load(&Bytes(vec![0, 255, 16]).save(base), base), Some(Bytes(vec![0, 255, 16])));
        let error: std::io::Result<u32> = Err(std::io::Error::from_raw_os_error(libc::ENOENT));
        let loaded = std::io::Result::<u32>::load(&error.save(base), base).unwrap();
        assert_eq!(loaded.unwrap_err().raw_os_error(), Some(libc::ENOENT));
        assert_eq!(<[u64; 4]>::load(&[1u64, 2, 3, 4].save(base), base), Some([1, 2, 3, 4]));
    }

    /// A text's digest does not depend on where its chunks break.
    #[test]
    fn a_chunked_digest_is_the_flat_digest() {
        let text = b"one two three";
        assert_eq!(chunks_digest([&text[..4], &text[4..9], &text[9..]].into_iter()), digest(text));
    }

    /// The digest has no per-process key: the same bytes give the same
    /// digest in every process.
    #[test]
    fn the_digest_is_fixed() {
        assert_eq!(digest(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
