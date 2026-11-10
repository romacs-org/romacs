//! Strand-local stop-at-wait-point compacting collector. No process-global state.
use super::*;

#[derive(Clone, Copy, Debug, Default)]
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
romacs_snapshot::snapshot_struct!(HeapStats { collections, allocations, reclaimed, live_objects, live_bytes, capacity_bytes, elapsed_ns });

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
#[derive(Clone, Copy, Debug, Default)]
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
    /// Faults a test plants in this VM's collections, and no other's.
    #[cfg(test)]
    pub(crate) faults: Faults,
    /// Graves this VM's collections have handed the dropper (or held, for a
    /// test that holds them), for a test to ask.
    #[cfg(test)]
    pub(crate) buried: usize,
}
romacs_snapshot::snapshot_struct!(Stress { every, entries, pads, shift, verify, moving, threads, inplace, #[cfg(test)] faults, #[cfg(test)] buried });

/// What a test breaks in a collection, to see a check catch it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Faults {
    /// No string's properties handed over, in what both marks share.
    pub(crate) skip_properties: bool,
    /// The first chunk a pool worker takes panics.
    pub(crate) worker_panics: bool,
    /// A cons allocated between the mark and the scan, or the sweep.
    pub(crate) allocate_before_scan: bool,
    pub(crate) allocate_before_sweep: bool,
}
#[cfg(test)]
romacs_snapshot::snapshot_struct!(Faults { skip_properties, worker_panics, allocate_before_scan, allocate_before_sweep });

/// Records a session keeps between its usage log's writes: at one a minute
/// or so, this holds a burst; past it collections are only counted.
pub(crate) const COLLECTION_RECORDS: usize = 256;

/// A finished collection, for a real pause to name itself in the usage log
/// (rlisp/romacs-usage-log.el). MS bounds what a key or command waited.
#[derive(Clone, Debug, Default)]
pub(crate) struct CollectionRecord {
    /// The idle pass's or a prompt's: it gave way to no key while marking.
    pub(crate) idle: bool,
    /// Asked for by `garbage-collect' or the host.
    pub(crate) explicit: bool,
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
}
romacs_snapshot::snapshot_struct!(CollectionRecord { idle, explicit, us, command, key, redisplay, at, objects, bytes });

/// The collections since `Vm::take_collections', at most COLLECTION_RECORDS.
#[derive(Clone, Debug, Default)]
pub(crate) struct CollectionLog {
    records: Vec<CollectionRecord>,
    /// Collections past the limit.
    dropped: u64,
    /// The running collection was asked for.
    pub(crate) explicit: bool,
}
romacs_snapshot::snapshot_struct!(CollectionLog { records, dropped, explicit });

/// What a collection does, in order.
#[derive(Clone, Copy)]
pub(crate) enum Phase { Setup, Roots, Mark, Weak, Retire, Census, Obarray, Sweep, Shrink, Settle, Rewrite }
/// The names `romacs--gc-phases' answers, by `Phase'.
const PHASES: [&str; 11] = ["setup", "roots", "mark", "weak", "retire", "census", "obarray", "sweep", "shrink", "settle", "rewrite"];
/// Where the last completed collection's time went, in nanoseconds by
/// `Phase': zero in a world build, whose heap never depends on a clock.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GcTimes {
    pub(crate) ns: [u64; PHASES.len()],
    /// Entries marked, per table: conses, closures, cells, objects.
    pub(crate) marked: [usize; 4],
    /// Entries the collection found in each table.
    pub(crate) entries: [usize; 4],
}
romacs_snapshot::snapshot_struct!(GcTimes { ns, marked, entries });

/// What a completed mark holds, for the rest of the collection.
struct Marked {
    marking: gc_mark::Marking,
    own: gc_mark::Worker,
    doomed: Vec<Value>,
    local_live: HashSet<usize>,
    code_marks: Vec<bool>,
    census: Census,
    state: MarkState,
}

