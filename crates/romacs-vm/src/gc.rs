//! Strand-local stop-at-wait-point compacting collector. No process-global state.
use super::*;

#[derive(Clone, Copy, Debug, Default, romacs_snapshot::Snapshot)]
pub struct HeapStats {
    pub collections: u64,
    pub allocations: u64,
    pub reclaimed: u64,
    pub live_objects: usize,
    pub live_bytes: usize,
    pub capacity_bytes: usize,
    /// Wall time spent collecting, every collection included (GNU's
    /// `gc-elapsed' reports it).
    pub elapsed_ns: u64,
}

/// Values a native frame keeps across a call that can run Lisp: a span of
/// `external_roots`, pushed before the call and read back by index after it,
/// so only the forwarded copy is used. Frames nest; release each in reverse
/// order on every exit, the failing ones included. A panic leaves them to
/// whatever catches it (`Vm::catching`).
#[must_use = "a root frame is released on every exit"]
pub(crate) struct RootFrame {
    base: usize,
    len: usize,
    /// Only compared, to refuse a frame of another VM.
    vm: *const Vm,
}
// SAFETY: a frame holds indices into its VM's traced `external_roots`, never a
// Value; every read, write and release checks the VM is the frame's.
unsafe impl crate::brand::Plain for RootFrame {}

/// Test mode: every Nth Lisp entry (`threaded::execute_args`) collects and
/// moves every live object, so a heap Value a native frame kept in a Rust
/// local across the call becomes a wrong object. Enabled by
/// `ROMACS_GC_STRESS=N` or [`Vm::set_gc_stress`]; off (0) by default.
#[derive(Clone, Copy, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Stress {
    every: u32,
    entries: u32,
    /// Dead fillers the last collection left at the front of each arena.
    pads: usize,
    /// The collection about to run is a forced one.
    shift: bool,
    /// `ROMACS_GC_VERIFY': a freed entry is poisoned and never reused
    /// (`Table::sweep'), so a stale reference to it is seen, not hidden by
    /// the entry's next tenant.
    pub(crate) verify: bool,
    /// Every collection compacts ([`Vm::set_gc_moving`]): the fixtures that
    /// prove rooting under the moving collector.
    moving: bool,
    /// Mark on this many threads, whatever the process would
    /// (`ROMACS_GC_WORKERS' or [`Vm::set_gc_threads`]); 0 leaves that to it.
    pub(crate) threads: usize,
    /// Stress collections free in place instead of moving
    /// (`ROMACS_GC_INPLACE'): free entries are reused at once, so a Value a
    /// native frame kept names something else.
    inplace: bool,
    /// Automatic collections may be minors: on, unless `ROMACS_GC_MINOR` is
    /// 0 or [`Vm::set_gc_minor`] says otherwise.
    minor: bool,
    /// Faults a test plants in this VM's collections, and no other's.
    #[cfg(test)]
    pub(crate) faults: Faults,
    /// Graves this VM's collections have handed the dropper (or held, for a
    /// test that holds them), for a test to ask.
    #[cfg(test)]
    pub(crate) buried: usize,
    /// The last mark's weak rounds and `lambda_lists` entries traced, for a
    /// test to ask.
    #[cfg(test)]
    pub(crate) last_mark: (usize, usize),
}

/// What a test breaks in a collection, to see a check catch it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Faults {
    /// No string's properties handed over, in what both marks share.
    pub(crate) skip_properties: bool,
    /// The first chunk a pool worker takes panics.
    pub(crate) worker_panics: bool,
    /// A cons allocated between the mark and the scan, or the sweep.
    pub(crate) allocate_before_scan: bool,
    pub(crate) allocate_before_sweep: bool,
    /// A minor panics once it has begun, before its mark.
    pub(crate) minor_panics: bool,
}

/// Records a session keeps between its usage log's writes: at one a minute
/// or so, this holds a burst; past it collections are only counted.
pub(crate) const COLLECTION_RECORDS: usize = 256;

/// A finished collection, for a real pause to name itself in the usage log
/// (lisp/session/romacs-usage-log.el). MS bounds what a key or command waited.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct CollectionRecord {
    /// The idle pass's or a prompt's: it gave way to no key while marking.
    pub(crate) idle: bool,
    /// Asked for by `garbage-collect' or the host.
    pub(crate) explicit: bool,
    /// A minor: only what was made since the last collection could go.
    pub(crate) minor: bool,
    /// Its pause in microseconds: a 120 Hz frame is 8.3 ms.
    pub(crate) us: u32,
    /// The command it ran in or at the end of, "anonymous" for a closure: a
    /// key ended the last idleness and nothing has begun another.
    pub(crate) command: Option<String>,
    /// Outside a command, a key press was queued when it ended.
    pub(crate) key: bool,
    /// In a prompt's redisplay, the host taken for the read: no key can be
    /// seen, and one pressed waits.
    pub(crate) redisplay: bool,
    /// When it ended, in seconds as `float-time` reads, if a clock timed it
    /// (`host::collection_clock`).
    pub(crate) at: Option<f64>,
    /// What it left live in the private heap: `private_objects`, and
    /// `Vm::live_bytes_estimate`.
    pub(crate) objects: usize,
    pub(crate) bytes: usize,
    /// Objects past the trigger when it began, at the collector's charge
    /// (`collection_budget`): mostly what allocation ran on between falling
    /// due and the poll that took it. A minor's is the bytes handed out past
    /// `MINOR_BYTES` over a cons's 16 (`Vm::young_bytes`), which weigh each
    /// object by its size: not the full's charge, object for object. A
    /// trigger that moved down (a lowered threshold or percentage, a lifted
    /// floor, minors allowed again) or a hold that just ended gives a large
    /// value with a prompt poll, so a consumer sizing a nursery from it reads
    /// its upper percentiles with that in mind. A float, as the trigger is.
    pub(crate) late: f64,
}

/// The collections since `Vm::take_collections', at most COLLECTION_RECORDS.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct CollectionLog {
    records: Vec<CollectionRecord>,
    /// Collections past the limit.
    dropped: u64,
    /// The running collection was asked for.
    pub(crate) explicit: bool,
}

/// What a collection does, in order.
#[derive(Clone, Copy)]
pub(crate) enum Phase { Setup, Roots, Mark, Weak, Retire, Census, Obarray, Sweep, Shrink, Settle, Rewrite }
/// The names `romacs--gc-phases' answers, by `Phase'.
const PHASES: [&str; 11] = ["setup", "roots", "mark", "weak", "retire", "census", "obarray", "sweep", "shrink", "settle", "rewrite"];
/// Where the last completed collection's time went, in nanoseconds by
/// `Phase': zero in a world build, whose heap never depends on a clock.
#[derive(Clone, Copy, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct GcTimes {
    pub(crate) ns: [u64; PHASES.len()],
    /// Entries marked, per table: conses, closures, cells, objects.
    pub(crate) marked: [usize; 4],
    /// Entries the collection found in each table.
    pub(crate) entries: [usize; 4],
    /// Image entries a full read as roots, per table (`Vm::image_roots`).
    pub(crate) remembered: [usize; 4],
}

/// What a completed mark holds, for the rest of the collection.
struct Marked {
    marking: gc_mark::Marking,
    own: gc_mark::Worker,
    doomed: Vec<Value>,
    code_marks: Vec<bool>,
    census: Census,
    /// The payload (`data::payload_bytes`) of everything it marked.
    payload: usize,
    state: MarkState,
}

/// What a mark is for: a collection, which a key press ends while it marks
/// when UNTIL_KEY; or a mark alone (`Vm::mark_only'), which keeps every weak
/// entry when KEEP_WEAK.
#[derive(Clone, Copy)]
enum MarkFor {
    Collection { until_key: bool },
    Only { keep_weak: bool },
    /// A minor's mark: of the functions, it reads those changed since the
    /// last collection.
    Minor,
}

/// What a mark twin's mark found beside the tables' colours (`Vm::mark_only'):
/// what `check_reachable' compares a mark with.
pub(crate) struct MarkedSet {
    pub(crate) code_marks: Vec<bool>,
    pub(crate) obarray_symbols_live: HashSet<usize>,
    pub(crate) region_marks: Vec<bool>,
    /// Weak tables the weak rounds looked through, one per table per round.
    #[cfg(test)]
    pub(crate) weak_visits: usize,
}

/// The symbols of obarrays of their own a mark keeps: those it reached, or
/// every one, to a minor, which reclaims none.
#[derive(Default, Debug, PartialEq, Eq)]
struct KeptSymbols {
    all: bool,
    reached: HashSet<usize>,
}

impl KeptSymbols {
    /// Every symbol kept: a minor's.
    fn all() -> Self {
        Self { all: true, reached: HashSet::default() }
    }
    fn contains(&self, slot: usize) -> bool {
        self.all || self.reached.contains(&slot)
    }
    /// Keep SLOT: whether the mark has it to read now, first met.
    fn insert(&mut self, slot: usize) -> bool {
        !self.all && self.reached.insert(slot)
    }
}

/// What the coordinator gathers while marking.
#[derive(Default)]
struct MarkState {
    native_nodes: HashSet<usize>,
    /// How this mark reads storage no entry owns alone (`crate::epoch`):
    /// a minor reads the char-table nodes and the text properties written
    /// since the VM's last collection, anything else reads them whole.
    reading: properties::Reading,
    /// Weak hash tables reached; their entries are not traced until
    /// `weak_round' (the ephemeron fixpoint) decides which survive.
    weak_tables: Vec<usize>,
    /// How many of `weak_tables' a mark that keeps every weak entry has
    /// kept: their entries are pushed once, so they need no later round.
    weak_kept: usize,
    /// Obarrays reached: each drops its removed entries of dead symbols.
    live_obarrays: Vec<usize>,
    obarray_symbols_live: KeptSymbols,
    /// Frozen regions reached (`frozen.rs'); region 0 is the executable
    /// image, always live.
    region_marks: Vec<bool>,
    /// Buffers reached whose undo list `Vm::undo_round` has yet to look at.
    undo_owners: Vec<usize>,
    /// What it decided, for once the mark can no longer be abandoned.
    undo: undo::UndoDecided,
    /// A mark that keeps every weak entry (a mark twin's) holds undo lists
    /// whole too: what it leaves unmarked must be dead while Lisp runs on,
    /// and no list is cut for it.  So does a minor's: it applies no undo
    /// edits, and the next full collection decides the lists.
    undo_strong: bool,
    /// `lambda_lists` keys whose entry is not traced yet: each is traced
    /// once, by the round that finds its list live, and those left at the
    /// fixpoint are the dead lists.
    untraced_lists: Vec<u64>,
    /// The objects storing their own writes (`shares_storage`) the mark met.
    shared_seen: Vec<u32>,
    /// A minor's old holders of shared storage nothing stored into since the
    /// last collection, in index order: read again for their text, while
    /// their buffer's locals, whose every store marks the buffer, name
    /// nothing young.
    clean_holders: Vec<u32>,
    /// Weak tables looked through, one per table per round (`MarkedSet`).
    #[cfg(test)]
    weak_visits: usize,
    /// Weak rounds run and `lambda_lists` entries traced (`Stress::last_mark`).
    #[cfg(test)]
    last_mark: (usize, usize),
}

/// A running phase clock (`GcTimes').
pub(crate) struct Lap(pub(crate) Option<std::time::Instant>);
impl Lap {
    pub(crate) fn start() -> Self { Self(crate::host::collection_clock()) }
    /// Charge the time since the last lap to PHASE of TIMES.
    pub(crate) fn lap(&mut self, times: &mut GcTimes, phase: Phase) {
        let Some(at) = self.0 else { return };
        // HOST(Instant::now): a collection's phase clock (`romacs--gc-phases'), running only once
        // `host::collection_clock' started it, which no builder does, its heap being the image
        let now = std::time::Instant::now();
        times.ns[phase as usize] += (now - at).as_nanos() as u64;
        self.0 = Some(now);
    }
}

/// What the funnels count as they hand entries out (`Vm::alloc_cons' and its
/// siblings, data.rs).
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Allocation {
    /// Bytes the funnels have handed out, ever. Two meters read it: the
    /// memory profiler's (profiler.rs), which table growth no longer
    /// measures, and the engine's request to poll (`Vm::poll_requested`).
    /// So every funnel adds what it allocates, whatever its kind.
    pub(crate) allocated: u64,
    /// Where ALLOCATED next asks the running engine to poll: `POLL_STEP`
    /// past the last settled request, or 0 to poll now (`Vm::request_poll`).
    pub(crate) poll_at: u64,
    /// GNU's consing counters, ever (`memory-use-counts').
    pub(crate) consed: Consed,
    /// The next collection compacts (`Vm::collect_compacting').
    pub(crate) compact: bool,
    /// ALLOCATED when the last collection ended: what was handed out since
    /// is the young, and makes a minor due (`Vm::minor_due`).
    pub(crate) collected_at: u64,
    /// The epoch the last collection ended at (`crate::epoch`): storage
    /// stamped below it holds only what that collection, or one before,
    /// promoted.
    pub(crate) since: u64,
}

/// What the last mark found in the objects table: `garbage-collect's
/// statistics and the pacing's traced slots, counted once per entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Census {
    pub(crate) strings: usize,
    pub(crate) string_bytes: usize,
    pub(crate) vectors: usize,
    /// Slots of vectors and records.
    pub(crate) vector_slots: usize,
    pub(crate) vector_free: usize,
    pub(crate) floats: usize,
    pub(crate) buffers: usize,
}
impl Census {
    pub(crate) fn add(&mut self, other: &Self) {
        self.strings += other.strings;
        self.string_bytes += other.string_bytes;
        self.vectors += other.vectors;
        self.vector_slots += other.vector_slots;
        self.vector_free += other.vector_free;
        self.floats += other.floats;
        self.buffers += other.buffers;
    }
}


/// What `memory-use-counts' reports, in its units; symbols are counted from
/// the symbol table, and VM2 has no intervals
/// to count (VM2-MEMCOUNT1).
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Consed {
    pub(crate) conses: u64,
    pub(crate) floats: u64,
    pub(crate) vector_cells: u64,
    pub(crate) reused_symbols: u64,
    pub(crate) string_chars: u64,
    pub(crate) strings: u64,
}

/// What a freed cons or cell holds under `ROMACS_GC_VERIFY': a primitive
/// index no table has, which no operation accepts.
pub(crate) const POISON: Value = Value(((u32::MAX as u64) << 3) | 6);

/// What can stand at a `Vm::catching` outside any engine, for a panic to put
/// back: an embedder's root frames and holds, the bindings, handlers and
/// backtrace entries below it, watchers already running, and the Lisp depth.
struct NestingMark {
    roots: usize,
    holds: usize,
    dynamic: usize,
    retrap: usize,
    handlers: usize,
    backtrace: usize,
    eval_depth: usize,
}

/// Bytes charged per heap object against `gc-cons-threshold': GNU's cons.
const CONS_BYTES: i64 = 16;
/// Allocation between the running engine's requests to poll for a
/// collection (`Vm::poll_requested`).
const POLL_STEP: u64 = 256 << 10;
/// Bytes handed out since the last collection that make a minor due: 256K
/// objects at a cons's charge. A late poll adds up to `POLL_STEP`.
const MINOR_BYTES: u64 = (256 << 10) * CONS_BYTES as u64;
/// alloc.c:7508, the C initializer of `memory-signal-data`. VM2 has no such
/// variable, so GNU's startup.el:1738 rewrite is not observable (VM2-FR5-2).
pub(crate) const MEMORY_FULL: &str = "Memory exhausted--use M-x save-some-buffers then exit and restart Emacs";
/// GNU's floor under `gc-cons-threshold' (alloc.c:5747, GC_DEFAULT_THRESHOLD / 10).
const MIN_CONS_THRESHOLD: i64 = 80_000;
/// Traced heaps a collection waits for, per unit of `gc-cons-percentage':
/// GNU's 0.1 collects after one traced heap (VM2-GC-PACING).
const DEFERRAL_SCALE: f64 = 10.0;
/// The most traced heaps a large setting can defer by.
const MAX_DEFERRAL: f64 = 18.0;
/// Whether the mark must visit V: a heap reference, or a symbol interned in
/// an obarray of its own, the only symbols a collection reclaims. Anything
/// else the mark would pop and discard.
#[inline(always)]
pub(crate) fn traced(v: Value, in_obarray: &[bool]) -> bool {
    match v.tag() {
        0 => v != Value::NIL,
        3..=5 => true,
        7 => v.payload() >= 1 && in_obarray.get(v.payload() as usize - 1).copied().unwrap_or(false),
        _ => false,
    }
}
/// Call caches filled since the last collection that `Vm::caches_filled`
/// records; a minor after more asks every cache.
pub(crate) const CACHES_FILLED: usize = 1 << 16;
/// Whether OBJECT keeps storage whose writes no store into its entry
/// colours: buffer text and char-table nodes, each with its own record of
/// what was written since (`crate::epoch`), which a minor must look at for
/// an old one as it does for a dirty entry.
pub(crate) fn shares_storage(object: &data::Object) -> bool {
    matches!(object, data::Object::Buffer(_) | data::Object::ByteBuffer(_) | data::Object::CharTable(_) | data::Object::SubCharTable(_) | data::Object::MapKeymapCursor(_))
}
/// Values an idle collection marks between its looks for a key press
/// (`Vm::collect_marked'): a millisecond or two of marking.
const KEY_POLL: usize = 16_384;
/// `gc-cons-percentage' P as traced heaps to wait for, in proportion up to
/// MAX_DEFERRAL; a negative or NaN P leaves only the threshold, as GNU's does.
pub(crate) fn deferral(p: f64) -> f64 {
    if p.is_nan() { 0.0 } else { (p * DEFERRAL_SCALE).clamp(0.0, MAX_DEFERRAL) }
}