/// What the coordinator gathers while marking.
#[derive(Default)]
struct MarkState {
    native_nodes: HashSet<usize>,
    /// Weak hash tables reached; their entries are not traced until
    /// `weak_round' (the ephemeron fixpoint) decides which survive.
    weak_tables: Vec<usize>,
    /// Obarrays reached: each drops its removed entries of dead symbols.
    live_obarrays: Vec<usize>,
    obarray_symbols_live: HashSet<usize>,
    /// Frozen regions reached (`frozen.rs'); region 0 is the executable
    /// image, always live.
    region_marks: Vec<bool>,
}

/// A running phase clock (`GcTimes').
pub(crate) struct Lap(pub(crate) Option<std::time::Instant>);
impl Lap {
    pub(crate) fn start() -> Self { Self(crate::host::collection_clock()) }
    /// Charge the time since the last lap to PHASE of TIMES.
    pub(crate) fn lap(&mut self, times: &mut GcTimes, phase: Phase) {
        let Some(at) = self.0 else { return };
        let now = std::time::Instant::now();
        times.ns[phase as usize] += (now - at).as_nanos() as u64;
        self.0 = Some(now);
    }
}

/// What the funnels count as they hand entries out (`Vm::alloc_cons' and its
/// siblings, data.rs).
#[derive(Clone, Debug, Default)]
pub(crate) struct Allocation {
    /// Bytes the funnels have handed out, ever: the memory profiler's
    /// meter (profiler.rs), which table growth no longer measures.
    pub(crate) allocated: u64,
    /// GNU's consing counters, ever (`memory-use-counts').
    pub(crate) consed: Consed,
    /// The next collection compacts (`Vm::collect_compacting').
    pub(crate) compact: bool,
}
romacs_snapshot::snapshot_struct!(Allocation { allocated, consed, compact });

/// What the last mark found in the objects table: `garbage-collect's
/// statistics and the pacing's traced slots, counted once per entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
romacs_snapshot::snapshot_struct!(Census { strings, string_bytes, vectors, vector_slots, vector_free, floats, buffers });
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
/// the symbol table, less its private binding slots, and VM2 has no intervals
/// to count (VM2-MEMCOUNT1).
#[derive(Clone, Debug, Default)]
pub(crate) struct Consed {
    pub(crate) conses: u64,
    pub(crate) floats: u64,
    pub(crate) vector_cells: u64,
    pub(crate) reused_symbols: u64,
    pub(crate) string_chars: u64,
    pub(crate) strings: u64,
}
romacs_snapshot::snapshot_struct!(Consed { conses, floats, vector_cells, reused_symbols, string_chars, strings });

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
        let every = std::env::var("ROMACS_GC_STRESS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        let verify = std::env::var_os("ROMACS_GC_VERIFY").is_some_and(|v| !v.is_empty() && v != "0");
        let threads = std::env::var("ROMACS_GC_WORKERS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        let inplace = std::env::var_os("ROMACS_GC_INPLACE").is_some_and(|v| !v.is_empty() && v != "0");
        Self { every, verify, threads, inplace, ..Self::default() }
    }
    /// This process's test mode, from its environment, for a VM a world
    /// carried here: the fillers the last collection left stay counted.
    pub(crate) fn rearm(&mut self) {
        let fresh = Self::from_env();
        (self.every, self.entries, self.verify, self.threads, self.inplace) = (fresh.every, fresh.entries, fresh.verify, fresh.threads, fresh.inplace);
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
        self.site_leaf = (0, 0);
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
    /// Run Lisp that must not collect: BODY's collections are refused (the
    /// funnel sees the hold), so its caller may keep Values in Rust locals
    /// across it. Named so each one is counted: a deferred collection is
    /// latency debt.
    pub(crate) fn defer<T>(&mut self, body: impl FnOnce(&mut Vm) -> T) -> T {
        self.gc_holds += 1;
        let out = body(self);
        self.gc_holds -= 1;
        out
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
    /// builder and no watcher notification from an unrooted native writer.
    /// Every other native frame on the stack holds its Values in traced
    /// storage: a binding, a `#:` slot or a [`RootFrame`]
    /// (tests/fixtures/reentry-sites.txt lists each call site).
    pub(crate) fn may_collect(&self, own: usize) -> bool {
        self.gc_holds == 0 && self.running_engines.saturating_sub(self.suspended.len()) == own
    }
    /// Set the suspended engine's register stack aside for a nested run,
    /// still traced, its allocation unmoved for the frames' raw pointers.
    pub(crate) fn suspend_stack(&mut self) {
        let stack = std::mem::take(&mut self.stack);
        self.suspended_stacks.push(stack);
    }
    pub(crate) fn resume_stack(&mut self) {
        debug_assert!(self.stack.is_empty(), "a nested run returned its registers");
        self.stack = self.suspended_stacks.pop().expect("a suspended register stack");
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
        let Stress { verify, moving, threads, inplace, .. } = self.gc_stress;
        #[cfg(test)]
        let (faults, buried) = (self.gc_stress.faults, self.gc_stress.buried);
        self.gc_stress = Stress { every, verify, moving, threads, inplace, ..Stress::default() };
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
    /// Make every collection compact, as stress collections do, or return
    /// to compacting on demand (`compacts').
    pub fn set_gc_moving(&mut self, moving: bool) {
        self.gc_stress.moving = moving;
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
    /// After a collection: no payload charged since, no poll pending, and the
    /// vector slots the live heap keeps, for the proportional rule.
    fn settle_payload(&mut self) {
        self.gc_payload_bytes = 0;
        self.collect_poll = false;
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
            ("symbols", size_of::<Slot>(), self.slots.len() - self.obarray_free_slots.len() - self.local_binding_slots.len(), Some(self.obarray_free_slots.len())),
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
    fn survives(&self, symbols: &HashSet<usize>, regions: &[bool], v: Value) -> bool {
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
            7 if v != Value::UNBOUND && self.obarray_symbol_names.contains_key(&(i - 1)) => symbols.contains(&(i - 1)),
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
    /// The cheap test of a collection's aftermath: doomed finalizers, or a
    /// collection whose `post-gc-hook' is not yet looked at. Most polls find
    /// neither.
    #[inline]
    pub(crate) fn aftermath_flagged(&self) -> bool {
        !self.doomed_finalizers.is_empty() || self.post_gc_due
    }
    /// A collection's aftermath waits: doomed finalizers (one may yet add to
    /// the hook), or a collection ended with a `post-gc-hook' to run. Cheap,
    /// for the places that make a poll come soon (a cleanup's end, an
    /// engine's): the hook is read once per collection, the flag clearing
    /// when it is nil (`post_gc_hook_due').
    pub(crate) fn aftermath_pending(&mut self) -> bool {
        !self.doomed_finalizers.is_empty() || self.post_gc_hook_due()
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
    /// Marking only grows, so the rounds reach a fixpoint; weak tables reached
    /// by the new marks join the next round. True when anything was pushed.
    fn weak_round(
        &self,
        weak: &[usize],
        symbols: &HashSet<usize>,
        regions: &[bool],
        pending: &mut Vec<Value>,
        local_live: &mut HashSet<usize>,
    ) -> bool {
        for &t in weak {
            let data::Object::HashTable(table) = &self.objects[t] else {
                unreachable!("weak table index");
            };
            for (_, key, value) in table.entries() {
                let (k, v) = (self.survives(symbols, regions, key), self.survives(symbols, regions, value));
                // The buffer-keyed ephemeron owns a local value indirectly
                // through a stable symbol-slot ID. The slot is NOT a global
                // root: trace its moving Value only once its owner is live.
                if k && table.weakness == hash::Weakness::Key {
                    if let Some(slot) = value.index_if(7).and_then(|i| i.checked_sub(1)) {
                        if self.local_binding_slots.contains(&slot) && local_live.insert(slot) {
                            pending.push(self.slots[slot].value);
                        }
                    }
                }
                if table.weakness.keep(k, v) {
                    if !k {
                        pending.push(key);
                    }
                    if !v {
                        pending.push(value);
                    }
                }
            }
        }
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
                        + t.len() * size_of::<(u32, romacs_regex_engine::SyntaxClass)>()
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
                            + t.len() * size_of::<(u32, romacs_regex_engine::SyntaxClass)>()
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
        std::mem::take(&mut self.allocation.compact) || (self.gc_stress.every > 0 && !self.gc_stress.inplace) || self.gc_stress.moving
    }
    /// A collection stopped: the tables hold no marks.
    pub(crate) fn clear_marks(&mut self) {
        self.conses.clear_marks();
        self.closures.clear_marks();
        self.cells.clear_marks();
        self.objects.clear_marks();
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
    pub(super) fn note_collection(&mut self, elapsed_ns: Option<u64>, idle: bool) {
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
        self.collection_log.records.push(CollectionRecord { idle, explicit, us, command, key, redisplay, at, objects, bytes });
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
    /// RECORD (KIND MS MET COMMAND T OBJECTS BYTES), KIND `idle`, `explicit` or
    /// `automatic`, MS a float, the pause in milliseconds to the microsecond,
    /// MET `command`, `key`, `redisplay` or nil, COMMAND a string or nil,
    /// T when it ended as `float-time` reads, or nil if nothing timed it,
    /// OBJECTS and BYTES what it left live (BYTES an estimate that does not walk).
    pub(crate) fn take_collections(&mut self) -> Value {
        let CollectionLog { records, dropped, .. } = std::mem::take(&mut self.collection_log);
        let mut result = Value::NIL;
        for record in records.into_iter().rev() {
            let kind = if record.idle { "idle" } else if record.explicit { "explicit" } else { "automatic" };
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
            let mut row = self.keymap_cons(size(record.bytes), Value::NIL);
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
    /// local marks). Whether it collected.
    pub(super) fn collect_marked(&mut self, roots: &mut [Value], until_key: bool) -> bool {
        // The one funnel: no collection while a hold is open or an engine runs
        // unsuspended, whatever reached here. Raw code cannot suspend its own
        // engine (only the engine's lends do), so it cannot make this pass.
        debug_assert!(self.suspended.len() <= self.running_engines, "suspended engines outnumber running ones");
        if !self.may_collect(0) {
            return false;
        }
        let mut lap = Lap::start();
        // Buffers move and local bindings go (`Vm::eval_limit_local').
        self.forwarded.local_epoch = self.forwarded.local_epoch.wrapping_add(1);
        // Patterns kept for a table name it by a Value this may move.
        self.clear_regexp_cache(true);
        self.mode_line_forms.clear();
        self.lambda_lists.clear();
        let before = self.heap_objects();
        // Finished nested runs' programs are caches: relowering what is used
        // again costs less than tracing, retiring and forwarding every copy.
        for program in &mut self.spare_programs {
            program.forget_code();
        }
        // Membership by slot, tested for every symbol and slot visited below.
        let in_obarray = self.slot_set(self.obarray_symbol_names.keys());
        let local_binding = self.slot_set(self.local_binding_slots.iter());
        lap.lap(&mut self.gc_times, Phase::Setup);
        let n = gc_mark::threads(self.gc_stress.threads);
        let Some(mut marked) = self.mark_heap(roots, &in_obarray, &local_binding, until_key, n, &mut lap) else { return false };
        if self.gc_stress.verify {
            if n > 1 {
                marked = self.verify_mark(marked, n, roots, &in_obarray, &local_binding);
            }
            self.check_reachable(roots, &marked.code_marks, &marked.state.region_marks, &marked.state.obarray_symbols_live);
        }
        let Marked { marking, mut own, doomed, local_live, code_marks, census, state } = marked;
        let MarkState { mut native_nodes, weak_tables, live_obarrays, obarray_symbols_live, region_marks } = state;
        // From here the collection changes the heap: the last point a key
        // press ends it.
        if until_key && self.host_key_pending() { return false; }
        lap.lap(&mut self.gc_times, Phase::Mark);
        self.gc_census = census;
        // A lexical pair's cell takes its value, which marking only read.
        let pairs: Vec<(usize, Value)> = self.cell_pairs.iter().filter(|&(&i, _)| self.cells.marked(i))
            .map(|(&i, &pair)| (i, self.cons_pair(pair).expect("live lexical pair").1)).collect();
        for (i, value) in pairs { self.cells.collector_entries_mut()[i] = value; }
        self.gc_times.entries = [self.conses.len(), self.closures.len(), self.cells.len(), self.objects.len()];
        if !doomed.is_empty() {
            let gone: HashSet<u64> = doomed.iter().map(|f| f.0).collect();
            self.finalizers.retain(|f| !gone.contains(&f.0));
            self.doomed_finalizers.extend(doomed.iter().copied());
        }
        self.gc_stats.allocations += before.saturating_sub(self.gc_retained) as u64;
        let pads = self.gc_stress.take_pads();
        // A throw's no-catch mark is weak: an error object nothing else
        // reaches is carried by no escape, so its mark goes.
        let thrown = std::mem::take(&mut self.thrown_no_catch);
        self.thrown_no_catch = thrown
            .into_iter()
            .filter(|&object| self.survives(&obarray_symbols_live, &region_marks, object))
            .collect();
        // GNU `sweep_weak_table (h, true)`: drop entries the rule rejects
        // before forwarding, which asserts that every rewritten Value is live.
        for &t in &weak_tables {
            let dead: Vec<_> = match &self.objects[t] {
                data::Object::HashTable(table) => table
                    .entries()
                    .filter(|&(_, key, value)| {
                        !(self.survives(&obarray_symbols_live, &region_marks, key)
                          && self.survives(&obarray_symbols_live, &region_marks, value))
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
        // The dead with side effects go here, on this thread, on either path:
        // markers leave their buffers, and the rest are dropped.
        // Both taken here, by the collection they were planted for.
        #[cfg(test)]
        let (before_scan, before_sweep) = (std::mem::take(&mut self.gc_stress.faults.allocate_before_scan), std::mem::take(&mut self.gc_stress.faults.allocate_before_sweep));
        #[cfg(test)]
        if before_scan {
            let _ = self.alloc_cons(Value::NIL, Value::NIL);
        }
        let dead = marking.dead_objects(self, &mut own);
        for &i in &dead {
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
        // Dead-owner entries have been swept. Discard only their unowned
        // slot values before forwarding; every live-owner and active-binding
        // value was marked above and is rewritten by the ordinary slot pass.
        self.free_local_binding_slots.clear();
        for &slot in &self.local_binding_slots {
            if !local_live.contains(&slot) {
                self.slots[slot].value = Value::UNBOUND;
                self.slots[slot].plist = Value::NIL;
                self.slots[slot].local = Value::NIL;
                self.free_local_binding_slots.push(slot);
            }
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
                t.reclaim_removed(&obarray_symbols_live);
            }
        }
        let compacting = self.compacts();
        lap.lap(&mut self.gc_times, Phase::Census);
        if !compacting {
            self.reclaim_obarray_symbols(&obarray_symbols_live);
            lap.lap(&mut self.gc_times, Phase::Obarray);
            let cells = &self.cells;
            self.cell_pairs.retain(|&i, _| cells.marked(i));
            for &i in &dead {
                self.objects.collector_entries_mut()[i as usize] = data::Object::Free;
            }
            let verify = self.gc_stress.verify;
            #[cfg(test)]
            if before_sweep {
                let _ = self.alloc_cons(Value::NIL, Value::NIL);
            }
            let swept = marking.sweep(self, &mut own);
            self.conses.swept(swept[0].0, swept[0].1, verify);
            self.closures.swept(swept[1].0, swept[1].1, verify);
            self.cells.swept(swept[2].0, swept[2].1, verify);
            self.objects.swept(swept[3].0, swept[3].1, verify);
            marking.finish(own);
            #[cfg(test)]
            { self.gc_stress.buried += marking.buried.load(std::sync::atomic::Ordering::Acquire); }
            self.referrers = None;
            self.local_owners = None;
            lap.lap(&mut self.gc_times, Phase::Sweep);
            self.shrink_tables();
            lap.lap(&mut self.gc_times, Phase::Shrink);
            self.settle_collection(before);
            lap.lap(&mut self.gc_times, Phase::Settle);
            return true;
        }
        // Compaction: every live entry moves down, every reference is
        // rewritten, and no entry is left free. Past here a panic leaves the
        // references half rewritten, which nothing can undo.
        // Stress fillers go only where live objects would move past them.
        let forwarded = |marks: &mut dyn Iterator<Item = bool>, pads: usize| {
            let (mut map, mut next, mut any) = (Vec::new(), pads, false);
            for live in marks {
                any |= live;
                map.push(if live { next += 1; next - 1 } else { usize::MAX });
            }
            (map, if any { pads } else { 0 })
        };
        let tables = [
            forwarded(&mut self.conses.marks(), pads),
            forwarded(&mut self.closures.marks(), pads),
            forwarded(&mut self.cells.marks(), pads),
            forwarded(&mut self.objects.marks(), pads),
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
        self.conses.collector_compact(|p| {
            rewrite(&mut p.0);
            rewrite(&mut p.1);
        });
        self.closures.collector_compact(|c| {
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
        self.cells.collector_compact(|v| rewrite(v));
        for v in roots
            .iter_mut()
            .chain(self.stack.iter_mut())
            .chain(self.suspended_stacks.iter_mut().flatten())
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
            if obarray_symbols_live.contains(slot) { rewrite(name); }
        }
        self.reclaim_obarray_symbols(&obarray_symbols_live);
        for name in self.permanent_symbol_names.values_mut() { rewrite(name); }
        for s in &mut self.slots {
            rewrite(&mut s.value);
            rewrite(&mut s.function);
            rewrite(&mut s.plist);
            rewrite(&mut s.local);
        }
        self.forwarded.visit_values(&mut |v| rewrite(v));
        for v in &mut self.standard_case_tables { rewrite(v); }
        rewrite(&mut self.command_point.1);
        for v in self.finalizers.iter_mut().chain(self.doomed_finalizers.iter_mut()) { rewrite(v); }
        self.charsets.rewrite(&rewrite);
        for (_, v, buffer, _) in &mut self.dynamic {
            rewrite(v);
            rewrite(buffer);
        }
        for c in &mut self.caches {
            rewrite(&mut c.target);
        }
        // Forward each physical native slot once, even when several live
        // cursors retain the same subtree (possibly detached from its table).
        native_nodes.clear();
        self.objects.collector_compact(|object| {
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
                    data::Object::Buffer(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::ByteBuffer(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::Marker(m) => m.visit_values(&mut |v| rewrite(v)),
                    data::Object::Overlay(o) => o.visit_values(&mut |v| rewrite(v)),
                    data::Object::String(s) | data::Object::RawString(s) => {
                        s.properties.visit_values(&mut |v| rewrite(v))
                    }
                    data::Object::HashTable(t) => t.forward(&mut |v| rewrite(v)),
                    data::Object::Obarray(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::CharTable(t) => t.collector_visit_values(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::SubCharTable(node) => char_table::collector_visit_node(node, &mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::MapKeymapCursor(c) => c.collector_visit_values(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
        });
        self.referrers = None;
        self.local_owners = None;
        for f in &mut self.functions {
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
        true
    }
    /// A collection's books, BEFORE being the objects it found.
    fn settle_collection(&mut self, before: usize) {
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
    /// Mark from ROOTS and everything the VM holds, on N threads: what it
    /// reached, or None when a key press ended it (UNTIL_KEY), its marks left
    /// for the next mark to clear. Writes nothing but colours and its own state.
    fn mark_heap(&mut self, roots: &[Value], in_obarray: &[bool], local_binding: &[bool], until_key: bool, n: usize, lap: &mut Lap) -> Option<Marked> {
        let keep = |v: &Value| traced(*v, in_obarray);
        let mut pending: Vec<Value> = roots.iter().copied().filter(keep).collect();
        pending.extend_from_slice(&self.external_roots);
        for engine in &mut self.suspended {
            engine.visit_state_values(&mut |v| if keep(v) { pending.push(*v) });
        }
        pending.extend(self.doomed_finalizers.iter().copied());
        pending.extend(self.permanent_symbol_names.values().copied());
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
        let mut local_live = HashSet::default();
        // GNU's C globals and `buffer_defaults`: roots outside every value
        // cell (forwarded.rs). Per-buffer values are traced with their buffer.
        self.forwarded.visit_values(&mut |v| if keep(v) { pending.push(*v) });
        pending.extend_from_slice(&self.standard_case_tables);
        pending.push(self.command_point.1);
        self.charsets.trace(&mut pending);
        for &(slot, old, buffer, _) in &self.dynamic {
            // Both saved and active values stay roots even if Lisp switches
            // away from their buffer during the dynamic extent.
            pending.extend([old, self.slots[slot].value, buffer, Value::reference(slot + 1, 7)]);
            if local_binding[slot] {
                local_live.insert(slot);
            }
        }
        let code_marks: Vec<std::sync::atomic::AtomicBool> = self.functions.iter().map(|_| false.into()).collect();
        let active = !self.stack.is_empty() || !self.suspended_stacks.is_empty();
        let mut code_pending: Vec<_> = self
            .request_code
            .live
            .iter()
            .enumerate()
            .filter_map(|(id, &live)| {
                (live && (active || self.request_code.permanent[id])).then_some(id)
            })
            .collect();
        self.source_code_roots(&mut code_pending);
        for (id, entry) in &self.conditions.entries {
            let custom = id.value().index_if(7).and_then(|n| n.checked_sub(1))
                .is_some_and(|slot| self.obarray_symbol_names.contains_key(&slot));
            if !custom {
                for &(key, value) in &entry.properties { pending.extend([key, value]); }
            }
        }
        lap.lap(&mut self.gc_times, Phase::Roots);
        let mut state = MarkState { region_marks: vec![false; self.regions.len()], ..MarkState::default() };
        state.region_marks[0] = true;
        // GNU dooms finalizers once, after the roots and before the weak
        // tables' contents are marked.
        let mut finalizers_doomed = false;
        let mut doomed = Vec::new();
        // SAFETY: from here to the sweep the collection grows, shrinks,
        // sweeps and re-marks no table but through MARKING (`verify_mark'
        // hands its own on), and its scan and sweep check that first.
        let marking = unsafe { gc_mark::Marking::new(self, n) };
        let mut own = gc_mark::Worker::default();
        // All a mark wants of the VM but to read it, while nothing else asks.
        let mut host = if until_key { self.host_input.take() } else { None };
        let this = &*self;
        let view = this.mark_view(in_obarray, local_binding, &code_marks);
        let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| marking.crew(&view, || {
            marking.publish_roots(&view, std::mem::take(&mut pending), std::mem::take(&mut code_pending));
            // Here while the threads start on the rest: suspended engines'
            // lowered code.
            for engine in &this.suspended {
                engine.code_values(&mut |v| if keep(&v) { pending.push(v) });
            }
            marking.add(&mut own, &mut pending);
            loop {
                let drained = marking.round(&view, &mut own, KEY_POLL,
                    &mut |handed, pending| this.mark_serial(handed, in_obarray, pending, &mut state),
                    &mut || host.as_mut().is_some_and(|host| host.key_pending()));
                if !drained { return false; }
                if !std::mem::replace(&mut finalizers_doomed, true) && this.doom_finalizers(&mut pending, &mut doomed) {
                    marking.add(&mut own, &mut pending);
                    continue;
                }
                if this.weak_round(&state.weak_tables, &state.obarray_symbols_live, &state.region_marks, &mut pending, &mut local_live) {
                    marking.add(&mut own, &mut pending);
                    continue;
                }
                return true;
            }
        })));
        // The host goes back however the mark ended: a session that lost it
        // would see no key at any later idle collection.
        if until_key { self.host_input = host; }
        let finished = finished.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if !finished { return None; }
        let census = marking.census(&own);
        let code_marks = code_marks.into_iter().map(|mark| mark.into_inner()).collect();
        Some(Marked { marking, own, doomed, local_live, code_marks, census, state })
    }
    /// `ROMACS_GC_VERIFY': mark again, alone, from nothing, and require the
    /// threads' answer: every colour, function, symbol and region, the doomed
    /// and the figures. A mark misses what a race loses, and a sweep then
    /// frees what is reachable; nothing else would show it.
    /// The re-mark's state goes on, its colours being these: the tables were
    /// last taken by it (`RawTable::holds'). It sweeps on the threads' N, so
    /// verify keeps the threaded sweep and its graves.
    fn verify_mark(&mut self, threads: Marked, n: usize, roots: &[Value], in_obarray: &[bool], local_binding: &[bool]) -> Marked {
        let colors = [self.conses.colors(), self.closures.colors(), self.cells.colors(), self.objects.colors()];
        self.clear_marks();
        let mut lap = Lap(None);
        let alone = self.mark_heap(roots, in_obarray, local_binding, false, 1, &mut lap).expect("no key ends it");
        let again = [self.conses.colors(), self.closures.colors(), self.cells.colors(), self.objects.colors()];
        for (table, (colors, again)) in ["conses", "closures", "cells", "objects"].iter().zip(colors.iter().zip(&again)) {
            if let Some(i) = colors.iter().zip(again).position(|(a, b)| a != b) {
                panic!("threads and one thread disagree on {table} entry {i}: {} against {}", colors[i], again[i]);
            }
        }
        assert_eq!(threads.code_marks, alone.code_marks, "threads and one thread reached different code");
        assert_eq!(threads.census, alone.census, "threads and one thread counted different objects");
        assert_eq!(threads.doomed, alone.doomed, "threads and one thread doomed different finalizers");
        assert_eq!(threads.local_live, alone.local_live, "threads and one thread kept different local bindings");
        assert_eq!(threads.state.obarray_symbols_live, alone.state.obarray_symbols_live, "threads and one thread kept different symbols");
        assert_eq!(threads.state.region_marks, alone.state.region_marks, "threads and one thread kept different regions");
        let sorted = |list: &[usize]| { let mut list = list.to_vec(); list.sort_unstable(); list };
        assert_eq!(sorted(&threads.state.weak_tables), sorted(&alone.state.weak_tables), "threads and one thread found different weak tables");
        assert_eq!(sorted(&threads.state.live_obarrays), sorted(&alone.state.live_obarrays), "threads and one thread found different obarrays");
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
                pending.extend([name, s.value, s.function, s.plist, s.local]);
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
        let native_nodes = &mut state.native_nodes;
        match handed.object() {
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
            data::Object::CharTable(t) => t.collector_visit_values(native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::SubCharTable(node) => char_table::collector_visit_node(node, native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::MapKeymapCursor(c) => c.collector_visit_values(native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Buffer(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::ByteBuffer(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Marker(m) => m.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::Overlay(o) => o.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::String(s) | data::Object::RawString(s) => s.properties.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::HashTable(t) if t.weakness == hash::Weakness::None => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
            data::Object::HashTable(_) => state.weak_tables.push(i),
            _ => unreachable!("a kind the workers trace"),
        }
    }
    /// The last completed collection: ((PHASE . SECONDS)...), then
    /// (marked CONSES CLOSURES CELLS OBJECTS) and (entries ...) likewise.
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
        let mut result = self.keymap_cons(entries, Value::NIL);
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
        let private = vm.slots.len();
        vm.slots.push(Slot::new());
        vm.symbol_names.push("private".into());
        vm.local_binding_slots.insert(private);
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