impl Stress {
    pub(crate) fn from_env() -> Self {
        // HOST(std::env::var): the collector's thread count for a test (ROMACS_GC_WORKERS): what a
        // collection leaves is the same at any count; invisible to Lisp, read again on restore
        // (Stress::rearm)
        let every = std::env::var("ROMACS_GC_STRESS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        // HOST(std::env::var_os): the collector's in-place stress mode (ROMACS_GC_INPLACE): stress
        // collections free in place instead of moving; invisible to Lisp, read again on restore
        // (Stress::rearm)
        let verify = std::env::var_os("ROMACS_GC_VERIFY").is_some_and(|v| !v.is_empty() && v != "0");
        // HOST(std::env::var): the collector's test mode (ROMACS_GC_STRESS): collections are
        // invisible to Lisp, so no world keys it, and a restored VM reads it again (Stress::rearm)
        let threads = std::env::var("ROMACS_GC_WORKERS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        // HOST(std::env::var_os): the collector's verify mode (ROMACS_GC_VERIFY): freed entries are
        // poisoned, never reused; invisible to Lisp, read again on restore (Stress::rearm)
        let inplace = std::env::var_os("ROMACS_GC_INPLACE").is_some_and(|v| !v.is_empty() && v != "0");
        // HOST(std::env::var_os): whether automatic collections may be minors (ROMACS_GC_MINOR): a
        // minor frees only what nothing reaches, as a full does; invisible to Lisp, read again on
        // restore (Stress::rearm)
        let minor = std::env::var_os("ROMACS_GC_MINOR").is_none_or(|v| v != "0");
        Self { every, verify, threads, inplace, minor, ..Self::default() }
    }
    /// This process's test mode, from its environment, for a VM a world
    /// carried here: the fillers the last collection left stay counted.
    pub(crate) fn rearm(&mut self) {
        let fresh = Self::from_env();
        (self.every, self.entries, self.verify, self.threads, self.inplace, self.minor) = (fresh.every, fresh.entries, fresh.verify, fresh.threads, fresh.inplace, fresh.minor);
        // A child or a restored VM plants none of its parent's faults.
        #[cfg(test)]
        { self.faults = Faults::default(); }
    }
    /// Count one Lisp entry; true when this one must collect.
    pub(crate) fn due(&mut self) -> bool {
        if self.every == 0 {
            return false;
        }
        self.entries += 1;
        self.entries % self.every == 0
    }
    /// Fillers for the next collection: a forced one changes the count from
    /// the last, so every object with no other garbage below it moves.
    fn take_pads(&mut self) -> usize {
        let pads = match std::mem::take(&mut self.shift) {
            false => 0,
            true if self.pads == 1 => 2,
            true => 1,
        };
        self.pads = pads;
        pads
    }
}

/// What a retention probe measured at each workload size.
#[derive(Clone, Debug, Default)]
pub struct RetentionReport {
    pub sizes: Vec<usize>,
    pub objects: Vec<usize>,
    pub bytes: Vec<usize>,
    pub collections: u64,
}

/// Run WORKLOAD at ascending SIZES and require retention to be flat in **both**
/// objects and bytes.
///
/// Three independent lanes have shipped the same defect through a green suite by
/// asserting only one of the two dimensions: a buffer-local table whose
/// `live_objects` stayed at 5 while `live_bytes` went 41,480 -> 82,672 ->
/// 165,056; a weak-key table that removed entries without ever compacting its
/// storage; and a window parent that dropped children with `retain`, which does
/// not reduce capacity. Objects alone cannot see a container that frees entries
/// but keeps its buffer, and bytes alone cannot see an object leak inside a
/// fixed allocation, so a retention test has to assert both or it is measuring
/// half the heap.
///
/// Flat, not merely sub-linear: a workload whose final state is identical at
/// every size must retain an identical amount, so any growth at all is the bug.
/// The panic names which dimension grew and prints both series, because "bytes
/// grew" and "objects grew" have completely different causes.
///
/// WORKLOAD must leave the heap in the *same* logical state for every size --
/// build `n` things and dispose of them -- otherwise flatness is not the
/// property you want and this helper is the wrong tool.
pub fn assert_no_retention_growth(
    vm: &mut Vm,
    sizes: &[usize],
    mut workload: impl FnMut(&mut Vm, usize),
) -> RetentionReport {
    assert!(
        sizes.len() >= 3,
        "a retention probe needs at least three sizes to distinguish flat from growing, got {:?}",
        sizes
    );
    let before = vm.heap_stats().collections;
    let mut report = RetentionReport::default();
    for &n in sizes {
        workload(vm, n);
        vm.collect(&mut []);
        let s = vm.heap_stats();
        report.sizes.push(n);
        report.objects.push(s.live_objects);
        report.bytes.push(s.live_bytes);
        eprintln!(
            "retention size={n} collections={} objects={} bytes={} capacity={}",
            s.collections, s.live_objects, s.live_bytes, s.capacity_bytes
        );
    }
    report.collections = vm.heap_stats().collections - before;
    assert_eq!(
        report.collections as usize,
        sizes.len(),
        "each size must be measured after exactly one counted collection: {report:?}"
    );
    let flat = |v: &[usize]| v.iter().all(|x| *x == v[0]);
    assert!(
        flat(&report.objects),
        "retained OBJECTS grew with workload size -- an object leak: sizes={:?} objects={:?} bytes={:?}",
        report.sizes,
        report.objects,
        report.bytes
    );
    assert!(
        flat(&report.bytes),
        "retained BYTES grew while objects stayed flat -- a container that drops \
         entries without reclaiming storage: sizes={:?} objects={:?} bytes={:?}",
        report.sizes,
        report.objects,
        report.bytes
    );
    report
}

impl Vm {
    /// Run BODY outside any engine and, if it panics, put the VM back as it
    /// was, for a caller that goes on using it: a tool scoring a file, a test.
    ///
    /// The invariant: **a session never uses its VM after a panic.** A panic or
    /// `kill-emacs` ends it, and its VM with it, so nothing on the way in pays
    /// to be recoverable, and no frame restores anything as a panic crosses it.
    /// `tests/panic_boundary.rs` lists every `catch_unwind` that could reach a
    /// VM, and fails on a new one. What a panic left is
    /// dropped: suspended engines' state, root frames, holds; the bindings and
    /// handlers made since are popped, without their watchers, and no cleanup
    /// runs; what they owned (`romacs--post-gc-owned`, `inhibit-quit`, a job's
    /// claim) ends with them, and a variable whose watchers were running is
    /// trapped again (`Vm::retrap`).
    ///
    /// # Errors
    /// The panic's payload, if BODY panicked.
    ///
    /// # Panics
    /// If an engine is running: a panic is caught outside any.
    pub fn catching<T>(&mut self, body: impl FnOnce(&mut Vm) -> T) -> std::thread::Result<T> {
        assert_eq!(self.running_engines, 0, "a panic is caught outside any engine");
        let mark = NestingMark {
            roots: self.external_roots.len(),
            holds: self.gc_holds,
            dynamic: self.dynamic.len(),
            retrap: self.retrap.len(),
            handlers: self.signal_handlers.len(),
            backtrace: self.backtrace.frames.len(),
            eval_depth: self.eval_depth,
        };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(self)));
        if caught.is_err() {
            self.restore_nesting(&mark);
        }
        caught
    }
    fn restore_nesting(&mut self, mark: &NestingMark) {
        // First, while the backtrace still names them.
        let trapped = std::mem::take(&mut self.trapped);
        self.unwind(self.dynamic.len().saturating_sub(mark.dynamic));
        self.trapped = trapped;
        for base in self.retrap.drain(mark.retrap.min(self.retrap.len())..) { self.trapped.insert(base); }
        self.external_roots.truncate(mark.roots);
        self.gc_holds = mark.holds;
        self.signal_handlers.truncate(mark.handlers);
        self.backtrace.leave(mark.backtrace);
        self.eval_depth = mark.eval_depth;
        // Outside any engine, these are all at rest.
        self.suspended.clear();
        self.suspended_stacks.clear();
        self.stack = Vec::new();
        self.running_engines = 0;
        self.unwinding = 0;
        self.aftermath_runs = 0;
        self.rooted_write = false;
        self.deferred_writes.clear();
        self.site_leaf = (0, 0);
        // A panic in a host wait drops its host, as it did when the wait
        // kept it on the native stack.
        self.host_wait = None;
        // A collection the panic stopped may have left the generations
        // false: a minor makes the old entries it reads again young for its
        // length, and a sweep stopped part way leaves old, clean survivors
        // naming young entries it did not reach. With everything promoted
        // no such edge is left; nothing is freed.
        self.promote_all();
    }
    pub(crate) fn root_frame(&mut self, values: &[Value]) -> RootFrame {
        let base = self.external_roots.len();
        self.external_roots.extend_from_slice(values);
        RootFrame { base, len: values.len(), vm: std::ptr::from_ref(self) }
    }
    pub(crate) fn rooted(&self, frame: &RootFrame, i: usize) -> Value {
        assert!(std::ptr::eq(frame.vm, self), "a root frame of another VM");
        assert!(i < frame.len, "root frame index");
        self.external_roots[frame.base + i]
    }
    pub(crate) fn set_rooted(&mut self, frame: &RootFrame, i: usize, value: Value) {
        assert!(std::ptr::eq(frame.vm, self), "a root frame of another VM");
        assert!(i < frame.len, "root frame index");
        self.external_roots[frame.base + i] = value;
    }
    /// FRAME's values, forwarded.
    pub(crate) fn rooted_values(&self, frame: &RootFrame) -> &[Value] {
        assert!(std::ptr::eq(frame.vm, self), "a root frame of another VM");
        &self.external_roots[frame.base..frame.base + frame.len]
    }
    /// Release FRAME, returning its forwarded values.
    pub(crate) fn release_frame<const N: usize>(&mut self, frame: RootFrame) -> [Value; N] {
        assert_eq!(frame.len, N, "root frames are released whole");
        let values = std::array::from_fn(|i| self.rooted(&frame, i));
        self.drop_frame(frame);
        values
    }
    pub(crate) fn drop_frame(&mut self, frame: RootFrame) {
        assert!(std::ptr::eq(frame.vm, self), "a root frame of another VM");
        assert_eq!(self.external_roots.len(), frame.base + frame.len, "root frames are released in reverse order");
        self.external_roots.truncate(frame.base);
    }
    /// GNU's `garbage_collection_inhibited`, derived from its owners: a
    /// collection may run only when no frame keeps heap Values in untraced
    /// Rust storage. Those are OWN unsuspended engines (the caller's own),
    /// no other running engine outside `suspended`, no open compiler
    /// builder, no watcher notification from an unrooted native writer, and
    /// no native write waiting for its engine (`deferred_writes`, untraced).
    /// Every other native frame on the stack holds its Values in traced
    /// storage: a binding, a `#:` slot or a [`RootFrame`]
    /// (tests/fixtures/reentry-sites.txt lists each call site).
    pub(crate) fn may_collect(&self, own: usize) -> bool {
        self.gc_holds == 0 && self.deferred_writes.is_empty()
            && self.running_engines.saturating_sub(self.suspended.len()) == own
    }
    /// Set the register stack aside, still traced, its allocation unmoved for
    /// the frames' raw pointers: an engine's for a nested run, or for a
    /// storage kernel's.
    pub(crate) fn suspend_stack(&mut self) {
        let stack = std::mem::take(&mut self.stack);
        self.suspended_stacks.push(stack);
    }
    pub(crate) fn resume_stack(&mut self) {
        debug_assert!(self.stack.is_empty(), "a nested run returned its registers");
        self.stack = self.suspended_stacks.pop().expect("a suspended register stack");
    }
    /// RUN on ARGS in the reused kernel file, the current one set aside beneath
    /// it, still traced, its allocation unmoved: the one way a native borrows
    /// register slots (a storage kernel, a property query) without touching a
    /// running engine's. RUN gets the arguments' count; they start at 0.
    pub(crate) fn with_kernel_stack<T>(&mut self, args: &[Value], run: impl FnOnce(&mut Self, usize) -> T) -> T {
        self.suspend_stack();
        self.stack = std::mem::take(&mut self.kernel_stack);
        self.stack.clear();
        self.stack.extend_from_slice(args);
        let out = run(self, args.len());
        // Untraced between calls: nothing it held may stay in it.
        self.stack.clear();
        self.kernel_stack = std::mem::take(&mut self.stack);
        self.resume_stack();
        out
    }
    /// SLOTS as a dense membership table over every symbol slot.
    fn slot_set<'a>(&self, slots: impl Iterator<Item = &'a usize>) -> Vec<bool> {
        let mut set = vec![false; self.slots.len()];
        for &slot in slots {
            set[slot] = true;
        }
        set
    }
    /// [`Stress`]: collect at every Nth Lisp entry; 0 turns it off. The
    /// verify and moving modes stay as they are.
    pub fn set_gc_stress(&mut self, every: u32) {
        let Stress { verify, moving, threads, inplace, minor, .. } = self.gc_stress;
        #[cfg(test)]
        let (faults, buried) = (self.gc_stress.faults, self.gc_stress.buried);
        self.gc_stress = Stress { every, verify, moving, threads, inplace, minor, ..Stress::default() };
        #[cfg(test)]
        { (self.gc_stress.faults, self.gc_stress.buried) = (faults, buried); }
    }
    /// Mark on N threads, in this VM, for a test. A process that may fork
    /// must not (`gc_mark::set_workers').
    pub fn set_gc_threads(&mut self, n: usize) {
        self.gc_stress.threads = n;
    }
    /// Turn `ROMACS_GC_VERIFY' on or off, for a fixture that needs free
    /// entries reused (or quarantined).
    pub fn set_gc_verify(&mut self, verify: bool) {
        self.gc_stress.verify = verify;
    }
    /// Let automatic collections be minors, or not (`ROMACS_GC_MINOR`).
    pub fn set_gc_minor(&mut self, minor: bool) {
        self.gc_stress.minor = minor;
    }
    /// Make every collection compact, as stress collections do, or return
    /// to compacting on demand (`compacts').
    pub fn set_gc_moving(&mut self, moving: bool) {
        self.gc_stress.moving = moving;
    }
    /// Whether the running engine should poll at its next tick: allocation
    /// passed another `POLL_STEP` since the last settled request, or
    /// something asked for a poll. A primitive's return tests it, as GNU's
    /// `maybe_gc` runs at every call; compiled code's own allocation is
    /// left to the budget's poll.
    #[inline]
    pub(crate) fn poll_requested(&self) -> bool {
        self.allocation.allocated >= self.allocation.poll_at
    }
    /// Ask the running engine, or the one it returns to, to poll soon.
    pub(crate) fn request_poll(&mut self) {
        self.allocation.poll_at = 0;
    }
    /// A poll or a collection settled the request: the next is `POLL_STEP`
    /// of allocation away.
    pub(crate) fn settle_poll(&mut self) {
        self.allocation.poll_at = self.allocation.allocated + POLL_STEP;
    }
    /// GNU `maybe_garbage_collect' (alloc.c:5740-5809): collect once the
    /// allocation since the last collection reaches `gc-cons-threshold'
    /// (at least 80000 bytes) or, when `gc-cons-percentage' is a float, a
    /// proportion of the live heap (`deferral'). Allocation is counted
    /// in objects charged CONS_BYTES each, plus the payload bytes (string
    /// text, vector slots) made since; the live heap is what a collection
    /// traverses, retained objects and compiled code units. Both variables
    /// are read live, so binding them takes effect at once.
    pub(crate) fn collection_due(&self) -> bool {
        self.full_due() || self.minor_due()
    }
    /// Whether a minor is due: minors are on, nothing asks this VM to move
    /// its heap or paces it as a job's child, and `MINOR_BYTES` were handed
    /// out since the last collection. `gc-cons-threshold` paces fulls, by
    /// what the heap has grown since the last one, minors or not.
    pub(crate) fn minor_due(&self) -> bool {
        self.minor_allowed() && self.young_bytes() >= MINOR_BYTES
    }
    /// The bytes the funnels handed out since the last collection
    /// (`Allocation::allocated`): what makes a minor due.
    fn young_bytes(&self) -> u64 {
        self.allocation.allocated - self.allocation.collected_at
    }
    /// Whether this VM's automatic collections may be minors at all.
    fn minor_allowed(&self) -> bool {
        self.gc_stress.minor && !self.will_compact() && self.job_budget.is_none()
    }
    /// Whether the automatic collection about to run is a minor: allocation
    /// since the last collection alone made it due, or stress forced it
    /// where minors are allowed (stress that frees in place), so that every
    /// forced collection puts a minor before verify's oracle. Whatever else
    /// asked is owed a full: the heap's growth, a scarce region id, or the
    /// request code compiled since the last one, which only a full at a
    /// request's boundary retires.
    pub(crate) fn minor_chosen(&self) -> bool {
        // Owed where it can be paid: only a collection at a request's
        // boundary retires request code and starts the count again.
        let code = self.stack.is_empty() && self.suspended_stacks.is_empty()
            && self.request_code.since_collection >= self.code_collection_threshold();
        !self.full_due() && !code && (self.minor_due() || (self.gc_stress.shift && self.minor_allowed()))
    }
    /// Whether a full collection is due, by GNU's rule above.
    pub(crate) fn full_due(&self) -> bool {
        // Frozen region ids are scarce: at 75% in use, collect now.
        if self.region_collect {
            return true;
        }
        let (allocated, budget) = self.collection_budget();
        allocated >= budget
    }
    /// The objects allocated since the last collection, and how many make
    /// the next automatic one due (GNU's `gc_threshold').
    pub(crate) fn collection_budget(&self) -> (f64, f64) {
        let allocated = self.heap_objects().saturating_sub(self.gc_retained) as f64
            + (self.gc_payload_bytes / CONS_BYTES as usize) as f64;
        // A job's growth over the heap it was forked with, not the live
        // heap's share: an ordinary job never collects.
        if let Some((base, objects)) = self.job_budget {
            return (allocated, objects.saturating_sub(self.job_live().saturating_sub(base)).max(1) as f64);
        }
        // An embedder's floor overrides, as GNU's memory-full state does.
        if let Some(floor) = self.gc_floor {
            return (allocated, self.gc_retained.saturating_mul(2).max(floor).saturating_sub(self.gc_retained) as f64);
        }
        let threshold = self.forwarded.gc_cons_threshold().max(MIN_CONS_THRESHOLD) as f64 / CONS_BYTES as f64;
        let percentage = match self.forwarded.gc_cons_percentage() {
            Ok(value) => match self.object_if(value) {
                Some(data::Object::Float(f)) => Some(*f),
                _ => None,
            },
            Err(initial) => Some(initial),
        };
        let proportional = percentage.map_or(0.0, |p| {
            // A cons, the unit, traces two slots.
            let live = self.gc_retained + self.gc_code + self.gc_retained_slots / 2;
            deferral(p) * live as f64
        });
        (allocated, threshold.max(proportional))
    }
    /// In a child forked for one job: BYTES of growth over the heap it was
    /// forked with, at the collector's charge per object, pace its
    /// collections in place of the default rule. A collection in a child
    /// rewrites what it shares with its zygote, so an ordinary job should
    /// never run one; its garbage dies with the process.
    pub(crate) fn set_job_budget(&mut self, bytes: usize) {
        let objects = (bytes / CONS_BYTES as usize).max(1);
        self.job_budget = Some((self.job_live(), objects));
    }
    /// What the last collection kept, in objects at the collector's charge:
    /// the retained objects and the payload allocation charged for them.
    fn job_live(&self) -> usize {
        self.gc_retained + self.gc_retained_payload / CONS_BYTES as usize
    }
    /// Measure the payload the heap holds now, for `job_live`: a zygote's
    /// baseline, a child's after each collection. One pass over the objects.
    pub(crate) fn measure_retained_payload(&mut self) {
        self.gc_retained_payload = self.objects.iter().map(crate::data::payload_bytes).sum();
    }
    /// Whether what survived the last collection alone exceeds the job's
    /// budget: the child's memory is exhausted.
    fn job_over_budget(&self) -> bool {
        self.job_budget.is_some_and(|(base, objects)| self.job_live().saturating_sub(base) > objects)
    }
    /// How far the next automatic collection is along, 1 being due: the
    /// larger of the objects allocated over their trigger and the request
    /// code compiled over its threshold (`finish_request'), which only a
    /// collection at a request boundary retires.
    pub(crate) fn collection_share(&self) -> f64 {
        let (allocated, budget) = self.collection_budget();
        let code = self.request_code.since_collection as f64 / self.code_collection_threshold() as f64;
        (allocated / budget).max(code)
    }
    /// Objects past the automatic trigger now (`CollectionRecord::late`):
    /// a minor's, `MINOR_BYTES` since the last collection, or a full's.
    pub(crate) fn collection_late(&self, minor: bool) -> f64 {
        if minor {
            return self.young_bytes().saturating_sub(MINOR_BYTES) as f64 / CONS_BYTES as f64;
        }
        let (allocated, budget) = self.collection_budget();
        (allocated - budget).max(0.0)
    }
    /// After a collection: no payload charged since, no poll pending, and the
    /// vector slots the live heap keeps, for the proportional rule.
    fn settle_payload(&mut self) {
        self.gc_payload_bytes = 0;
        self.payload_settled = 0;
        self.allocation.collected_at = self.allocation.allocated;
        self.settle_poll();
        self.gc_retained_slots = self.gc_census.vector_slots;
        if self.job_budget.is_some() { self.measure_retained_payload(); }
    }
    /// A forced stress collection is due at this Lisp entry.
    pub(crate) fn stress_due(&mut self) -> bool {
        let due = self.gc_stress.due();
        self.gc_stress.shift |= due;
        due
    }
    /// Snapshot before constructing the result, just as GNU snapshots gcstat.
    /// Object arenas share spare storage: no typed free string/float/buffer pools.
    pub(crate) fn garbage_collect_statistics(&mut self) -> Value {
        let Census { strings, string_bytes, vectors, vector_slots, vector_free, floats, buffers } = self.gc_census;
        let rows: [(&str, usize, usize, Option<usize>); 8] = [
            ("conses", size_of::<(Value, Value)>(), self.conses.live(), Some(self.conses.capacity() - self.conses.live())),
            ("symbols", size_of::<Slot>(), self.slots.len() - self.obarray_free_slots.len(), Some(self.obarray_free_slots.len())),
            ("strings", size_of::<data::Object>() + size_of::<data::LispString>(), strings, Some(0)),
            ("string-bytes", 1, string_bytes, None),
            ("vectors", size_of::<data::Object>(), vectors, None),
            ("vector-slots", size_of::<Value>(), vector_slots, Some(vector_free)),
            ("floats", size_of::<data::Object>(), floats, Some(0)),
            ("buffers", size_of::<data::Object>(), buffers, None),
        ];
        let mut result = Value::NIL;
        for (name, size, used, free) in rows.into_iter().rev() {
            let mut row = Value::NIL;
            if let Some(free) = free { row = self.keymap_cons(Value::integer(free as i64).expect("heap count fits fixnum"), row); }
            row = self.keymap_cons(Value::integer(used as i64).expect("heap count fits fixnum"), row);
            row = self.keymap_cons(Value::integer(size as i64).expect("heap count fits fixnum"), row);
            let symbol = self.intern(name);
            row = self.keymap_cons(symbol, row);
            result = self.keymap_cons(row, result);
        }
        result
    }

    /// Whether a Value is retained by the current mark: immediates, symbols,
    /// primitives and executable-image objects always survive; a frozen
    /// value survives iff its region is marked (REGIONS).
    fn survives(&self, symbols: &KeptSymbols, regions: &[bool], v: Value) -> bool {
        if Self::image_value(v) {
            let id = crate::frozen::region_of(v.payload() as usize);
            return id == 0 || regions.get(id).copied().unwrap_or(false);
        }
        let i = v.payload() as usize;
        match v.tag() {
            0 if v != Value::NIL => self.objects.marked(i - 1),
            3 => self.conses.marked(i),
            4 => self.closures.marked(i),
            5 => self.cells.marked(i),
            7 if v != Value::UNBOUND && !symbols.all && self.obarray_symbol_names.contains_key(&(i - 1)) => symbols.contains(i - 1),
            _ => true,
        }
    }
    /// Whether a collection ended since `post-gc-hook' last ran and the hook
    /// has a function to run; with none, the flag clears. A run started for
    /// finalizers takes the collection itself and looks at the hook after
    /// them (`romacs--run-finalizers').
    pub(crate) fn post_gc_hook_due(&mut self) -> bool {
        // Asked after every collection: a hook never interned is nil, and
        // asking interns nothing.
        if self.post_gc_due && let Some(&hook) = self.names.get("post-gc-hook") {
            self.post_gc_due = self.variable_value(hook).is_ok_and(|value| value != Value::NIL);
        } else {
            self.post_gc_due = false;
        }
        self.post_gc_due
    }
    /// `romacs--run-finalizers' when a collection left doomed finalizers or a
    /// `post-gc-hook' to run, and the prelude defines it. Lisp runs them, never
    /// the collector. With finalizers doomed the hook is not looked at here:
    /// one of them may yet add to it.
    pub(crate) fn finalizer_runner(&mut self) -> Option<Value> {
        if !self.aftermath_pending() { return None; }
        let &slot = self.names.get("romacs--run-finalizers")?;
        let function = self.slots[slot].function;
        (function != Value::NIL && function != Value::UNBOUND).then(|| Value::reference(slot + 1, 7))
    }
    /// The cheap test of a collection's aftermath: doomed finalizers, an undo
    /// list to offer, or a collection whose `post-gc-hook` is not yet looked
    /// at. Most polls find none.
    #[inline]
    pub(crate) fn aftermath_flagged(&self) -> bool {
        !self.doomed_finalizers.is_empty() || self.undo_offers_due || self.post_gc_due
    }
    /// A collection's aftermath waits: doomed finalizers (one may yet add to
    /// the hook), `undo-outer-limit-function' to ask, or a collection ended
    /// with a `post-gc-hook' to run. Cheap,
    /// for the places that make a poll come soon (a cleanup's end, an
    /// engine's): the hook is read once per collection, the flag clearing
    /// when it is nil (`post_gc_hook_due').
    pub(crate) fn aftermath_pending(&mut self) -> bool {
        !self.doomed_finalizers.is_empty() || self.undo_offers_due || self.post_gc_hook_due()
    }
    /// The gate for every automatic path, with OWN engines of its own running
    /// (1 at an engine's poll or wait, 0 at a request's end): the runner, when
    /// aftermath is pending, nothing holds it back (`Vm::aftermath_held': no
    /// escape unwound, no cleanup or run in progress, in any engine) and the
    /// heap may be collected (VM2-GC-FINALIZER). An explicit `garbage-collect'
    /// asks `finalizer_runner' itself.
    pub(crate) fn safe_runner(&mut self, own: usize) -> Option<Value> {
        if !self.may_collect(own) || self.aftermath_held() { return None; }
        self.finalizer_runner()
    }
    /// Whether anything in progress holds aftermath back: an escape being
    /// unwound, a run of aftermath, or a suspended engine's cleanup. A running
    /// engine answers for its own cleanups (`Engine::safe_runner`), and an
    /// engine that is neither fails `may_collect`.
    pub(crate) fn aftermath_held(&self) -> bool {
        self.unwinding != 0 || self.aftermath_runs != 0 || self.suspended.iter().any(threaded::Suspended::in_cleanup)
    }
    /// alloc.c queue_doomed_finalizers and mark_finalizer_list: the
    /// finalizers marking has not reached are marked, with their functions,
    /// to survive until they run, and go to DOOMED, which the collection
    /// moves to the doomed queue once it can no longer give way to a key
    /// (`collect_marked'). True when any was doomed.
    fn doom_finalizers(&self, pending: &mut Vec<Value>, doomed: &mut Vec<Value>) -> bool {
        let found: Vec<Value> = self.finalizers.iter().copied()
            .filter(|&f| !(Self::image_value(f) || self.objects.marked(f.payload() as usize - 1)))
            .collect();
        pending.extend_from_slice(&found);
        doomed.extend_from_slice(&found);
        !found.is_empty()
    }
    /// GNU `sweep_weak_table (h, false)`, run whenever the worklist drains:
    /// an entry whose keep rule already holds makes its other half live.
    /// LISTS, `lambda_lists` lent to the mark, is an ephemeron table too: a
    /// live list keeps what its entry holds. With KEEP_ALL (a mark twin's,
    /// `Vm::mark_only'), every weak hash table entry is kept, its key live
    /// from then on, so each table's entries go once, in the round after it
    /// is reached: a chain of weak tables costs its length. LISTS stays an
    /// ephemeron table, since a collection reads it only by a live list.
    /// Marking only grows, so the rounds reach a fixpoint; weak tables
    /// reached by the new marks join the next round. True when anything was
    /// pushed.
    fn weak_round(
        &self,
        state: &mut MarkState,
        lists: &mut std::collections::HashMap<u64, source_cache::LambdaList>,
        keep_all: bool,
        pending: &mut Vec<Value>,
    ) -> bool {
        #[cfg(test)]
        { state.last_mark.0 += 1; }
        let (symbols, regions) = (&state.obarray_symbols_live, &state.region_marks);
        let from = if keep_all { state.weak_kept } else { 0 };
        for &t in &state.weak_tables[from..] {
            let data::Object::HashTable(table) = &self.objects[t] else {
                unreachable!("weak table index");
            };
            #[cfg(test)]
            { state.weak_visits += 1; }
            for (_, key, value) in table.entries() {
                let (k, v) = (self.survives(symbols, regions, key), self.survives(symbols, regions, value));
                if keep_all || table.weakness.keep(k, v) {
                    if !k {
                        pending.push(key);
                    }
                    if !v {
                        pending.push(value);
                    }
                }
            }
        }
        if keep_all {
            state.weak_kept = state.weak_tables.len();
        }
        // Traced once: what an entry pushes stays marked.
        state.untraced_lists.retain(|&list| {
            let live = self.survives(symbols, regions, Value(list));
            if live && let Some(entry) = lists.get_mut(&list) {
                #[cfg(test)]
                { state.last_mark.1 += 1; }
                entry.visit_values(&mut |v| if !self.survives(symbols, regions, *v) { pending.push(*v) });
            }
            !live
        });
        !pending.is_empty()
    }
    /// Allocated heap accounting; unreachable objects are included until collection.
    /// After collection these are retained/live counts, excluding compiler and image.
    pub fn heap_stats(&self) -> HeapStats {
        let mut s = self.gc_stats;
        s.allocations += (self.heap_objects() - self.gc_retained) as u64;
        s.live_objects = self.heap_objects();
        s.live_bytes = self.table_bytes()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.len() + c.metadata.len()) * 8)
                .sum::<usize>();
        // Native nodes may be retained by both tables and suspended cursors.
        // Keep this accounting separate from the moving arena object counts.
        let mut native_nodes = HashSet::default();
        s.live_bytes += self
            .objects
            .iter()
            .map(|o| match o {
                data::Object::Float(_) => 0,
                data::Object::Bignum(n) => size_of::<num_bigint::BigInt>()
                    + n.bits().div_ceil(64) as usize * size_of::<u64>(),
                data::Object::Window(w) => w.storage_bytes(),
                data::Object::Frame(f) => f.storage_bytes(),
                data::Object::Terminal(t) => t.storage_bytes(),
                data::Object::WindowConfiguration(c) => c.storage_bytes(),
                data::Object::Sqlite(h) => h.storage_bytes(),
                data::Object::Treesit(t) => t.storage_bytes(),
                data::Object::Finalizer(_) | data::Object::Thread | data::Object::Free => 0,
                data::Object::Process(p) => p.storage_bytes(),
                data::Object::String(v) | data::Object::RawString(v) => v.storage_bytes(false),
                data::Object::Bytes(v) => v.len(),
                data::Object::BoolVector(v) => size_of::<data::BoolVector>() + v.bytes.len(),
                data::Object::ByteBuffer(b) => {
                    size_of::<bytes::ByteBuffer>() + b.storage_bytes(false)
                }
                data::Object::Marker(_) => size_of::<annotations::Marker>(),
                data::Object::Overlay(o) => {
                    size_of::<annotations::Overlay>()
                        + o.properties.len() * size_of::<(Value, Value)>()
                }
                data::Object::Vector(v) | data::Object::Record(v) => v.len() * std::mem::size_of::<Value>(),
                data::Object::CharTable(t) => t.storage_bytes_seen(false, &mut native_nodes),
                data::Object::SubCharTable(node) => char_table::CharTableNode::storage_bytes(node, false, &mut native_nodes),
                data::Object::MapKeymapCursor(c) => c.storage_bytes(false, &mut native_nodes),
                data::Object::Buffer(t) => {
                    size_of::<annotations::AnnotatedBuffer>() + t.storage_bytes(false)
                }
                data::Object::SyntaxTable(t) => {
                    size_of_val(t)
                        + t.len() * size_of::<(u32, romacs_regex::SyntaxClass)>()
                }
                data::Object::HashTable(t) => t.storage_bytes(false),
                data::Object::Obarray(t) => t.storage_bytes(false),
            })
            .sum::<usize>();
        s.capacity_bytes = self.conses.capacity_bytes()
            + self.cells.capacity_bytes()
            + self.closures.capacity_bytes()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.capacity() + c.metadata.capacity()) * 8)
                .sum::<usize>();
        native_nodes.clear();
        s.capacity_bytes += self.objects.capacity_bytes()
            + self
                .objects
                .iter()
                .map(|o| match o {
                    data::Object::Float(_) => 0,
                    data::Object::Bignum(n) => size_of::<num_bigint::BigInt>()
                        + n.bits().div_ceil(64) as usize * size_of::<u64>(),
                    data::Object::Window(w) => w.storage_bytes(),
                    data::Object::Frame(f) => f.storage_bytes(),
                    data::Object::Terminal(t) => t.storage_bytes(),
                    data::Object::WindowConfiguration(c) => c.storage_bytes(),
                    data::Object::Sqlite(h) => h.storage_bytes(),
                    data::Object::Treesit(t) => t.storage_bytes(),
                    data::Object::Finalizer(_) | data::Object::Thread | data::Object::Free => 0,
                    data::Object::Process(p) => p.storage_bytes(),
                    data::Object::String(v) | data::Object::RawString(v) => v.storage_bytes(true),
                    data::Object::Bytes(v) => v.capacity(),
                    data::Object::BoolVector(v) => size_of::<data::BoolVector>() + v.bytes.capacity(),
                    data::Object::ByteBuffer(b) => {
                        size_of::<bytes::ByteBuffer>() + b.storage_bytes(true)
                    }
                    data::Object::Marker(_) => size_of::<annotations::Marker>(),
                    data::Object::Overlay(o) => {
                        size_of::<annotations::Overlay>()
                            + o.properties.capacity() * size_of::<(Value, Value)>()
                    }
                    data::Object::Vector(v) | data::Object::Record(v) => v.capacity() * std::mem::size_of::<Value>(),
                    data::Object::CharTable(t) => t.storage_bytes_seen(true, &mut native_nodes),
                    data::Object::SubCharTable(node) => char_table::CharTableNode::storage_bytes(node, true, &mut native_nodes),
                    data::Object::MapKeymapCursor(c) => c.storage_bytes(true, &mut native_nodes),
                    data::Object::Buffer(t) => {
                        size_of::<annotations::AnnotatedBuffer>() + t.storage_bytes(true)
                    }
                    data::Object::SyntaxTable(t) => {
                        size_of_val(t)
                            + t.len() * size_of::<(u32, romacs_regex::SyntaxClass)>()
                    }
                    data::Object::HashTable(t) => t.storage_bytes(true),
                    data::Object::Obarray(t) => t.storage_bytes(true),
                })
                .sum::<usize>();
        s
    }
    /// Whether this collection compacts the tables, moving their entries
    /// and rewriting every reference: when asked (`collect_compacting') and
    /// always under the stress and moving modes. Nothing else moves the
    /// heap: a compaction is a long stall that no key press can end, and a
    /// sweep gives back a dead tail without one.
    fn compacts(&mut self) -> bool {
        let compacts = self.will_compact();
        self.allocation.compact = false;
        compacts
    }
    /// Whether the next collection compacts (`compacts'), asking nothing:
    /// a mark twin's check names entries by where they are (`twin.rs').
    pub(crate) fn will_compact(&self) -> bool {
        self.allocation.compact || (self.gc_stress.every > 0 && !self.gc_stress.inplace) || self.gc_stress.moving
    }
    /// A collection stopped: the tables hold no marks.
    pub(crate) fn clear_marks(&mut self) {
        self.conses.clear_marks();
        self.closures.clear_marks();
        self.cells.clear_marks();
        self.objects.clear_marks();
    }
    /// Every table as a completed collection leaves it, its entries old and
    /// its log empty, without collecting: what a world image holds, since to
    /// the process that restores it the image is the old generation.
    pub(crate) fn settle_generations(&mut self) {
        // What becomes old here was not met by a minor, and every cache's
        // target is old.
        self.shared_holders = None;
        self.caches_filled.clear();
        self.names_added.clear();
        self.conditions.settle();
        // Everything charged since the last collection is old now.
        self.payload_settled = self.gc_payload_bytes;
        self.conses.settle_old();
        self.closures.settle_old();
        self.cells.settle_old();
        self.objects.settle_old();
        self.functions.settle();
        self.slots.settle();
        self.allocation.since = crate::epoch::commit_point();
    }
    /// Whether every table is as a completed collection leaves it: its
    /// entries old and clean, its log empty.
    #[cfg(test)]
    pub(crate) fn generations_settled(&self) -> bool {
        self.conses.settled() && self.closures.settled() && self.cells.settled() && self.objects.settled() && self.functions.settled() && self.slots.settled()
    }
    /// Under verify, after a collection: every entry in use is exactly old
    /// and no log holds anything, or the next collection would start from
    /// generations this one did not leave; and no call cache names what it
    /// freed.
    fn check_settled(&self) {
        for (table, settled) in [("conses", self.conses.settled()), ("closures", self.closures.settled()), ("cells", self.cells.settled()), ("objects", self.objects.settled())] {
            assert!(settled, "a collection left {table} with a young or dirty entry, or a log");
        }
        self.check_caches();
    }
    /// Reset the slots of the obarray symbols LIVE leaves out and free them.
    fn reclaim_obarray_symbols(&mut self, live: &HashSet<usize>) {
        let dead: Vec<usize> = self.obarray_symbol_names.keys().copied().filter(|slot| !live.contains(slot)).collect();
        for slot in dead {
            let version = self.slots[slot].version.checked_add(1).expect("symbol version exhausted");
            self.slots[slot] = Slot::new();
            self.slots[slot].version = version;
            self.symbol_names[slot].clear();
            self.symbol_names[slot].shrink_to_fit();
            if let Some(redirect) = self.variable_redirects.get_mut(slot) { *redirect = None; }
            self.macros.remove(&slot);
            self.conditions.entries.remove(&conditions::SymbolId::new(Value::reference(slot + 1, 7)).unwrap());
            if let Some(id) = self.generated_slots.remove(&slot) { self.generated_symbols.remove(&id); }
            self.obarray_free_slots.push(slot);
        }
        self.obarray_symbol_names.retain(|slot, _| live.contains(slot));
    }
    /// Release grossly oversized tables and free lists, but retain modest
    /// slack for allocation.
    fn shrink_tables(&mut self) {
        let budget = self.collection_budget().1 as usize;
        self.objects.shrink(1024, budget);
        self.conses.shrink(16384, budget);
        self.cells.shrink(1024, budget);
        self.closures.shrink(1024, budget);
    }
    /// Keep a finished collection's record, `elapsed_ns` long if a clock timed
    /// it, IDLE as in `collect_until_key`. One look at the host and one at the
    /// command variable, per collection: a key pays nothing.
    pub(super) fn note_collection(&mut self, elapsed_ns: Option<u64>, idle: bool, minor: bool, late: f64) {
        let explicit = std::mem::take(&mut self.collection_log.explicit);
        if self.collection_log.records.len() >= COLLECTION_RECORDS {
            self.collection_log.dropped += 1;
            return;
        }
        let command = if self.idle_since.is_none() && !idle { self.command_name() } else { None };
        let redisplay = command.is_none() && self.idle_since.is_some() && self.host_input.is_none();
        let key = command.is_none() && self.host_key_pending();
        let us = (elapsed_ns.unwrap_or(0) / 1_000).min(u32::MAX as u64) as u32;
        let at = elapsed_ns
            .and_then(|_| crate::host::clock().duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs_f64());
        let (objects, bytes) = (self.private_objects(), self.live_bytes_estimate());
        self.collection_log.records.push(CollectionRecord { idle, explicit, minor, us, command, key, redisplay, at, objects, bytes, late });
    }
    /// What the tables hold at their entries' own sizes.
    fn table_bytes(&self) -> usize {
        self.conses.live() * std::mem::size_of::<(Value, Value)>()
            + self.cells.live() * std::mem::size_of::<Value>()
            + self.closures.live() * std::mem::size_of::<Closure>()
            + self.objects.live() * std::mem::size_of::<data::Object>()
    }
    /// The live heap's size just after a collection, without a walk: the
    /// tables, and the strings and vectors the mark counted. It leaves out what
    /// only `heap_stats`, which walks, sees: buffers' text, hash tables, windows
    /// and the rest; and frozen regions.
    fn live_bytes_estimate(&self) -> usize {
        self.table_bytes() + self.gc_census.string_bytes + self.gc_census.vector_slots * std::mem::size_of::<Value>()
    }
    /// The running command's name, or the last one's in the command loop's
    /// tail (`romacs--command-loop-top' has cleared the running one).
    fn command_name(&mut self) -> Option<String> {
        let mut bound = |name: &str| {
            let slot = self.names.get(name).copied()?;
            // Through the variable: `real-last-command' is keyboard-local.
            self.variable_value(slot).ok().filter(|&value| value != Value::NIL && value != Value::UNBOUND)
        };
        let binding = bound("real-this-command").or_else(|| bound("real-last-command"))?;
        Some(binding.index(7).ok().and_then(|i| self.symbol_names.get(i.saturating_sub(1)).cloned()).unwrap_or_else(|| "anonymous".into()))
    }
    /// The records since the last call, emptied: (DROPPED RECORD...), each
    /// RECORD (KIND MS MET COMMAND T OBJECTS BYTES LATE), KIND `minor`, `idle`,
    /// `explicit` or `automatic`, MS a float, the pause in milliseconds to the microsecond,
    /// MET `command`, `key`, `redisplay` or nil, COMMAND a string or nil,
    /// T when it ended as `float-time` reads, or nil if nothing timed it,
    /// OBJECTS and BYTES what it left live (BYTES an estimate that does not walk),
    /// LATE a float, how far past its trigger it began (`CollectionRecord::late`).
    pub(crate) fn take_collections(&mut self) -> Value {
        let CollectionLog { records, dropped, .. } = std::mem::take(&mut self.collection_log);
        let mut result = Value::NIL;
        for record in records.into_iter().rev() {
            let kind = if record.minor { "minor" } else if record.idle { "idle" } else if record.explicit { "explicit" } else { "automatic" };
            let met = match (&record.command, record.key, record.redisplay) {
                (Some(_), ..) => self.intern("command"),
                (None, true, _) => self.intern("key"),
                (None, false, true) => self.intern("redisplay"),
                (None, false, false) => Value::NIL,
            };
            let command = record.command.map_or(Value::NIL, |name| self.make_lisp_string(name.into_bytes(), true));
            let kind = self.intern(kind);
            let ms = self.allocate_object(data::Object::Float(record.us as f64 / 1000.0));
            let at = record.at.map_or(Value::NIL, |at| self.allocate_object(data::Object::Float(at)));
            let size = |n: usize| Value::integer(i64::try_from(n).expect("a heap's size fits i64")).expect("a heap's size fits fixnum");
            let late = self.allocate_object(data::Object::Float(record.late));
            let mut row = self.keymap_cons(late, Value::NIL);
            row = self.keymap_cons(size(record.bytes), row);
            row = self.keymap_cons(size(record.objects), row);
            row = self.keymap_cons(at, row);
            row = self.keymap_cons(command, row);
            row = self.keymap_cons(met, row);
            row = self.keymap_cons(ms, row);
            row = self.keymap_cons(kind, row);
            result = self.keymap_cons(row, result);
        }
        self.keymap_cons(Value::integer(dropped as i64).expect("a count fits fixnum"), result)
    }
    /// Whether the host has a key press queued (`HostInput::key_pending').
    fn host_key_pending(&mut self) -> bool {
        self.host_input.as_mut().is_some_and(|host| host.key_pending())
    }
    /// Collect at a strand wait point. All externally retained Values must be in
    /// `roots`; compaction rewrites them. Never share private Values across VMs.
    /// Runtime register, global, binding and inline-cache roots are added here.
    /// With UNTIL_KEY, a key press queued while it marks ends it: nothing is
    /// collected, and the heap is as it was (marking writes only caches and
    /// local marks, and an undo list's cut waits for the end of the mark).
    /// Whether it collected.
    pub(super) fn collect_marked(&mut self, roots: &mut [Value], until_key: bool) -> bool {
        // The one funnel: no collection while a hold is open or an engine runs
        // unsuspended, whatever reached here. Raw code cannot suspend its own
        // engine (only the engine's lends do), so it cannot make this pass.
        debug_assert!(self.suspended.len() <= self.running_engines, "suspended engines outnumber running ones");
        if !self.may_collect(0) {
            return false;
        }
        // Every engine is suspended: trace each file only to its engine's top.
        self.clear_dead_tails();
        let mut lap = Lap::start();
        let before = self.heap_objects();
        self.clear_collection_caches();
        // Membership by slot, tested for every symbol and slot visited below.
        let in_obarray = self.slot_set(self.obarray_symbol_names.keys());
        lap.lap(&mut self.gc_times, Phase::Setup);
        let n = gc_mark::threads(self.gc_stress.threads);
        let Some(mut marked) = self.mark_heap(roots, &in_obarray, MarkFor::Collection { until_key }, Vec::new(), n, &mut lap) else { return false };
        // From here the collection changes the heap: the last point a key
        // press ends it.
        if until_key && self.host_key_pending() { return false; }
        if self.gc_stress.verify && n > 1 {
            marked = self.verify_mark(marked, n, roots, &in_obarray);
        }
        // Before the reference walk, which follows the lists this leaves.
        self.apply_undo_edits(std::mem::take(&mut marked.state.undo.edits));
        if self.gc_stress.verify {
            self.check_reachable(roots, &marked.code_marks, &marked.state.region_marks, &marked.state.obarray_symbols_live.reached);
        }
        let Marked { marking, own, doomed, code_marks, census, payload: _, state } = marked;
        let MarkState { mut native_nodes, reading: _, weak_tables, weak_kept: _, live_obarrays, obarray_symbols_live, region_marks, untraced_lists, undo_owners: _, undo: _, undo_strong: _, mut shared_seen, clean_holders: _, #[cfg(test)] weak_visits: _, #[cfg(test)] last_mark } = state;
        #[cfg(test)]
        { self.gc_stress.last_mark = last_mark; }
        lap.lap(&mut self.gc_times, Phase::Mark);
        self.gc_census = census;
        self.settle_cell_pairs();
        self.gc_times.entries = [self.conses.len(), self.closures.len(), self.cells.len(), self.objects.len()];
        self.doom(&doomed);
        self.gc_stats.allocations += before.saturating_sub(self.gc_retained) as u64;
        let pads = self.gc_stress.take_pads();
        self.drop_weak(&weak_tables, untraced_lists, &obarray_symbols_live, &region_marks);
        // Both taken here, by the collection they were planted for.
        #[cfg(test)]
        let (before_scan, before_sweep) = (std::mem::take(&mut self.gc_stress.faults.allocate_before_scan), std::mem::take(&mut self.gc_stress.faults.allocate_before_sweep));
        #[cfg(test)]
        if before_scan {
            let _ = self.alloc_cons(Value::NIL, Value::NIL);
        }
        lap.lap(&mut self.gc_times, Phase::Weak);
        self.retire_code(&code_marks);
        lap.lap(&mut self.gc_times, Phase::Retire);
        // The code this collection traced, for the next one's pacing.
        self.gc_code = code_marks.iter().zip(&self.functions).filter(|&(&live, _)| live).map(|(_, f)| f.code.len()).sum::<usize>()
            + self.execution.code_len()
            + self.suspended.iter().map(threaded::Suspended::code_len).sum::<usize>();
        // Only a collection with no request running retires live request
        // code; one inside an engine must not postpone the next of those.
        if self.stack.is_empty() && self.suspended_stacks.is_empty() {
            self.reset_code_count();
        }
        // Unmarked regions go now: no surviving value names one, and no weak
        // entry into one is left (swept above), so their ids can be reused.
        self.sweep_regions(&region_marks);
        for &i in &live_obarrays {
            if let data::Object::Obarray(t) = &mut self.objects.collector_entries_mut()[i] {
                t.reclaim_removed(&obarray_symbols_live.reached);
            }
        }
        self.empty_dead_caches(&obarray_symbols_live, &region_marks, false);
        let compacting = self.compacts();
        lap.lap(&mut self.gc_times, Phase::Census);
        if !compacting {
            self.reclaim_obarray_symbols(&obarray_symbols_live.reached);
            lap.lap(&mut self.gc_times, Phase::Obarray);
            #[cfg(test)]
            if before_sweep {
                let _ = self.alloc_cons(Value::NIL, Value::NIL);
            }
            self.sweep_in_place(&marking, own);
            lap.lap(&mut self.gc_times, Phase::Sweep);
            self.shrink_tables();
            lap.lap(&mut self.gc_times, Phase::Shrink);
            self.settle_collection(before);
            // Every live holder was met, the image's remembered ones as
            // roots, and none moved: the next minor's reads, in index order,
            // so it asks no object its kind.
            shared_seen.sort_unstable();
            self.shared_holders = Some(shared_seen);
            lap.lap(&mut self.gc_times, Phase::Settle);
            if self.gc_stress.verify { self.check_settled(); }
            return true;
        }
        // The dead with side effects end before their entries move: markers
        // leave their buffers (a sweep in place finds them itself).
        let dead = marking.dead_objects(self);
        self.unanchor(&dead);
        // Compaction: every live entry moves down, every reference is
        // rewritten, and no entry is left free. Past here a panic leaves the
        // references half rewritten, which nothing can undo.
        // Stress fillers go only where live objects would move past them,
        // and the image's entries never move (`Table::compaction_map`).
        let tables = [
            self.conses.compaction_map(pads),
            self.closures.compaction_map(pads),
            self.cells.compaction_map(pads),
            self.objects.compaction_map(pads),
        ];
        let fill: Vec<usize> = tables.iter().map(|table| table.1).collect();
        let maps: Vec<Vec<usize>> = tables.into_iter().map(|table| table.0).collect();
        let rewrite = |v: &mut Value| {
            let tag = v.tag();
            let arena = match tag {
                // Same omission as the mark phase, same cause: tag 3..=5 was
                // guarded against image values but tag 0 was not. A mapped
                // string never moves, so rewriting its reference is wrong as
                // well as out of bounds.
                0 if *v != Value::NIL && !Self::image_value(*v) => 3,
                3..=5 if !Self::image_value(*v) => (tag - 3) as usize,
                _ => return,
            };
            let offset = usize::from(tag == 0);
            let i = maps[arena][v.payload() as usize - offset];
            assert_ne!(i, usize::MAX, "rewriting an unmarked root");
            *v = Value::reference(i + offset, tag);
        };
        self.conses.collector_compact(&maps[0], |p| {
            rewrite(&mut p.0);
            rewrite(&mut p.1);
        });
        self.closures.collector_compact(&maps[1], |c| {
            for v in c.captures.iter_mut().chain(c.metadata.iter_mut()) {
                rewrite(v);
            }
            rewrite(&mut c.document);
        });
        self.cell_pairs = std::mem::take(&mut self.cell_pairs).into_iter().filter_map(|(i, mut pair)| {
            if !self.cells.marked(i) { return None; }
            let mut cell = Value::reference(i, 5);
            rewrite(&mut cell);
            rewrite(&mut pair);
            Some((cell.payload() as usize, pair))
        }).collect();
        self.cells.collector_compact(&maps[2], |v| rewrite(v));
        // The traced register slots (`register_roots`); above the tops is NIL.
        let tops = self.register_tops();
        let registers = self.suspended_stacks.iter_mut().chain(std::iter::once(&mut self.stack)).zip(tops);
        for v in roots
            .iter_mut()
            .chain(registers.flat_map(|(file, top)| file[..top].iter_mut()))
            .chain(self.external_roots.iter_mut())
            .chain(self.terminal_bindings.iter_mut().flat_map(|binding| std::iter::once(&mut binding.buffer).chain(binding.away.iter_mut()).chain(binding.unremapped_maps.iter_mut())))
        {
            rewrite(v);
        }
        if let Some(escape) = &mut self.propagated_escape {
            escape.visit_values(&mut |v| rewrite(v));
        }
        for v in &mut self.thrown_no_catch {
            rewrite(v);
        }
        for handler in &mut self.signal_handlers {
            handler.visit_values(&mut |v| rewrite(v));
        }
        self.backtrace.visit_values(&mut |v| rewrite(v));
        self.profiler.visit_values(&mut |v| rewrite(v));
        for (slot, name) in &mut self.obarray_symbol_names {
            if obarray_symbols_live.contains(*slot) { rewrite(name); }
        }
        self.reclaim_obarray_symbols(&obarray_symbols_live.reached);
        for name in self.permanent_symbol_names.values_mut() { rewrite(name); }
        for s in self.slots.collector_iter_mut() {
            rewrite(&mut s.value);
            rewrite(&mut s.function);
            rewrite(&mut s.plist);
        }
        self.forwarded.visit_values(&mut |v| rewrite(v));
        for v in &mut self.standard_case_tables { rewrite(v); }
        rewrite(&mut self.command_point.1);
        for v in self.finalizers.iter_mut().chain(self.doomed_finalizers.iter_mut()) { rewrite(v); }
        self.charsets.rewrite(&rewrite);
        // HOST(dynamic:saved): collector: marks the saved values
        for (_, v, buffer, _) in &mut self.dynamic {
            rewrite(v);
            rewrite(buffer);
        }
        for c in &mut self.caches {
            rewrite(&mut c.target);
        }
        for (known, _) in self.mode_line_forms.values_mut() { rewrite(known); }
        // Keyed by the list's bits, which move with it.
        self.lambda_lists = std::mem::take(&mut self.lambda_lists).into_iter().map(|(list, mut entry)| {
            let mut list = Value(list);
            rewrite(&mut list);
            entry.visit_values(&mut |v| rewrite(v));
            (list.0, entry)
        }).collect();
        // Forward each physical native slot once, even when several live
        // cursors retain the same subtree (possibly detached from its table).
        native_nodes.clear();
        // Keyed by object index, as `cell_pairs' by cell: what died leaves
        // it, the rest follow their objects.
        if let Some(referrers) = &mut self.referrers {
            let objects = &self.objects;
            referrers.forward(|i| {
                if !objects.marked(i) { return None; }
                let mut object = Value::reference(i + 1, 0);
                rewrite(&mut object);
                object.index_if(0)?.checked_sub(1)
            });
        }
        let whole = properties::Reading { since: self.allocation.since, whole: true };
        self.objects.collector_compact(&maps[3], |object| {
                match object {
                    data::Object::Vector(values) | data::Object::Record(values) => {
                        for v in values {
                            rewrite(v);
                        }
                    }
                    data::Object::Window(w) => w.visit_values(&mut |v| rewrite(v)),
                    data::Object::Frame(f) => f.visit_values(&mut |v| rewrite(v)),
                    data::Object::Terminal(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::WindowConfiguration(c) => c.visit_values(&mut |v| rewrite(v)),
                    data::Object::Sqlite(_) | data::Object::Thread | data::Object::Free => {}
                    data::Object::Treesit(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::Finalizer(function) => rewrite(function),
                    data::Object::Process(p) => p.visit_values(&mut |v| rewrite(v)),
                    data::Object::Buffer(t) => t.visit_values(whole, &mut |v| rewrite(v)),
                    data::Object::ByteBuffer(t) => t.visit_values(whole, &mut |v| rewrite(v)),
                    data::Object::Marker(m) => m.visit_values(&mut |v| rewrite(v)),
                    data::Object::Overlay(o) => o.visit_values(&mut |v| rewrite(v)),
                    data::Object::String(s) | data::Object::RawString(s) => {
                        s.properties.visit_values(&mut |v| rewrite(v))
                    }
                    data::Object::HashTable(t) => t.forward(&mut |v| rewrite(v)),
                    data::Object::Obarray(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::CharTable(t) => t.collector_visit_values(&mut native_nodes, 0, &mut |v| rewrite(v)),
                    data::Object::SubCharTable(node) => char_table::collector_visit_node(node, &mut native_nodes, 0, &mut |v| rewrite(v)),
                    data::Object::MapKeymapCursor(c) => c.collector_visit_values(&mut native_nodes, 0, &mut |v| rewrite(v)),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
        });
        for f in self.functions.collector_iter_mut() {
            // Function Arcs are private to this VM; no executable shares IR.
            let f = Arc::get_mut(f).expect("source IR must be exclusively owned at a wait point");
            for op in &mut f.code {
                op.visit_values(&mut |v| rewrite(v));
            }
            // Retired functions dropped their bodies before forwarding, so
            // every body still present was marked through its function.
            if let Some(lazy) = &mut f.lazy {
                rewrite(&mut Arc::get_mut(lazy).expect("lazy body is private at a wait point").body);
            }
            if let Some(body) = &mut f.pending {
                body.values.iter_mut().for_each(|v| rewrite(v));
            }
        }
        for program in std::iter::once(&mut self.execution).chain(self.spare_programs.iter_mut()) {
            program.visit_values(&mut |v| rewrite(v));
        }
        for engine in &mut self.suspended {
            engine.visit_values(&mut |v| rewrite(v));
        }
        self.conditions.visit_values(&mut |v| rewrite(v));
        // Stress fillers: unreachable, so the next collection drops them.
        self.conses.collector_prepend(fill[0], || (Value::NIL, Value::NIL));
        self.closures.collector_prepend(fill[1], || Closure { function: 0, captures: vec![], metadata: vec![], document: Value::NIL });
        self.cells.collector_prepend(fill[2], || Value::NIL);
        self.objects.collector_prepend(fill[3], || data::Object::String(Box::new(data::LispString::from_bytes(vec![], true))));
        lap.lap(&mut self.gc_times, Phase::Rewrite);
        self.shrink_tables();
        lap.lap(&mut self.gc_times, Phase::Shrink);
        self.settle_collection(before);
        lap.lap(&mut self.gc_times, Phase::Settle);
        if self.gc_stress.verify { self.check_settled(); }
        true
    }
    /// A lexical pair's cell takes its value, which marking only read.
    fn settle_cell_pairs(&mut self) {
        let pairs: Vec<(usize, Value)> = self.cell_pairs.iter().filter(|&(&i, _)| self.cells.marked(i))
            .map(|(&i, &pair)| (i, self.cons_pair(pair).expect("live lexical pair").1)).collect();
        for (i, value) in pairs {
            self.cells.collector_entries_mut()[i] = value;
            // An edge into the image no store recorded.
            self.cells.remember(i);
        }
    }
    /// The finalizers a mark DOOMED leave the live list for the doomed one.
    fn doom(&mut self, doomed: &[Value]) {
        if !doomed.is_empty() {
            let gone: HashSet<u64> = doomed.iter().map(|f| f.0).collect();
            self.finalizers.retain(|f| !gone.contains(&f.0));
            self.doomed_finalizers.extend(doomed.iter().copied());
        }
    }
    /// What a completed mark drops before anything is freed: the weak
    /// references to what it did not reach, SYMBOLS and REGIONS being the
    /// obarray symbols and frozen regions it kept.
    fn drop_weak(&mut self, weak_tables: &[usize], untraced_lists: Vec<u64>, symbols: &KeptSymbols, regions: &[bool]) {
        // A throw's no-catch mark is weak: an error object nothing else
        // reaches is carried by no escape, so its mark goes.
        let thrown = std::mem::take(&mut self.thrown_no_catch);
        self.thrown_no_catch = thrown
            .into_iter()
            .filter(|&object| self.survives(symbols, regions, object))
            .collect();
        // GNU `sweep_weak_table (h, true)`: drop entries the rule rejects
        // before forwarding, which asserts that every rewritten Value is live.
        for &t in weak_tables {
            let dead: Vec<_> = match &self.objects[t] {
                data::Object::HashTable(table) => table
                    .entries()
                    .filter(|&(_, key, value)| {
                        !(self.survives(symbols, regions, key)
                          && self.survives(symbols, regions, value))
                    })
                    .map(|(slot, _, _)| slot)
                    .collect(),
                _ => unreachable!("weak table index"),
            };
            if let data::Object::HashTable(table) = &mut self.objects.collector_entries_mut()[t] {
                for slot in dead {
                    table.sweep(slot);
                }
                table.compact_local_bindings();
            }
        }
        // A cache entry goes when what it is keyed by dies: a list's
        // compiled closure with the list (`weak_round`), a segment's form
        // with the value it was converted from, which the slot can never
        // hold again.
        for list in untraced_lists { self.lambda_lists.remove(&list); }
        let dead: Vec<usize> = self.mode_line_forms.iter()
            .filter(|&(_, &(known, _))| !self.survives(symbols, regions, known))
            .map(|(&slot, _)| slot).collect();
        for slot in dead { self.mode_line_forms.remove(&slot); }
    }
    /// The dead markers of DEAD leave their buffers.
    fn unanchor(&mut self, dead: &[u32]) {
        for &i in dead {
            if let data::Object::Marker(m) = &self.objects[i as usize] && let Some(anchor) = m.anchor {
                let buffer = m.buffer;
                let Ok(index) = buffer.index(0) else { continue };
                match self.objects.collector_entries_mut().get_mut(index.saturating_sub(1)) {
                    Some(data::Object::Buffer(b)) => { b.anchors.remove(anchor); }
                    Some(data::Object::ByteBuffer(b)) => { b.anchors.remove(anchor); }
                    _ => {}
                }
            }
        }
    }
    /// A call cache whose target died is stale: its symbol's function
    /// changed since it was filled, or the symbol itself, in an obarray of
    /// its own, is dead; else the cell would hold the target live. It is
    /// emptied while the marks are whole, so that no cache names a freed
    /// entry once this collection is over: the next may cut that entry off
    /// its table, or compact and rewrite every reference it keeps.
    /// A minor (FILLED_ONLY) asks only the caches filled since the last
    /// collection: that one left every target alive, so old to it.
    fn empty_dead_caches(&mut self, symbols: &KeptSymbols, regions: &[bool], filled_only: bool) {
        let empty_if_dead = |vm: &mut Self, i: usize| {
            if i < vm.caches.len() && !vm.survives(symbols, regions, vm.caches[i].target) {
                vm.caches[i] = Cache { version: 0, target: Value::UNBOUND };
            }
        };
        if filled_only && self.caches_filled.len() < CACHES_FILLED {
            for k in 0..self.caches_filled.len() {
                let i = self.caches_filled[k] as usize;
                empty_if_dead(self, i);
            }
        } else {
            for i in 0..self.caches.len() { empty_if_dead(self, i); }
        }
        self.caches_filled.clear();
    }
    /// Under verify, after any collection: no call cache names a freed
    /// entry, which a fill no record saw would leave (`caches_filled`). An
    /// own-obarray symbol a full reclaimed is not asked after.
    fn check_caches(&self) {
        for (i, cache) in self.caches.iter().enumerate() {
            let target = cache.target;
            if Self::image_value(target) { continue; }
            let freed = match target.tag() {
                0 if target != Value::NIL => (target.payload() as usize).checked_sub(1).is_none_or(|n| n >= self.objects.len() || self.objects.is_free(n)),
                3 => target.payload() as usize >= self.conses.len() || self.conses.is_free(target.payload() as usize),
                4 => target.payload() as usize >= self.closures.len() || self.closures.is_free(target.payload() as usize),
                5 => target.payload() as usize >= self.cells.len() || self.cells.is_free(target.payload() as usize),
                _ => false,
            };
            assert!(!freed, "call cache {i} names a freed entry");
        }
    }
    /// Free in place what MARKING left unmarked: every survivor is left old
    /// and clean, and the logs empty.
    fn sweep_in_place(&mut self, marking: &gc_mark::Marking, mut own: gc_mark::Worker) {
        let cells = &self.cells;
        self.cell_pairs.retain(|&i, _| cells.marked(i));
        let verify = self.gc_stress.verify;
        let gc_mark::Swept { tables: swept, dead } = marking.sweep(self, &mut own);
        // The dead with side effects, still where they were, end here, on
        // this thread, before the books may cut a table's dead tail: markers
        // leave their buffers, the referrers index lets them go, and the
        // rest are dropped. (A panic in the sweep leaves them so, free and
        // anchored; the VM is used again only after `promote_all`.)
        self.unanchor(&dead);
        if let Some(referrers) = &mut self.referrers { referrers.forget(&dead, &self.objects); }
        for &i in &dead {
            self.objects.collector_entries_mut()[i as usize] = data::Object::Free;
        }
        self.conses.swept(swept[0], verify);
        self.closures.swept(swept[1], verify);
        self.cells.swept(swept[2], verify);
        self.objects.swept(swept[3], verify);
        marking.finish(own);
        #[cfg(test)]
        { self.gc_stress.buried += marking.buried.load(std::sync::atomic::Ordering::Acquire); }
    }
    /// A minor collection: what was made since the last collection and is
    /// no longer reached is freed, and nothing old is read but what may name
    /// the young. To its mark every old entry counts as reached
    /// (`Table::raw`), so the mark, the weak rounds and the sweep are a full
    /// collection's over the young alone. Read again, whatever their age: the entries stored into since
    /// the last collection; every buffer and char table, for the text
    /// properties and nodes written since, which no entry's colour records
    /// (`crate::epoch`); the symbol slots written since (`Slots`), those of
    /// obarrays of their own included; the functions changed since
    /// (`Functions`); every lexical pair; and every other root. Nothing a full reclaims beside
    /// table entries goes: no code is retired, no region, obarray symbol or
    /// local binding freed, and nothing moves (a table's dead tail is cut,
    /// as any sweep cuts it). Like a full it has spare programs forget their
    /// lowered code first. With a log or the functions' record lost, it
    /// promotes everything instead.
    /// Whether it collected.
    pub(crate) fn collect_minor(&mut self, roots: &[Value]) -> bool {
        if !self.may_collect(0) {
            return false;
        }
        if [self.conses.log_lost(), self.closures.log_lost(), self.cells.log_lost(), self.objects.log_lost(), self.functions.changed().is_none()].contains(&true) {
            self.promote_all();
        } else {
            self.minor(roots);
        }
        self.gc_stats.collections += 1;
        true
    }
    /// Promote all: every entry in use becomes old and clean, and every
    /// record of what changed since the last collection is emptied (the put
    /// logs, the functions changed, the slot cards, the epoch that text and
    /// char-table storage is stamped against). Nothing is freed, doomed or
    /// dropped. With no young entry left no old one names a young one, so
    /// it is sound wherever a collection may start, whatever was traced
    /// before: a minor's way to give up, the recovery of a collection a
    /// panic stopped (`restore_nesting`), and the generations' commit point
    /// for whoever needs one. A record a later slice adds is emptied here,
    /// or in what this calls.
    pub(crate) fn promote_all(&mut self) {
        self.settle_generations();
        self.settle_minor();
    }
    /// `collect_minor`, once it is known to run. One that a panic stops
    /// has made old entries young, and may have swept part of a table: the
    /// VM is used again only through `Vm::catching`, which promotes all.
    fn minor(&mut self, roots: &[Value]) {
        self.clear_dead_tails();
        let mut lap = Lap::start();
        self.clear_collection_caches();
        if self.gc_stress.verify {
            self.conses.check_cards("cons");
            self.closures.check_cards("closure");
            self.cells.check_cards("cell");
            self.objects.check_cards("object");
        }
        let mut again: Vec<Value> = roots.to_vec();
        again.extend(self.conses.read_again().into_iter().map(|i| Value::reference(i, 3)));
        again.extend(self.closures.read_again().into_iter().map(|i| Value::reference(i, 4)));
        again.extend(self.cells.read_again().into_iter().map(|i| Value::reference(i, 5)));
        // The old objects storing their own writes, as the last collection
        // met them all: a later one is young, so met if it lives, and one
        // that took the kind in place was stored into. Unknown (after a
        // compaction, a restore or a promote-all), every object is asked its
        // kind.
        let shared = |object: &data::Object| shares_storage(object);
        let holders = self.shared_holders.take().unwrap_or_else(|| self.objects.iter().enumerate()
            .filter(|&(_, object)| shares_storage(object)).map(|(i, _)| i as u32).collect());
        if self.gc_stress.verify {
            self.check_holders(&holders);
        }
        let mut objects = self.objects.read_again();
        let clean = self.objects.read_again_at(&holders, &shared);
        objects.extend(clean.iter().copied());
        // Old, and marked as the young are: what they hold is no growth.
        let old_payload: usize = objects.iter().map(|&i| data::payload_bytes(&self.objects[i])).sum();
        again.extend(objects.into_iter().map(|i| Value::reference(i + 1, 0)));
        for table in [self.conses.logged(), self.closures.logged(), self.cells.logged(), self.objects.logged()] {
            again.extend_from_slice(table);
        }
        // A lexical pair is named by the map alone, and one may have been
        // made since the last collection for a cell that is old and clean:
        // a minor reads them all, at a claim each. (An old cell's pair is
        // kept until a full finds the cell dead; a dead young cell's pair
        // is kept by this minor, which drops its mapping, and goes at the
        // next full.)
        again.extend(self.cell_pairs.values().copied());
        // No symbol is reclaimed: those of obarrays of their own are roots
        // like the rest, their names made since among the roots
        // (`names_added`), their slots by the slots' cards, and what was put
        // on any condition since by the conditions' log.
        again.extend(self.conditions.changed_values());
        #[cfg(test)]
        assert!(!std::mem::take(&mut self.gc_stress.faults.minor_panics), "a planted fault");
        let none = vec![false; self.slots.len()];
        // Setup is the old marked and what is read again gathered; Roots and
        // Mark as a full's; Weak, all that is dropped; then the sweep.
        lap.lap(&mut self.gc_times, Phase::Setup);
        let Some(marked) = self.mark_heap(&again, &none, MarkFor::Minor, clean.into_iter().map(|i| u32::try_from(i).expect("an object index")).collect(), 1, &mut lap) else { unreachable!("no key ends a minor") };
        lap.lap(&mut self.gc_times, Phase::Mark);
        if self.gc_stress.verify {
            self.check_minor(roots);
        }
        let Marked { marking, own, doomed, payload: marked_payload, state, .. } = marked;
        let MarkState { weak_tables, untraced_lists, obarray_symbols_live: symbols, region_marks: regions, mut shared_seen, .. } = state;
        self.settle_cell_pairs();
        self.doom(&doomed);
        self.drop_weak(&weak_tables, untraced_lists, &symbols, &regions);
        self.empty_dead_caches(&symbols, &regions, true);
        lap.lap(&mut self.gc_times, Phase::Weak);
        self.gc_times.entries = [self.conses.len(), self.closures.len(), self.cells.len(), self.objects.len()];
        self.sweep_in_place(&marking, own);
        // The next full is paced by what the old generation grew by: the
        // payload the young that survived hold now, beside what earlier
        // minors since the last full promoted. Neither what the dead were
        // charged nor how they changed size since enters it.
        self.gc_payload_bytes = self.payload_settled.saturating_add(marked_payload.saturating_sub(old_payload));
        lap.lap(&mut self.gc_times, Phase::Sweep);
        self.gc_times.marked = [self.conses.live(), self.closures.live(), self.cells.live(), self.objects.live()];
        if self.gc_stress.verify { self.check_settled(); }
        // Every live holder was met once: read again as old, or young and
        // reached. In index order, for the next minor's reads.
        shared_seen.sort_unstable();
        self.shared_holders = Some(shared_seen);
        self.settle_minor();
    }
    /// Under verify, at a minor's start: HOLDERS, what it reads for storage
    /// written past the entries, holds every old object in use that may
    /// hold such storage (`shares_storage`). One a list missed would keep
    /// a young value the minor then frees.
    fn check_holders(&self, holders: &[u32]) {
        for (i, object) in self.objects.iter().enumerate() {
            if shares_storage(object) && self.objects.old(i) && !self.objects.is_free(i) {
                assert!(u32::try_from(i).is_ok_and(|i| holders.binary_search(&i).is_ok()), "old holder {i} is not among a minor's holders");
            }
        }
    }
    /// A minor's books: the young start again from here, and no poll is
    /// pending. What the last full kept stays the base a full is paced
    /// from, so what this promoted counts toward the next one.
    fn settle_minor(&mut self) {
        // A forced collection is over (a full's takes its fillers with it).
        self.gc_stress.shift = false;
        self.payload_settled = self.gc_payload_bytes;
        self.names_added.clear();
        self.conditions.settle();
        self.functions.settle();
        self.slots.settle();
        self.allocation.since = crate::epoch::commit_point();
        self.allocation.collected_at = self.allocation.allocated;
        self.settle_poll();
    }
    /// A collection's books, BEFORE being the objects it found.
    fn settle_collection(&mut self, before: usize) {
        // A compaction moved them; a sweep in place gives its own after this.
        self.shared_holders = None;
        self.names_added.clear();
        self.conditions.settle();
        self.functions.settle();
        self.slots.settle();
        self.allocation.since = crate::epoch::commit_point();
        self.gc_retained = self.heap_objects();
        self.settle_payload();
        // The one place a job child's growth is measured, whatever asked for
        // the collection: past its budget it ends now, natively, and its
        // spawner replies for the job (VM2-ISOLATE-MEMORY).
        if self.job_over_budget() { crate::isolate::job_memory_exit(); }
        self.gc_stats.reclaimed += before.saturating_sub(self.gc_retained) as u64;
        self.gc_stats.collections += 1;
        self.gc_times.marked = [self.conses.live(), self.closures.live(), self.cells.live(), self.objects.live()];
    }
    /// What a collection drops before it marks, and a mark twin's parent
    /// before it forks (phoenix M1): the caches that name objects by value
    /// without rooting them, so none still names one the mark finds dead.
    pub(crate) fn clear_collection_caches(&mut self) {
        // Buffers move (`Places`).
        self.forwarded.collection_epoch = self.forwarded.collection_epoch.wrapping_add(1);
        self.resolved_buffer = None;
        // Patterns kept for a table name it by a Value this may move.
        self.clear_regexp_cache(true);
        // A view's rows are the Values of its table's entries.
        self.flat_tables.forget();
        // Finished nested runs' programs are caches: relowering what is used
        // again costs less than tracing, retiring and forwarding every copy.
        for program in &mut self.spare_programs {
            program.forget_code();
        }
    }
    /// A mark twin's mark (phoenix M2), in a process forked for it: the
    /// collection's own mark from ROOTS and everything the VM holds, on one
    /// thread, its finalizers doomed and its weak rounds run, every weak entry
    /// kept when KEEP_WEAK, stopped before a collection drops anything or
    /// sweeps. The colours stay in the tables (`Table::marked'); the rest is
    /// returned.
    pub(crate) fn mark_only(&mut self, roots: &[Value], keep_weak: bool) -> MarkedSet {
        let in_obarray = self.slot_set(self.obarray_symbol_names.keys());
        let marked = self.mark_heap(roots, &in_obarray, MarkFor::Only { keep_weak }, Vec::new(), 1, &mut Lap(None)).expect("no key ends it");
        let Marked { code_marks, state, .. } = marked;
        MarkedSet { code_marks, obarray_symbols_live: state.obarray_symbols_live.reached, region_marks: state.region_marks,
                    #[cfg(test)] weak_visits: state.weak_visits }
    }
    /// Mark from ROOTS and everything the VM holds, on N threads, for FOR: what
    /// it reached, or None when a key press ended it, its marks left for the
    /// next mark to clear. Writes nothing but colours and its own state.
    fn mark_heap(&mut self, roots: &[Value], in_obarray: &[bool], r#for: MarkFor, clean_holders: Vec<u32>, n: usize, lap: &mut Lap) -> Option<Marked> {
        let until_key = matches!(r#for, MarkFor::Collection { until_key: true });
        let keep_weak = matches!(r#for, MarkFor::Only { keep_weak: true });
        let keep = |v: &Value| traced(*v, in_obarray);
        self.undo_list_index();
        let undo_limits = self.undo_view();
        let mut pending = self.mark_roots(roots, in_obarray, matches!(r#for, MarkFor::Minor));
        // HOST(dynamic:saved): collector: forwards the saved values
        for &(slot, old, buffer, _) in &self.dynamic {
            // Both saved and active values stay roots even if Lisp switches
            // away from their buffer during the dynamic extent.
            pending.extend([old, self.slots[slot].value, buffer, Value::reference(slot + 1, 7)]);
        }
        let mut code_marks: Vec<std::sync::atomic::AtomicBool> = self.functions.iter().map(|_| false.into()).collect();
        let active = !self.stack.is_empty() || !self.suspended_stacks.is_empty();
        let minor_changed = matches!(r#for, MarkFor::Minor).then(|| self.minor_code(&mut code_marks));
        // A minor reads the functions changed since the last collection, and
        // no other code root.
        let mut code_pending: Vec<_> = match &minor_changed {
            Some(changed) => changed.clone(),
            None => {
                let mut pending: Vec<_> = self.request_code.live.iter().enumerate()
                    .filter_map(|(id, &live)| (live && (active || self.request_code.permanent[id])).then_some(id))
                    .collect();
                self.source_code_roots(&mut pending);
                pending
            }
        };
        // A minor reads what was put since the last collection (`minor`).
        if minor_changed.is_none() {
            for (id, entry) in &self.conditions.entries {
                let custom = id.value().index_if(7).and_then(|n| n.checked_sub(1))
                    .is_some_and(|slot| self.obarray_symbol_names.contains_key(&slot));
                if !custom {
                    for &(key, value) in &entry.properties { pending.extend([key, value]); }
                }
            }
        }
        // A minor's old include the image; any other mark reads of the image
        // only what may name what is not of it, and counts the rest.
        let image = (minor_changed.is_none()).then(|| self.image_roots(&mut pending, &mut code_pending, &mut code_marks));
        lap.lap(&mut self.gc_times, Phase::Roots);
        let untraced_lists = self.lambda_lists.keys().copied().collect();
        let reading = properties::Reading { since: self.allocation.since, whole: true };
        let undo_strong = keep_weak || matches!(r#for, MarkFor::Minor);
        let mut state = MarkState { region_marks: vec![false; self.regions.len()], untraced_lists, reading, undo_strong, clean_holders, ..MarkState::default() };
        state.region_marks[0] = true;
        if minor_changed.is_some() { self.minor_keeps(&mut state); }
        // GNU dooms finalizers once, after the roots and before the weak
        // tables' contents are marked.
        let mut finalizers_doomed = false;
        let mut doomed = Vec::new();
        // SAFETY: from here to the sweep the collection grows, shrinks,
        // sweeps and re-marks no table but through MARKING (`verify_mark'
        // hands its own on), and its scan and sweep check that first.
        let marking = unsafe { gc_mark::Marking::new(self, n, minor_changed.is_some()) };
        let mut own = gc_mark::Worker::default();
        // All a mark wants of the VM but to read it, while nothing else asks.
        let mut host = if until_key { self.host_input.take() } else { None };
        // Lent to the mark, which visits an entry through `&mut`, and back
        // however it ends.
        let mut lists = std::mem::take(&mut self.lambda_lists);
        let this = &*self;
        let written_slots = minor_changed.is_some().then(|| this.slots.written().collect());
        let view = this.mark_view(in_obarray, &code_marks, written_slots);
        let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| marking.crew(&view, || {
            marking.publish_roots(&view, std::mem::take(&mut pending), std::mem::take(&mut code_pending));
            // Here while the threads start on the rest: suspended engines'
            // lowered code.
            this.lowered_roots(minor_changed.as_deref(), &mut |v| if keep(&v) { pending.push(v) });
            marking.add(&mut own, &mut pending);
            loop {
                let drained = marking.round(&view, &mut own, KEY_POLL,
                    &mut |handed, pending| this.mark_serial(handed, in_obarray, pending, &mut state),
                    &mut || host.as_mut().is_some_and(|host| host.key_pending()));
                if !drained { return false; }
                // As garbage_collect: undo lists, then finalizers, then weak
                // tables. A buffer one of those reaches comes back here.
                if !state.undo_owners.is_empty() {
                    let owners = std::mem::take(&mut state.undo_owners);
                    if !this.undo_round(owners, &undo_limits, &marking, &mut pending, &mut state.undo,
                        &mut || host.as_mut().is_some_and(|host| host.key_pending())) { return false; }
                    marking.add(&mut own, &mut pending);
                    continue;
                }
                if !std::mem::replace(&mut finalizers_doomed, true) && this.doom_finalizers(&mut pending, &mut doomed) {
                    marking.add(&mut own, &mut pending);
                    continue;
                }
                if this.weak_round(&mut state, &mut lists, keep_weak, &mut pending) {
                    marking.add(&mut own, &mut pending);
                    continue;
                }
                return true;
            }
        })));
        // The host goes back however the mark ended: a session that lost it
        // would see no key at any later idle collection.
        if until_key { self.host_input = host; }
        self.lambda_lists = lists;
        let finished = finished.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if !finished { return None; }
        let mut census = marking.census(&own);
        // What it did not read of the image, the permanent objects, it counts
        // as they are: as the restore found them.
        if image == Some(true) {
            census.add(&self.world_image.census);
        }
        let code_marks = code_marks.into_iter().map(|mark| mark.into_inner()).collect();
        let payload = marking.marked_payload(&own);
        Some(Marked { marking, own, doomed, code_marks, census, payload, state })
    }
    /// The roots a mark starts from outside the heap's own entries, those it
    /// traces (`traced`): ROOTS and the external ones, suspended
    /// engines', the finalizers doomed, permanent symbols, terminal bindings,
    /// the escape in flight, signal handlers, the backtrace and the profiler,
    /// GNU's C globals, the case tables, the command point and the charsets.
    fn mark_roots(&mut self, roots: &[Value], in_obarray: &[bool], minor: bool) -> Vec<Value> {
        let keep = |v: &Value| traced(*v, in_obarray);
        let mut pending: Vec<Value> = roots.iter().copied().filter(keep).collect();
        pending.extend_from_slice(&self.external_roots);
        for engine in &mut self.suspended {
            engine.visit_state_values(&mut |v| if keep(v) { pending.push(*v) });
        }
        pending.extend(self.doomed_finalizers.iter().copied());
        // A minor's are the names made since the last collection, of
        // obarrays of their own too: the rest are old.
        if minor {
            pending.extend_from_slice(&self.names_added);
        } else {
            pending.extend(self.permanent_symbol_names.values().copied());
        }
        pending.extend(self.terminal_bindings.iter().map(|binding| binding.buffer));
        pending.extend(self.terminal_bindings.iter().flat_map(|binding| binding.away.iter().chain(&binding.unremapped_maps).copied()));
        if let Some(escape) = &mut self.propagated_escape {
            escape.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        }
        for handler in &mut self.signal_handlers {
            handler.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        }
        self.backtrace.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        self.profiler.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        // GNU's C globals and `buffer_defaults`: roots outside every value
        // cell (forwarded.rs). Per-buffer values are traced with their buffer.
        self.forwarded.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        pending.extend_from_slice(&self.standard_case_tables);
        pending.push(self.command_point.1);
        self.charsets.trace(&mut pending);
        pending
    }
    /// The functions a minor reads, those changed since the last collection:
    /// every other starts reached in `code_marks`, as the old entries do.
    fn minor_code(&self, code_marks: &mut [std::sync::atomic::AtomicBool]) -> Vec<usize> {
        let changed = self.functions.changed().expect("a minor with the record lost promotes instead").to_vec();
        for mark in code_marks.iter_mut() { *mark.get_mut() = true; }
        for &id in &changed {
            if let Some(mark) = code_marks.get_mut(id) { *mark.get_mut() = false; }
        }
        changed
    }
    /// A minor reclaims no region and no symbol: to its weak rounds every
    /// one is live, as each is to what it drops afterwards. Of char tables
    /// it reads the nodes written since the last collection.
    fn minor_keeps(&self, state: &mut MarkState) {
        state.reading.whole = false;
        state.region_marks.fill(true);
        state.obarray_symbols_live = KeptSymbols::all();
    }
    /// The lowered code a mark starts from: suspended engines', whole. A
    /// minor (CHANGED given) reads lowered code as it reads functions, that
    /// of the changed alone (`Suspended::minor_values`).
    fn lowered_roots(&self, changed: Option<&[usize]>, value: &mut impl FnMut(Value)) {
        for engine in &self.suspended {
            match changed {
                None => engine.code_values(value),
                Some(changed) => engine.minor_values(changed, value),
            }
        }
    }
    /// `ROMACS_GC_VERIFY': mark again, alone, from nothing, and require the
    /// threads' answer: every colour, function, symbol and region, the doomed
    /// and the figures. A mark misses what a race loses, and a sweep then
    /// frees what is reachable; nothing else would show it.
    /// The re-mark's state goes on, its colours being these: the tables were
    /// last taken by it (`RawTable::holds'). It sweeps on the threads' N, so
    /// verify keeps the threaded sweep and its graves.
    fn verify_mark(&mut self, threads: Marked, n: usize, roots: &[Value], in_obarray: &[bool]) -> Marked {
        fn sorted<T: Ord + Clone>(list: &[T]) -> Vec<T> { let mut list = list.to_vec(); list.sort_unstable(); list }
        let colors = [self.conses.colors(), self.closures.colors(), self.cells.colors(), self.objects.colors()];
        self.clear_marks();
        let mut lap = Lap(None);
        let alone = self.mark_heap(roots, in_obarray, MarkFor::Collection { until_key: false }, Vec::new(), 1, &mut lap).expect("no key ends it");
        let again = [self.conses.colors(), self.closures.colors(), self.cells.colors(), self.objects.colors()];
        for (table, (colors, again)) in ["conses", "closures", "cells", "objects"].iter().zip(colors.iter().zip(&again)) {
            if let Some(i) = colors.iter().zip(again).position(|(a, b)| a != b) {
                panic!("threads and one thread disagree on {table} entry {i}: {} against {}", colors[i], again[i]);
            }
        }
        assert_eq!(threads.state.undo.edits, alone.state.undo.edits, "threads and one thread decided different undo edits");
        assert_eq!(threads.code_marks, alone.code_marks, "threads and one thread reached different code");
        assert_eq!(threads.census, alone.census, "threads and one thread counted different objects");
        assert_eq!(threads.doomed, alone.doomed, "threads and one thread doomed different finalizers");
        assert_eq!(threads.state.obarray_symbols_live, alone.state.obarray_symbols_live, "threads and one thread kept different symbols");
        assert_eq!(threads.state.region_marks, alone.state.region_marks, "threads and one thread kept different regions");
        assert_eq!(sorted(&threads.state.weak_tables), sorted(&alone.state.weak_tables), "threads and one thread found different weak tables");
        assert_eq!(sorted(&threads.state.live_obarrays), sorted(&alone.state.live_obarrays), "threads and one thread found different obarrays");
        assert_eq!(sorted(&threads.state.untraced_lists), sorted(&alone.state.untraced_lists), "threads and one thread kept different lambda lists");
        let mut alone = alone;
        alone.marking.sweep_on(n);
        alone
    }
    /// What the workers hand back, claimed: the kinds with `&mut' visitors,
    /// symbols of an obarray of their own, and frozen values. Only the entry
    /// is written, through HANDED, beside the workers' reads of others.
    fn mark_serial(&self, handed: gc_mark::Handed<'_>, in_obarray: &[bool], pending: &mut Vec<Value>, state: &mut MarkState) {
        let keep = |v: &Value| traced(*v, in_obarray);
        let v = handed.value();
        if v.tag() == 7 {
            let slot = v.payload() as usize - 1;
            if let Some(&name) = self.obarray_symbol_names.get(&slot) && state.obarray_symbols_live.insert(slot) {
                let s = &self.slots[slot];
                pending.extend([name, s.value, s.function, s.plist]);
                if let Some(entry) = self.conditions.entries.get(&conditions::SymbolId::new(v).unwrap()) {
                    for &(key, value) in &entry.properties { pending.extend([key, value]); }
                }
                if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                    pending.push(Value::reference(target + 1, 7));
                }
            }
            return;
        }
        // A frozen value marks its region, whose symbols are then roots;
        // the region's own values are never traced (`frozen.rs`).
        if Self::image_value(v) {
            let id = crate::frozen::region_of(v.payload() as usize);
            if id != 0 && !std::mem::replace(&mut state.region_marks[id], true) {
                pending.extend_from_slice(self.region_symbols(id));
            }
            return;
        }
        let i = v.payload() as usize - 1;
        let object = handed.object();
        if shares_storage(object) { state.shared_seen.push(i as u32); }
        let native_nodes = &mut state.native_nodes;
        // A minor reads the nodes written since the last collection; any
        // other mark, of the image's holders, those written since the restore.
        let nodes_since = || if !state.reading.whole { state.reading.since } else if self.world_image.holds(i) { self.world_image.since } else { 0 };
        match object {
            data::Object::Window(w) => w.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Obarray(t) => {
                state.live_obarrays.push(i);
                t.visit_values(&mut |v| if keep(v) { pending.push(*v) })
            }
            data::Object::Frame(f) => f.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Terminal(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::WindowConfiguration(c) => c.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Treesit(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Process(p) => p.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::CharTable(t) => t.collector_visit_values(native_nodes, nodes_since(), &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::SubCharTable(node) => char_table::collector_visit_node(node, native_nodes, nodes_since(), &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::MapKeymapCursor(c) => c.collector_visit_values(native_nodes, nodes_since(), &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Buffer(t) => {
                let weak = (!state.undo_strong && t.owns_undo_list()).then_some(self.undo_list_slot);
                if weak.is_some() { state.undo_owners.push(i); }
                let locals = u32::try_from(i).map_or(true, |i| state.clean_holders.binary_search(&i).is_err());
                t.visit_values_but(state.reading, weak, locals, &mut |v| if keep(v) { pending.push(*v) });
            }
            data::Object::ByteBuffer(t) => {
                let weak = (!state.undo_strong && t.owns_undo_list()).then_some(self.undo_list_slot);
                if weak.is_some() { state.undo_owners.push(i); }
                let locals = u32::try_from(i).map_or(true, |i| state.clean_holders.binary_search(&i).is_err());
                t.visit_values_but(state.reading, weak, locals, &mut |v| if keep(v) { pending.push(*v) });
            }
            data::Object::Marker(m) => m.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Overlay(o) => o.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::String(s) | data::Object::RawString(s) => s.properties.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::HashTable(t) if t.weakness == hash::Weakness::None => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::HashTable(_) => state.weak_tables.push(i),
            _ => unreachable!("a kind the workers trace"),
        }
    }
    /// The last completed collection: ((PHASE . SECONDS)...), then
    /// (marked CONSES CLOSURES CELLS OBJECTS) and (entries ...) likewise; then
    /// (image ...), the restored image's entries, and (remembered ...), those
    /// of them the last full read.
    pub(crate) fn gc_phases(&mut self) -> Value {
        let times = self.gc_times;
        let counts = |vm: &mut Vm, name: &str, row: [usize; 4]| {
            let mut list = Value::NIL;
            for n in row.into_iter().rev() {
                list = vm.keymap_cons(Value::integer(n as i64).expect("a count fits fixnum"), list);
            }
            let symbol = vm.intern(name);
            vm.keymap_cons(symbol, list)
        };
        let entries = counts(self, "entries", times.entries);
        let marked = counts(self, "marked", times.marked);
        let image = counts(self, "image", self.world_image.entries);
        let remembered = counts(self, "remembered", times.remembered);
        let mut result = self.keymap_cons(remembered, Value::NIL);
        result = self.keymap_cons(image, result);
        result = self.keymap_cons(entries, result);
        result = self.keymap_cons(marked, result);
        for (name, ns) in PHASES.iter().zip(times.ns).rev() {
            let symbol = self.intern(name);
            let seconds = self.allocate_object(data::Object::Float(ns as f64 / 1e9));
            let row = self.keymap_cons(symbol, seconds);
            result = self.keymap_cons(row, result);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_cons_percentage_defers_in_proportion_without_a_cliff() {
        assert_eq!(DEFERRAL_SCALE, 10.0, "the measured choice (VM2-GC-PACING)");
        assert!((deferral(0.1) - 1.0).abs() < 1e-12, "GNU's default");
        assert!((deferral(0.2) - 2.0).abs() < 1e-12);
        assert!((deferral(0.6) - 6.0).abs() < 1e-12, "gcmh-style settings stay proportional");
        for large in [1.8, 2.0, 1e300, f64::INFINITY] {
            assert_eq!(deferral(large), MAX_DEFERRAL, "{large}");
        }
        assert_eq!(deferral(-0.5), 0.0);
        assert_eq!(deferral(f64::NAN), 0.0);
    }

    /// A cache left stale by a collection that frees in place names nothing
    /// by the time a later one compacts: the old callee, the last closure
    /// made, is cut off its table's tail by the first.
    #[test]
    fn a_stale_call_cache_is_emptied_before_a_later_compaction() {
        let mut vm = Vm::new();
        // Verify quarantines what it frees, and cuts no tail.
        vm.set_gc_verify(false);
        vm.eval_source_raw("(fset 'cache-caller #'(lambda () (cache-callee)))").unwrap();
        vm.eval_source_raw("(fset 'cache-callee (let ((held (make-vector 5000 0))) #'(lambda () held)))").unwrap();
        vm.eval_source_raw("(cache-caller)").unwrap();
        vm.eval_source_raw("(fset 'cache-callee 'list)").unwrap();
        let closures = vm.closures.len();
        vm.collect(&mut []);
        assert!(vm.closures.len() < closures, "the old callee was the tail, and is cut");
        vm.set_gc_moving(true);
        vm.collect(&mut []);
        assert_eq!(vm.eval_source_raw("(cache-caller)").unwrap(), Value::NIL, "and the site calls the new one");
    }

    /// The keymaps' late setup rewrites two function cells. A site that
    /// cached what one named before must see the change, whether or not
    /// what it cached is still alive: a cache is trusted only under its
    /// slot's version, so every write of a function cell moves it on.
    #[test]
    fn a_function_cell_rewritten_by_keymap_setup_is_not_called_from_a_cache() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(fset 'ESC-prefix (setq cache-kept #'(lambda () 7)))").unwrap();
        vm.eval_source_raw("(fset 'cache-caller #'(lambda () (ESC-prefix)))").unwrap();
        assert!(vm.eval_source_raw("(cache-caller)").is_ok(), "the closure, while the alias names it");
        vm.eval_source_raw("(make-keymap)").unwrap();
        assert!(vm.eval_source_raw("(cache-caller)").is_err(), "a keymap is no function to call with no event");
    }

    /// A call site's cache keeps no closure alive: once its symbol is
    /// redefined, what the old closure captured is garbage, though a site
    /// that has not run since was the last to name it; and a collection
    /// that compacts copes as one that frees in place does.
    #[test]
    fn a_stale_call_cache_keeps_nothing_alive() {
        for moving in [false, true] {
            let mut vm = Vm::new();
            vm.set_gc_moving(moving);
            vm.eval_source_raw("(fset 'cache-callee (let ((held (make-vector 5000 0))) #'(lambda () held)))").unwrap();
            vm.eval_source_raw("(fset 'cache-caller #'(lambda () (cache-callee)))").unwrap();
            vm.eval_source_raw("(cache-caller)").unwrap();
            vm.collect(&mut []);
            let slots = vm.gc_census.vector_slots;
            assert!(slots >= 5000, "the callee holds its vector");
            vm.eval_source_raw("(fset 'cache-callee #'(lambda () nil))").unwrap();
            vm.collect(&mut []);
            assert!(vm.gc_census.vector_slots + 5000 <= slots, "the old callee's vector died (moving: {moving})");
            assert_eq!(vm.eval_source_raw("(cache-caller)").unwrap(), Value::NIL, "and the site calls the new one");
        }
    }

    /// A call cache filled since the last collection may name what a minor
    /// frees: the minor asks the caches filled, as a full asks them all,
    /// and no cache is left naming a freed closure.
    #[test]
    fn a_minor_empties_a_cache_whose_young_target_died() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000) (fset 'cache-caller #'(lambda () (cache-callee)))").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(fset 'cache-callee (let ((held (make-vector 50 0))) #'(lambda () held)))").unwrap();
        let callee = vm.eval_source_raw("(symbol-function 'cache-callee)").unwrap();
        vm.eval_source_raw("(cache-caller)").unwrap();
        assert!(vm.caches.iter().any(|c| c.target == callee), "the call filled a cache with the young closure");
        vm.eval_source_raw("(fset 'cache-callee #'(lambda () nil))").unwrap();
        assert!(vm.collect_minor(&[]));
        assert!(vm.closures.is_free(callee.payload() as usize), "the young closure died");
        assert!(vm.caches.iter().all(|c| c.target != callee), "no cache names it");
        assert_eq!(vm.eval_source_raw("(cache-caller)").unwrap(), Value::NIL, "and the site calls the new one");
    }

    /// A symbol's name string first asked for since the last collection is
    /// held by the names map alone: a minor reads the names made since, and
    /// keeps it, its oracle agreeing.
    #[test]
    fn a_minor_keeps_a_symbol_name_made_since_the_last_collection() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000) (intern \"minor-named-symbol\")").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(symbol-name 'minor-named-symbol)").unwrap();
        assert!(vm.collect_minor(&[]));
        assert_eq!(vm.eval_source_raw("(equal (symbol-name 'minor-named-symbol) \"minor-named-symbol\")").unwrap(), Value::TRUE);
    }

    /// A name interned into an old obarray of its own since the last
    /// collection is young, and a root of a minor (`names_added`), which
    /// reads no other name of such a symbol.
    #[test]
    fn a_minor_keeps_a_name_interned_into_an_obarray_of_its_own() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 minor-obarray (obarray-make)) (intern \"minor-old\" minor-obarray)").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(intern (concat \"minor-\" \"young\") minor-obarray)").unwrap();
        assert!(vm.collect_minor(&[]));
        assert_eq!(vm.eval_source_raw("(equal (symbol-name (intern-soft \"minor-young\" minor-obarray)) \"minor-young\")").unwrap(), Value::TRUE);
    }

    /// What is put on a condition since the last collection is read by a
    /// minor from the registry's log, which is all it reads of the registry.
    #[test]
    fn a_minor_keeps_what_was_put_on_an_old_condition() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000) (define-error 'minor-condition \"Old\")").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(put 'minor-condition 'error-message (concat \"Young\" \" message\"))").unwrap();
        assert!(vm.collect_minor(&[]));
        assert_eq!(vm.eval_source_raw("(equal (get 'minor-condition 'error-message) \"Young message\")").unwrap(), Value::TRUE);
    }

    /// A collection leaves every survivor old and no log; Lisp's stores into
    /// what survived reach the logs, and the next collection empties them.
    #[test]
    fn a_collection_settles_the_generations_and_lisp_stores_are_logged() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gen-list (list 1 2 3) gen-vector (make-vector 4 nil) gen-table (make-hash-table))").unwrap();
        assert!(!vm.generations_settled(), "what was just made is young");
        vm.collect(&mut []);
        assert!(vm.generations_settled());
        vm.eval_source_raw("(setcar gen-list (list 4)) (setcdr (cdr gen-list) (list 5)) (setcar (cdr gen-list) 6)").unwrap();
        assert_eq!(vm.conses.dirty_old().count(), 2, "the two old conses written");
        vm.eval_source_raw("(aset gen-vector 0 (list 7)) (aset gen-vector 1 'symbol) (puthash (list 8) (list 9) gen-table)").unwrap();
        assert_eq!(vm.objects.logged().len(), 2, "a strong table's key and value");
        assert!(vm.objects.dirty_old().count() >= 1, "and a short vector");
        vm.collect(&mut []);
        assert!(vm.generations_settled());
        let kept = vm.eval_source_raw("(equal (list (car gen-list) (cdr (cdr gen-list)) (aref gen-vector 0) (hash-table-count gen-table)) '((4) (5) (7) 1))").unwrap();
        assert_eq!(kept, Value::TRUE, "what the stores put in survives");
    }

    /// A minor frees the young dead, keeps what an old entry was given and
    /// what a symbol holds, and leaves every survivor old; the oracle checks
    /// each one (verify).
    #[test]
    fn a_minor_frees_the_young_dead_and_keeps_what_the_old_were_given() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq minor-list (list 1 2 3) minor-vector (make-vector 4 nil) minor-table (make-hash-table) minor-weak (make-hash-table :weakness 'key) minor-chars (make-char-table 'minor) minor-symbol nil)").unwrap();
        // Symbols enough after it that no later one shares its card.
        vm.eval_source_raw("(dotimes (i 200) (intern (concat \"minor-filler-\" (number-to-string i))))").unwrap();
        // The table's nodes are split before the collection, so the stores
        // after it go into nodes it left old.
        vm.eval_source_raw("(aset minor-chars ?a 0) (set-char-table-range minor-chars '(#x4e00 . #x4e10) 0)").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(setcar minor-list (list 4)) (aset minor-vector 0 (list 5)) (puthash 'k (list 6) minor-table) (setq minor-young (list 7))").unwrap();
        vm.eval_source_raw("(fset 'minor-function #'(lambda () '(9 (9)))) (setq minor-symbol (list 14))").unwrap();
        vm.eval_source_raw("(aset minor-chars ?a (list 10)) (set-char-table-range minor-chars '(#x4e00 . #x4e10) (list 11))").unwrap();
        vm.eval_source_raw("(puthash (list 'dead) 1 minor-weak) (puthash minor-list (list 8) minor-weak) (dotimes (_ 1000) (list 1 2 3))").unwrap();
        let before = vm.conses.live();
        assert!(vm.collect_minor(&[]));
        assert!(vm.generations_settled());
        assert!(vm.conses.live() + 3000 <= before, "the garbage went: {before} to {}", vm.conses.live());
        let kept = vm.eval_source_raw("(equal (list (car minor-list) (aref minor-vector 0) (gethash 'k minor-table) minor-young (hash-table-count minor-weak) (gethash minor-list minor-weak) (minor-function) (aref minor-chars ?a) (aref minor-chars #x4e08) minor-symbol) '((4) (5) (6) (7) 1 (8) (9 (9)) (10) (11) (14)))").unwrap();
        assert_eq!(kept, Value::TRUE, "what the stores put in survives");
        vm.collect(&mut []);
        assert_eq!(vm.eval_source_raw("(equal (car minor-list) '(4))").unwrap(), Value::TRUE, "and a full after it finds the heap whole");
    }

    /// A minor holds undo lists whole: deciding one (a dead marker's entry,
    /// a cut at the limits) takes edits only a full collection applies, so a
    /// minor leaves the lists to the next full.  A young entry for a marker
    /// nothing holds survives a minor with every cons of its list.
    #[test]
    fn a_minor_holds_undo_lists_whole() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(progn (set-buffer (get-buffer-create \"minor-undo\")) (setq buffer-undo-list nil) (insert \"abcdefghij\"))").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(progn (let ((m (copy-marker 5))) (delete-region 3 8)) (setq minor-undo-before (prin1-to-string buffer-undo-list)))").unwrap();
        assert!(vm.collect_minor(&[]));
        // Printed, so the comparison holds none of the list's conses.
        let whole = vm.eval_source_raw("(equal (prin1-to-string buffer-undo-list) minor-undo-before)").unwrap();
        assert_eq!(whole, Value::TRUE, "the minor left the undo list as it was");
        vm.collect(&mut []);
        assert_eq!(vm.eval_source_raw("(listp buffer-undo-list)").unwrap(), Value::TRUE, "and a full after it decides it");
    }

    /// With minors on, allocation alone brings them: each frees the garbage
    /// made since the last, keeps what a variable still holds, and is
    /// recorded as a minor. `gc-cons-threshold` paces fulls, and here asks
    /// for none.
    #[test]
    fn allocation_brings_minors_when_they_are_on() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_minor(true);
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 minor-keep nil)").unwrap();
        vm.collect(&mut []);
        vm.collection_log = CollectionLog::default();
        let before = vm.conses.live();
        vm.eval_source_raw("(dotimes (i 1200000) (let ((c (list i i))) (when (= 0 (% i 2000)) (setq minor-keep (cons c minor-keep)))))").unwrap();
        let minors = vm.collection_log.records.iter().filter(|record| record.minor).count();
        assert!(minors >= 4, "2.4M conses, a minor each 256K objects: {minors}");
        assert_eq!(vm.collection_log.records.len(), minors, "and no full");
        assert!(vm.conses.live() < before + 400_000, "the garbage went: {before} to {}", vm.conses.live());
        let kept = vm.eval_source_raw("(equal (list (length minor-keep) (car (car minor-keep)) (car (nth 599 minor-keep))) '(600 1198000 0))").unwrap();
        assert_eq!(kept, Value::TRUE, "what the variable holds survives every minor");
    }

    /// Payload made and dropped between collections is the young's: the
    /// minors that free it give it back, so `gc-cons-threshold`, which paces
    /// fulls, sees only what the old generation grew by. Strings that die at
    /// once, eight thresholds of them, bring minors and no full.
    #[test]
    fn a_minor_gives_back_the_payload_it_frees() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_minor(true);
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold (* 64 1024 1024))").unwrap();
        vm.collect(&mut []);
        vm.collection_log = CollectionLog::default();
        vm.eval_source_raw("(dotimes (_ 8192) (make-string 65536 ?x))").unwrap();
        let minors = vm.collection_log.records.iter().filter(|record| record.minor).count();
        assert!(minors >= 64, "512 MiB of strings, a minor each 4 MiB: {minors}");
        assert_eq!(vm.collection_log.records.len(), minors, "and no full");
    }

    /// What a minor leaves charged to the next full is what the young that
    /// survived hold, however the dead changed size after they were charged:
    /// a string grown in place, or shrunk, dies beside a kept one, and only
    /// the kept one's payload stays charged.
    #[test]
    fn a_minor_charges_the_full_what_the_young_survivors_hold() {
        const N: usize = 1 << 20;
        for (grown, how) in [("(string-to-multibyte (make-string n ?a))", "(aset minor-dead i ?😀)"), ("(make-string n ?😀)", "(aset minor-dead i ?a)")] {
            // Its one minor is the one it asks for: four megabytes made bring
            // one, which would promote the string while it is still held.
            let mut vm = Vm::new();
            vm.install_lisp_prelude().unwrap();
            vm.set_gc_minor(false);
            vm.eval_source_raw(&format!("(setq gc-cons-threshold 1000000000 n {N} minor-kept nil minor-dead nil)")).unwrap();
            vm.collect(&mut []);
            // A global, not a lexical: a minor keeps what a lexical pair
            // names until the next full.
            vm.eval_source_raw(&format!("(setq minor-dead {grown}) (dotimes (i 1000) {how}) (setq minor-dead nil minor-kept (make-string (* 3 n) ?k))")).unwrap();
            assert!(vm.collect_minor(&[]));
            let charged = vm.gc_payload_bytes;
            assert!((3 * N..3 * N + N / 4).contains(&charged), "{how}: the kept string, about {} bytes, is what stays charged: {charged}", 3 * N);
        }
    }

    /// A minor marks on one thread, yet where the process may hold threads
    /// what it frees is the dropper's, off the pause, as a full's is.
    #[test]
    fn a_minor_leaves_its_dead_to_the_dropper() {
        let _alone = gc_mark::test::alone();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_minor(true);
        vm.set_gc_threads(2);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000)").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(dotimes (_ 2000) (make-string 64 ?x))").unwrap();
        // The full's batch first: one waits at a time.
        let start = std::time::Instant::now();
        while gc_mark::dropping() && start.elapsed() < std::time::Duration::from_secs(10) { std::thread::yield_now(); }
        let buried = vm.gc_stress.buried;
        assert!(vm.collect_minor(&[]));
        assert!(vm.gc_stress.buried >= buried + 2000, "the young strings went to the dropper: {} then {}", buried, vm.gc_stress.buried);
    }

    /// The dead whose end has side effects are the sweep's to find: markers
    /// made since the last collection and dropped leave their buffer at a
    /// minor, as at a full.
    #[test]
    fn a_dead_young_marker_leaves_its_buffer() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 minor-buffer (get-buffer-create \"minor-markers\")) (set-buffer minor-buffer) (insert \"hello\")").unwrap();
        vm.collect(&mut []);
        let buffer = vm.eval_source_raw("minor-buffer").unwrap().index(0).unwrap() - 1;
        let anchors = |vm: &Vm| match &vm.objects[buffer] { data::Object::Buffer(b) => b.anchors.len(), _ => unreachable!("a buffer") };
        let before = anchors(&vm);
        vm.eval_source_raw("(dotimes (_ 100) (copy-marker 3))").unwrap();
        assert_eq!(anchors(&vm), before + 100, "each marker is anchored");
        for collect in [Vm::collect_minor as fn(&mut Vm, &[Value]) -> bool, |vm, roots| vm.collect_until_key(&mut roots.to_vec(), false)] {
            vm.eval_source_raw("(dotimes (_ 100) (copy-marker 3))").unwrap();
            assert!(collect(&mut vm, &[]));
            assert_eq!(anchors(&vm), before, "the dead markers left their buffer");
        }
    }

    /// A minor inside a request, its engine suspended with code and entry
    /// caches whole: a call site whose symbol was given another function
    /// since it was last entered calls the new one afterwards.
    #[test]
    fn a_minor_inside_a_request_leaves_its_call_sites_right() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_minor(true);
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000) (fset 'entry-caller #'(lambda () (entry-callee)))").unwrap();
        vm.collect(&mut []);
        vm.collection_log = CollectionLog::default();
        // One request: its engine is suspended, code and entry caches whole,
        // when allocation brings the minor.
        let last = vm.eval_source_raw("(progn (fset 'entry-callee #'(lambda () '(1 2))) (entry-caller) (entry-caller)
          (fset 'entry-callee #'(lambda () nil)) (dotimes (i 400000) (list i i)) (entry-caller))").unwrap();
        assert!(vm.collection_log.records.iter().any(|record| record.minor), "a minor ran inside the request");
        assert_eq!(last, Value::NIL, "and the site calls the new one");
        vm.collect(&mut []);
    }

    /// A minor reads no entry cache, since a call fills one with a
    /// primitive or with nothing, never with a heap entry. Were one to name
    /// a function made since the last collection, the minor would free it
    /// under the cache: the oracle, which reads every cache, says so. The
    /// cache is filled by hand here, as no call fills it.
    #[test]
    #[should_panic(expected = "the minor missed a young")]
    fn the_oracle_reports_an_entry_cache_that_names_what_a_minor_frees() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(fset 'entry-caller #'(lambda () (entry-callee))) (fset 'entry-callee #'(lambda () 1))").unwrap();
        vm.eval_source_raw("(entry-caller)").unwrap();
        vm.collect(&mut []);
        let caller = vm.eval_source_raw("(symbol-function 'entry-caller)").unwrap();
        let function = vm.closures[usize::try_from(caller.payload()).unwrap()].function;
        let young = vm.eval_source_raw("(let ((held (list 1 2))) #'(lambda () held))").unwrap();
        assert!(!vm.closures.old(usize::try_from(young.payload()).unwrap()), "made since the collection");
        assert!(vm.execution.plant_entry(function, young), "the caller has lowered code in the idle program");
        vm.collect_minor(&[]);
    }

    /// A buffer's text belongs to no entry alone: a property stored into
    /// it, through the buffer or through an indirect buffer of it, is in
    /// the text's own record, which a minor reads.
    #[test]
    fn a_minor_keeps_what_was_stored_into_an_old_buffers_text() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq minor-buffer (get-buffer-create \"minor\")) (set-buffer minor-buffer) (insert \"hello world\")
          (setq minor-indirect (make-indirect-buffer minor-buffer \"minor-indirect\"))").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(set-buffer minor-buffer) (put-text-property 1 4 'minor (list 12))").unwrap();
        // Another VM of the process collects: the epoch moves on, and what
        // this one recorded must stand.
        let _ = crate::epoch::commit_point();
        vm.eval_source_raw("(set-buffer minor-indirect) (put-text-property 5 8 'minor (list 13))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(progn (set-buffer minor-buffer) (equal (list (get-text-property 2 'minor) (get-text-property 6 'minor)) '((12) (13))))").unwrap();
        assert_eq!(kept, Value::TRUE, "what the stores put in survives");
        vm.collect(&mut []);
    }

    /// A minor reads the holders of storage that records its own writes from
    /// the list the last minor left: old buffers and char tables it met, and
    /// those made since, which it met young. What is stored into each
    /// afterwards survives the next minor, its oracle agreeing.
    #[test]
    fn a_minor_reads_the_holders_the_last_minor_met() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        // Left current: switching away from a buffer stores into its entry.
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 minor-buffer (get-buffer-create \"minor-listed\") minor-table (make-char-table 'minor))
          (set-buffer minor-buffer) (insert \"hello world\")
          (setq minor-indirect (make-indirect-buffer minor-buffer \"minor-listed-indirect\")) (set-buffer minor-indirect)").unwrap();
        vm.collect(&mut []);
        // A full that compacted nothing leaves the holders it met, so no
        // minor asks every object its kind.
        assert!(vm.shared_holders.is_some(), "the full left its list");
        assert!(vm.collect_minor(&[]));
        assert!(vm.shared_holders.is_some(), "the minor left its list");
        // Through the indirect buffer: the base's text is written, and
        // nothing of the base's entry.
        vm.eval_source_raw("(put-text-property 1 4 'minor (list 31))
          (set-char-table-range minor-table ?a (list 32))
          (setq minor-young (get-buffer-create \"minor-young\")) (with-current-buffer minor-young (insert \"young\"))").unwrap();
        // Whatever else stored into the base's entry on the way: only the
        // list can say it holds what its text was given.
        let base = vm.eval_source_raw("minor-buffer").unwrap().index(0).unwrap() - 1;
        vm.objects.forget_store(base);
        assert!(vm.collect_minor(&[]));
        // The once-young buffer, its text replaced whole, then the base's text again.
        vm.eval_source_raw("(with-current-buffer minor-young (set-buffer-multibyte nil) (set-buffer-multibyte t) (put-text-property 1 3 'minor (list 33)))
          (put-text-property 5 8 'minor (list 34))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(equal (list (list (get-text-property 2 'minor) (get-text-property 6 'minor))
          (aref minor-table ?a) (with-current-buffer minor-young (get-text-property 2 'minor))) '(((31) (34)) (32) (33)))").unwrap();
        assert_eq!(kept, Value::TRUE, "what each holder was given survives");
        vm.collect(&mut []);
    }

    /// A buffer whose text is replaced whole (its bytes read again as
    /// characters, and back) has a text born written: a property stored
    /// into it afterwards, and the ones it carried over, survive a minor.
    #[test]
    fn a_minor_reads_a_text_that_replaced_another_in_an_old_buffer() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq minor-buffer (get-buffer-create \"minor-replaced\")) (set-buffer minor-buffer) (insert \"hello world\")").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(set-buffer minor-buffer) (put-text-property 1 4 'minor (list 15)) (set-buffer-multibyte nil)
          (set-buffer-multibyte t) (put-text-property 5 8 'minor (list 16))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(progn (set-buffer minor-buffer) (equal (list (get-text-property 2 'minor) (get-text-property 6 'minor)) '((15) (16))))").unwrap();
        assert_eq!(kept, Value::TRUE, "what both texts were given survives");
        vm.collect(&mut []);
    }

    /// A text given more values than its record keeps is read whole.
    #[test]
    fn a_minor_reads_whole_a_text_whose_record_overflowed() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 minor-buffer (get-buffer-create \"minor-overflow\")) (set-buffer minor-buffer) (insert \"hello world\")").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(set-buffer minor-buffer) (put-text-property 5 8 'first (list 17)) (dotimes (i 70000) (put-text-property 1 2 'minor (list i)))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(progn (set-buffer minor-buffer) (equal (list (get-text-property 6 'first) (get-text-property 1 'minor)) '((17) (69999))))").unwrap();
        assert_eq!(kept, Value::TRUE, "the first value stored and the last survive");
        vm.collect(&mut []);
    }

    /// A minor makes the old entries it reads again young for its length.
    /// A VM used again after one panicked part way (`Vm::catching`) has
    /// none young: everything is promoted, so what was old stays in use and
    /// old, and the next minor frees none of it.
    #[test]
    fn a_minor_that_panics_promotes_everything() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq panic-list (list 1 2 3) panic-held (list (list 4)))").unwrap();
        vm.collect(&mut []);
        // An old cons stored into, which the minor will read again, and
        // which only an old and clean cons names.
        vm.eval_source_raw("(setcar (car panic-held) 5)").unwrap();
        vm.gc_stress.faults.minor_panics = true;
        let unwound = vm.catching(|vm| vm.collect_minor(&[]));
        assert!(unwound.is_err(), "the planted fault");
        assert!(vm.generations_settled(), "everything in use is old and clean");
        vm.set_gc_verify(true);
        vm.eval_source_raw("(dotimes (_ 100) (list 1 2 3))").unwrap();
        assert!(vm.collect_minor(&[]));
        assert_eq!(vm.eval_source_raw("(equal panic-held '((5)))").unwrap(), Value::TRUE, "what was old survived the next minor");
    }

    /// A capture cell's lexical pair is named by `cell_pairs` alone. One
    /// made since the last collection, for a cell that collection left old
    /// and clean, survives a minor with its cell.
    #[test]
    fn a_minor_keeps_the_new_lexical_pair_of_an_old_cell() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        let cell = vm.alloc_cell(Value::TRUE);
        let mut roots = [Value::reference(cell, 5)];
        vm.collect(&mut roots);
        assert!(vm.cells.old(cell));
        let pair = vm.alloc_cons(Value::NIL, Value::integer(5).unwrap());
        vm.cell_pairs.insert(cell, pair);
        assert!(vm.collect_minor(&roots));
        assert!(vm.conses.old(usize::try_from(pair.payload()).unwrap()), "the pair survived, and is old");
        assert_eq!(vm.cell_pairs.get(&cell), Some(&pair));
    }

    /// After a promote-all nothing is young and every record is empty: a
    /// store into a char-table node, a text and a symbol afterwards is
    /// recorded afresh, and the next minor reads it.
    #[test]
    fn a_store_after_a_promote_all_is_read_by_the_next_minor() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq after-chars (make-char-table 'after) after-symbol nil after-buffer (get-buffer-create \"after\"))
          (aset after-chars ?a 0) (set-buffer after-buffer) (insert \"hello world\")").unwrap();
        vm.promote_all();
        assert!(vm.generations_settled());
        vm.eval_source_raw("(aset after-chars ?a (list 1)) (setq after-symbol (list 2)) (set-buffer after-buffer) (put-text-property 1 4 'after (list 3))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(progn (set-buffer after-buffer) (equal (list (aref after-chars ?a) after-symbol (get-text-property 2 'after)) '((1) (2) (3))))").unwrap();
        assert_eq!(kept, Value::TRUE);
    }

    /// An indirect buffer made since the last collection shares the text of
    /// its old base: a property stored through it survives a minor, and so
    /// does the indirect buffer.
    #[test]
    fn a_minor_keeps_a_young_indirect_buffer_and_what_it_stored() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq young-base (get-buffer-create \"young-base\")) (set-buffer young-base) (insert \"hello world\")").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(setq young-indirect (make-indirect-buffer young-base \"young-indirect\")) (set-buffer young-indirect) (put-text-property 1 4 'young (list 1))").unwrap();
        assert!(vm.collect_minor(&[]));
        let kept = vm.eval_source_raw("(progn (set-buffer young-base) (and (buffer-live-p young-indirect) (equal (get-text-property 2 'young) '(1))))").unwrap();
        assert_eq!(kept, Value::TRUE);
        vm.collect(&mut []);
    }

    /// Stress that frees in place forces a minor at every Lisp entry where
    /// minors are on, each checked by the oracle; a full still comes when
    /// the heap's growth asks for one.
    #[test]
    fn stress_in_place_forces_minors() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_minor(true);
        vm.set_gc_verify(true);
        vm.eval_source_raw("(setq gc-cons-threshold 1000000000 stress-keep nil)").unwrap();
        vm.collect(&mut []);
        vm.collection_log = CollectionLog::default();
        vm.gc_stress.inplace = true;
        vm.set_gc_stress(1);
        vm.eval_source_raw("(dotimes (i 20) (setq stress-keep (cons (list i) stress-keep)))").unwrap();
        vm.set_gc_stress(0);
        let minors = vm.collection_log.records.iter().filter(|record| record.minor).count();
        assert!(minors >= 1, "forced collections were minors: {:?}", vm.collection_log.records.iter().map(|record| record.minor).collect::<Vec<_>>());
        assert_eq!(vm.eval_source_raw("(length stress-keep)").unwrap(), Value::integer(20).unwrap());
    }

    /// With a log lost a minor frees nothing and promotes everything.
    #[test]
    fn a_minor_with_a_lost_log_promotes_everything() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq lost-table (make-hash-table))").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(dotimes (_ 100) (list 1 2 3))").unwrap();
        vm.objects.lose_the_log();
        let before = vm.conses.live();
        assert!(vm.collect_minor(&[]));
        assert!(vm.generations_settled());
        assert_eq!(vm.conses.live(), before, "nothing was freed");
    }

    /// What a capture does to the heap it writes: only old entries and no
    /// log, whatever was allocated or stored since the last collection.
    #[test]
    fn settling_leaves_one_old_generation() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq gen-old (list 1 2) gen-table (make-hash-table))").unwrap();
        vm.collect(&mut []);
        vm.eval_source_raw("(setcar gen-old (list 3)) (puthash (list 4) (list 5) gen-table)").unwrap();
        assert!(!vm.generations_settled());
        vm.settle_generations();
        assert!(vm.generations_settled());
    }

    #[test]
    fn shared_native_nodes_count_once_and_forward_once() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.conses.extend([(Value::NIL, Value::NIL); 2]);
        let payload = Value::reference(vm.conses.len(), 3);
        vm.conses.push((Value::TRUE, Value::NIL));
        let mut first = char_table::CharTable::new(Value::NIL);
        first.set(65, payload).unwrap();
        let second = char_table::CharTable::sharing_nodes_of(&first);
        let native_bytes = char_table::CharTableNode::storage_bytes(
            first.root(), false, &mut HashSet::default(),
        );
        vm.objects.push(data::Object::CharTable(Box::new(first)));
        vm.objects.push(data::Object::CharTable(Box::new(second)));
        let mut roots = [Value::reference(1, 0), Value::reference(2, 0)];
        let stats = vm.heap_stats();
        assert_eq!(stats.live_objects, 5, "native nodes are not arena objects");
        assert_eq!(stats.live_bytes, 3 * size_of::<(Value, Value)>()
            + 2 * size_of::<data::Object>() + 2 * size_of::<char_table::CharTable>()
            + native_bytes);
        vm.collect(&mut roots);
        for root in roots {
            let data::Object::CharTable(table) = vm.object(root).unwrap() else { panic!() };
            assert_eq!(table.get(65).unwrap(), Value::reference(0, 3));
        }
        assert_eq!(vm.heap_stats().live_objects, 3);
        assert_eq!(vm.heap_stats().reclaimed, 2);
        // A completely live arena must still take the original no-move path.
        vm.collect(&mut roots);
        assert_eq!(vm.heap_stats().live_objects, 3);
        assert_eq!(vm.heap_stats().reclaimed, 2);
    }

    #[test]
    fn request_code_dies_after_cache_clear() {
        let mut vm = Vm::new();
        for n in 0..128 {
            assert_eq!(
                vm.eval_source_raw(&format!("(let ((x {n})) (+ x 1))"))
                    .unwrap()
                    .as_integer()
                    .unwrap(),
                n + 1
            );
        }
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert!(
            vm.functions.iter().filter(|f| !f.code.is_empty()).count() <= 1,
            "unreachable request functions must be reclaimed"
        );
    }
    #[test]
    fn strings_and_vectors_survive_relocation_and_reclaim_exactly() {
        use data::Object;
        for vector in [false, true] {
            let mut vm = Vm::new();
            vm.set_gc_moving(true);
            // Dead prefix plus two live strings or vectors; vector edges include
            // a cons-car object, a cell and a closure, and a self cycle.
            vm.objects
                .push(Object::String(Box::new(data::LispString::new("x"))));
            vm.objects
                .push(Object::String(Box::new(data::LispString::new("λa"))));
            let string = Value::reference(2, 0);
            let mut roots = if vector {
                vm.conses.push((string, Value::NIL));
                vm.cells.push(string);
                vm.closures.push(Closure {
                    function: 0,
                    captures: vec![string],
                    metadata: vec![], document: Value::NIL,
                });
                vm.objects.push(Object::Vector(vec![
                    string,
                    Value::reference(0, 3),
                    Value::reference(0, 5),
                    Value::reference(0, 4),
                    Value::reference(3, 0),
                ]));
                vec![Value::reference(3, 0)]
            } else {
                vec![string]
            };
            let allocated = if vector { 6 } else { 2 };
            let retained = allocated - 1;
            assert_eq!(vm.heap_stats().allocations, allocated as u64);
            vm.collect(&mut roots);
            assert_eq!(vm.heap_objects(), retained);
            assert_eq!(vm.heap_stats().reclaimed, 1);
            assert!(matches!(&vm.objects[0], Object::String(s) if s.as_str() == "λa"));
            if vector {
                assert!(
                    matches!(&vm.objects[1], Object::Vector(v) if v[0] == Value::reference(1, 0) && v[4] == roots[0])
                );
                assert_eq!(vm.conses[0].0, Value::reference(1, 0));
                assert_eq!(vm.cells[0], Value::reference(1, 0));
                assert_eq!(vm.closures[0].captures[0], Value::reference(1, 0));
            } else {
                assert_eq!(roots[0], Value::reference(1, 0));
            }
            vm.collect(&mut []);
            assert_eq!(vm.heap_objects(), 0);
            assert_eq!(vm.heap_stats().allocations, allocated as u64);
            assert_eq!(vm.heap_stats().reclaimed, allocated as u64);
        }
    }
    #[test]
    fn statistics_measure_real_storage_and_exclude_private_slots() {
        fn rows(vm: &mut Vm) -> HashMap<String, Vec<i64>> {
            let mut result = HashMap::default();
            let mut list = vm.garbage_collect_statistics();
            while list != Value::NIL {
                let (mut row, next) = vm.cons_pair(list).unwrap();
                let (name, rest) = vm.cons_pair(row).unwrap();
                row = rest;
                let mut numbers = Vec::new();
                while row != Value::NIL {
                    let (number, next) = vm.cons_pair(row).unwrap();
                    numbers.push(number.integer_value().unwrap());
                    row = next;
                }
                result.insert(vm.symbol_names[name.payload() as usize - 1].clone(), numbers);
                list = next;
            }
            result
        }
        let mut vm = Vm::new();
        let symbol_count = vm.slots.len();
        vm.objects.push(data::Object::String(Box::new(data::LispString::new("abc"))));
        vm.objects.push(data::Object::Vector(vec![Value::NIL; 7]));
        vm.objects.push(data::Object::Float(1.0));
        // The figures are what the last mark counted.
        let mut roots = [Value::reference(1, 0), Value::reference(2, 0), Value::reference(3, 0)];
        vm.collect(&mut roots);
        let actual = rows(&mut vm);
        assert_eq!(actual["symbols"][1], symbol_count as i64, "GC_PUBLIC_SYMBOLS_ONLY");
        assert_eq!(actual["strings"][1], 1, "GC_REAL_STRINGS");
        assert_eq!(actual["string-bytes"], [1, 3], "GC_REAL_STRING_BYTES");
        assert_eq!(actual["vectors"][1], 1, "GC_REAL_VECTORS");
        assert_eq!(actual["vector-slots"][1], 7, "GC_REAL_VECTOR_SLOTS");
        assert_eq!(actual["floats"][1], 1, "GC_REAL_FLOATS");
        assert!(!actual.contains_key("intervals"), "GC_NO_FABRICATED_INTERVAL_POOL");
    }

    #[test]
    fn private_cycles_survive_compaction_and_are_reclaimed_when_unrooted() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.conses.push((Value::integer(99).unwrap(), Value::NIL));
        vm.cells.push(Value::reference(0, 4));
        vm.closures.push(Closure {
            function: 0,
            captures: vec![Value::reference(0, 5)],
            metadata: vec![], document: Value::NIL,
        });
        vm.conses
            .push((Value::reference(0, 5), Value::reference(1, 3)));
        let mut roots = [Value::reference(1, 3)];
        vm.collect(&mut roots);
        assert_eq!(vm.heap_objects(), 3);
        assert_eq!(roots[0], Value::reference(0, 3));
        assert_eq!(vm.conses[0].1, roots[0]);
        assert_eq!(vm.cells[0], Value::reference(0, 4));
        assert_eq!(vm.closures[0].captures[0], Value::reference(0, 5));
        vm.collect(&mut []);
        assert_eq!(vm.heap_stats().allocations, 4);
        assert_eq!(vm.heap_stats().reclaimed, 4);
        assert_eq!(vm.heap_objects(), 0);
    }

    impl Vm {
        /// `collect_compacting', keeping ROOTS.
        fn collect_compacting_with(&mut self, roots: &mut [Value]) {
            self.allocation.compact = true;
            self.collect(roots);
        }
    }

    fn conses(vm: &mut Vm, n: i64) -> Vec<Value> {
        (0..n).map(|i| vm.alloc_cons(Value::integer(i).unwrap(), Value::NIL)).collect()
    }

    /// `Allocation::allocated` is the poll's meter as well as the
    /// profiler's: every kind of allocation advances it by at least what it
    /// made, or a primitive that makes only that kind would never ask its
    /// engine to poll. A compiled unit's load is charged likewise
    /// (compiled_lisp_tests.rs, `install_charge`).
    #[test]
    fn every_allocation_kind_advances_the_allocation_meter() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        let value = size_of::<Value>() as u64;
        let mut check = |kind: &str, least: u64, allocate: &dyn Fn(&mut Vm)| {
            let before = vm.allocation.allocated;
            allocate(&mut vm);
            let advanced = vm.allocation.allocated - before;
            assert!(advanced >= least, "{kind} advanced the meter {advanced}, under {least}");
        };
        check("cons", 2 * value, &|vm| { vm.alloc_cons(Value::NIL, Value::NIL); });
        check("cell", value, &|vm| { vm.alloc_cell(Value::NIL); });
        check("closure", 2 * value, &|vm| {
            vm.alloc_closure(Closure { function: 0, captures: vec![Value::NIL; 2], metadata: Vec::new(), document: Value::NIL });
        });
        check("object", size_of::<data::Object>() as u64, &|vm| { vm.allocate_object(data::Object::Float(1.0)); });
        check("string payload", 1000, &|vm| { vm.make_lisp_string(vec![b'x'; 1000], false); });
        check("vector payload", 100 * value, &|vm| { vm.allocate_object(data::Object::Vector(vec![Value::NIL; 100])); });
        check("bool vector", 1000, &|vm| {
            vm.allocate_object(data::Object::BoolVector(Box::new(data::BoolVector::new(8000, false).unwrap())));
        });
    }

    /// A collection's record says how many objects past its trigger it
    /// began: none below the trigger, and exactly the excess above it.
    #[test]
    fn a_collection_records_how_far_past_its_trigger_it_began() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_collection_floor(100_000);
        vm.collect(&mut []);
        // The floor's trigger, as `collection_budget` sets it: twice what the
        // last collection kept, at least the floor, less what it kept.
        let budget = u32::try_from(vm.gc_retained.saturating_mul(2).max(100_000) - vm.gc_retained).unwrap();
        let trigger = |vm: &Vm| vm.collection_budget().1.to_string();
        assert_eq!(trigger(&vm), budget.to_string(), "the floor sets the trigger");
        conses(&mut vm, i64::from(budget / 2));
        vm.collect(&mut []);
        assert_eq!(trigger(&vm), budget.to_string(), "dead conses leave the trigger where it was");
        conses(&mut vm, i64::from(budget) + 5_000);
        vm.collect(&mut []);
        let late: Vec<String> = vm.collection_log.records.iter().map(|record| record.late.to_string()).collect();
        assert_eq!(late[late.len() - 2..], ["0", "5000"], "below the trigger, then 5,000 past it: {late:?}");
    }

    /// A mark twin's mark (`mark_only') marks what the roots reach, keeps a
    /// weak table's entries a collection's mark would drop, leaves the rest
    /// unmarked, and frees nothing.
    #[test]
    fn a_twins_mark_keeps_weak_entries_and_frees_nothing() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let table = vm.eval_source_raw("(let ((h (make-hash-table :test 'eq :weakness 'key))) (puthash (list 'weak-key) (list 'weak-value) h) h)").unwrap();
        let (key, value) = match vm.object(table).unwrap() {
            data::Object::HashTable(t) => t.entries().map(|(_, k, v)| (k, v)).next().unwrap(),
            _ => unreachable!("a hash table"),
        };
        let reached = vm.alloc_cons(Value::integer(1).unwrap(), Value::NIL);
        let lost = vm.alloc_cons(Value::integer(2).unwrap(), Value::NIL);
        let (conses, objects) = (vm.conses.len(), vm.objects.len());
        vm.mark_only(&[table, reached], true);
        let marked = |v: Value| vm.conses.marked(v.payload() as usize);
        assert!(marked(reached));
        assert!(!marked(lost), "nothing reaches it");
        assert!(marked(key) && marked(value), "a weak entry is kept");
        assert_eq!((vm.conses.len(), vm.objects.len()), (conses, objects), "nothing is freed");
        assert!(vm.cons_pair(lost).is_ok(), "an unmarked entry stays allocated");
    }

    /// A mark twin's mark keeps a chain of weak tables, each reached only
    /// through a dead-keyed entry of the one before, taking each once: a
    /// rescan of every table each round would look through 1275, not 50.
    #[test]
    fn a_twins_mark_keeps_a_chain_of_weak_tables() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let first = vm.eval_source_raw("(let ((next nil)) (dotimes (_ 50) (let ((h (make-hash-table :test 'eq :weakness 'key))) \
            (when next (puthash (list 'dead) next h)) (setq next h))) next)").unwrap();
        let empty = vm.mark_only(&[], true).weak_visits;
        vm.clear_marks();
        let visits = vm.mark_only(&[first], true).weak_visits - empty;
        assert_eq!(visits, 50, "each table of the chain is looked through once");
        let (mut table, mut chain) = (Some(first), 0);
        while let Some(t) = table {
            assert!(vm.objects.marked(t.payload() as usize - 1), "table {chain} of the chain is kept");
            chain += 1;
            table = match vm.object(t).unwrap() {
                data::Object::HashTable(h) => h.entries().map(|(_, _, value)| value).next(),
                _ => unreachable!("a hash table"),
            };
        }
        assert_eq!(chain, 50);
    }

    /// S3.1: a collection leaves live entries where they are, frees the
    /// others in place, cuts the dead tail, and allocation takes the lowest
    /// free entry first, counting what it hands out.
    #[test]
    fn collections_free_in_place_and_allocation_reuses_the_lowest_entry() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let list = conses(&mut vm, 6);
        let string = vm.allocate_object(data::Object::String(Box::new(data::LispString::new("dead"))));
        let kept = vm.allocate_object(data::Object::Vector(vec![list[1]]));
        let mut roots = [list[3], list[4], kept];
        vm.collect(&mut roots);
        assert_eq!(roots, [list[3], list[4], kept], "nothing moves");
        assert_eq!(vm.conses.len(), 5, "the dead tail is cut");
        assert_eq!(vm.conses.unused(), 2, "entries 0 and 2");
        assert_eq!(vm.conses[0], (Value::NIL, Value::NIL));
        assert_eq!(vm.objects.unused(), 1);
        assert!(vm.object(string).is_err(), "a free entry is no object");
        assert_eq!(vm.heap_objects(), 4);
        let before = vm.allocation.allocated;
        let fresh = conses(&mut vm, 3);
        assert_eq!(fresh, [list[0], list[2], list[5]]);
        assert_eq!(vm.allocation.allocated - before, 3 * 16, "reused entries count as allocation");
        assert_eq!(vm.allocate_object(data::Object::Float(2.0)), string);
        assert_eq!(vm.conses.unused() + vm.objects.unused(), 0);
        assert_eq!(vm.heap_objects(), 8);
        assert_eq!(vm.cons_pair(list[1]).unwrap().0, Value::integer(1).unwrap());
    }

    /// Each sweep rebuilds the free lists from its marks: an entry is listed
    /// once however many collections it stays free, and a table nothing
    /// survives in is cut to nothing.
    #[test]
    fn repeated_sweeps_free_each_entry_once() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let list = conses(&mut vm, 8);
        let mut roots = [list[7]];
        for round in 0..4 {
            if round > 1 {
                conses(&mut vm, 2);
            }
            vm.collect(&mut roots);
            assert_eq!(vm.conses.unused(), 7, "round {round}");
            assert_eq!(roots[0], list[7]);
        }
        roots[0] = Value::NIL;
        vm.collect(&mut roots);
        assert!(vm.conses.is_empty() && vm.conses.unused() == 0, "a dead table is cut to nothing");
    }

    /// A dead cell's lexical pair goes with it, before its entry is reused.
    #[test]
    fn a_reused_cell_inherits_no_lexical_pair() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let first = vm.alloc_cell(Value::TRUE);
        let pair = vm.alloc_cons(Value::NIL, Value::integer(5).unwrap());
        let dead = vm.alloc_cell(Value::NIL);
        vm.cell_pairs.insert(dead, pair);
        let last = vm.alloc_cell(Value::TRUE);
        let mut roots = [Value::reference(first, 5), Value::reference(last, 5)];
        vm.collect(&mut roots);
        assert!(!vm.cell_pairs.contains_key(&dead), "the dead cell's pair is pruned");
        assert_eq!(vm.alloc_cell(Value::NIL), dead);
        assert!(!vm.cell_pairs.contains_key(&dead));
    }

    /// A compaction after sweeps moves every live entry down and leaves no
    /// free entry, even when everything left is live.
    #[test]
    fn a_compaction_after_sweeps_leaves_dense_tables() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let list = conses(&mut vm, 6);
        let mut roots = [list[1], list[5]];
        vm.collect(&mut roots);
        assert_eq!(vm.conses.unused(), 4);
        vm.allocation.compact = true;
        vm.collect(&mut roots);
        assert_eq!(roots, [Value::reference(0, 3), Value::reference(1, 3)]);
        assert_eq!((vm.conses.len(), vm.conses.unused()), (2, 0));
        assert_eq!(vm.cons_pair(roots[1]).unwrap().0, Value::integer(5).unwrap());
        assert!(!vm.allocation.compact, "a request compacts once");
        conses(&mut vm, 2);
        vm.collect(&mut roots);
        assert_eq!(roots, [Value::reference(0, 3), Value::reference(1, 3)], "back to freeing in place");
        assert_eq!((vm.conses.len(), vm.conses.unused()), (2, 0));
    }

    /// Nothing but a request moves the heap: not a table a collapsed burst
    /// left nearly empty, whose survivor holds up its tail, at any collection,
    /// idle ones included. Its free entries are reused, lowest first.
    #[test]
    fn only_a_request_compacts() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let n = 1_000_000;
        let list = conses(&mut vm, n);
        let last = list[n as usize - 1];
        let mut roots = [last];
        for idle in [false, false, false, true, true] {
            vm.collect_until_key(&mut roots, idle);
            assert_eq!(roots[0], last, "nothing moved");
        }
        assert_eq!(vm.conses.unused(), n as usize - 1);
        assert_eq!(vm.alloc_cons(Value::NIL, Value::NIL), list[0], "the lowest entry is reused");
        vm.collect_compacting_with(&mut roots);
        assert_eq!(roots[0], Value::reference(0, 3), "a request compacts");
    }

    /// A burst swept while its last entry survives, then released: tables
    /// shrink back whatever the burst's size. Each burst is past what a table
    /// that took its reservation keeps for the next collection's allocations
    /// (`Table::shrink'), which is the path of a process with no address-space
    /// limit, a session's; one with a limit takes the other.
    #[test]
    fn a_collapsed_burst_leaves_no_capacity_behind() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        // A table keeps room for the next cycle's allocation: GNU's threshold
        // makes that small beside the bursts (VM2-GC-PACING).
        vm.eval_source_raw("(setq gc-cons-threshold 800000)").unwrap();
        let mut after = vec![];
        for n in [160_000, 320_000, 640_000] {
            let list = conses(&mut vm, n);
            let mut roots = [list[n as usize - 1]];
            vm.collect(&mut roots);
            roots[0] = Value::NIL;
            vm.collect(&mut roots);
            after.push(vm.heap_stats().capacity_bytes);
        }
        assert!(after.iter().all(|&c| c == after[0]), "capacity grew with the burst: {after:?}");
    }

    /// `ROMACS_GC_VERIFY': freed entries are poisoned and quarantined, so a
    /// stale reference finds poison, never a new tenant.
    #[test]
    fn verify_mode_poisons_and_quarantines_freed_entries() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(true);
        vm.alloc_cons(Value::TRUE, Value::NIL);
        let cell = vm.alloc_cell(Value::TRUE);
        let closure = vm.alloc_closure(Closure { function: 0, captures: vec![], metadata: vec![], document: Value::NIL });
        let object = vm.allocate_object(data::Object::Float(1.0));
        let mut roots = [vm.alloc_cons(Value::NIL, Value::NIL)];
        vm.collect(&mut roots);
        assert_eq!(vm.conses[0], (POISON, POISON));
        assert_eq!(vm.cells[cell], POISON);
        assert_eq!(vm.closures[closure.payload() as usize].function, usize::MAX);
        assert!(vm.object(object).is_err());
        assert_eq!((vm.conses.unused(), vm.cells.unused(), vm.closures.unused(), vm.objects.unused(), vm.heap_objects()), (1, 1, 1, 1, 1), "counted free");
        assert_eq!((vm.conses.len(), vm.cells.len(), vm.objects.len()), (2, 1, 1), "never cut");
        assert_eq!(vm.alloc_cons(Value::NIL, Value::NIL), Value::reference(2, 3), "never reused");
    }
}
