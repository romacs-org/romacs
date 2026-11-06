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
/// order on every exit, the failing ones included. A panic unwinding the
/// owning frame drops the span.
#[must_use = "a root frame is released on every exit"]
pub(crate) struct RootFrame {
    base: usize,
    len: usize,
    vm: *mut Vm,
}

impl Drop for RootFrame {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // SAFETY: as for `UnwindMark`.
            unsafe { (*self.vm).external_roots.truncate(self.base) };
        }
    }
}

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
    /// (`sweep_tables'), so a stale reference to it is seen, not hidden by
    /// the entry's next tenant.
    pub(crate) verify: bool,
    /// Every collection compacts ([`Vm::set_gc_moving`]): the fixtures that
    /// prove rooting under the moving collector.
    moving: bool,
}
romacs_snapshot::snapshot_struct!(Stress { every, entries, pads, shift, verify, moving });

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
    pub(crate) ms: u32,
    /// The command it ran in or at the end of, "anonymous" for a closure: a
    /// key ended the last idleness and nothing has begun another.
    pub(crate) command: Option<String>,
    /// Outside a command, a key press was queued when it ended.
    pub(crate) key: bool,
    /// In a prompt's redisplay, the host taken for the read: no key can be
    /// seen, and one pressed waits.
    pub(crate) redisplay: bool,
}
romacs_snapshot::snapshot_struct!(CollectionRecord { idle, explicit, ms, command, key, redisplay });

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

/// The tables' free entries (design note §4.4, S3.1): collections leave
/// every live entry where it is and list the others here, one stack per
/// table with the lowest index on top, which the allocation funnels pop
/// (`Vm::alloc_cons' and its siblings, data.rs). A compacting collection
/// empties them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FreeEntries {
    pub(crate) conses: Vec<u32>,
    pub(crate) closures: Vec<u32>,
    pub(crate) cells: Vec<u32>,
    pub(crate) objects: Vec<u32>,
    /// Entries `ROMACS_GC_VERIFY' freed, per table (conses, closures,
    /// cells, objects): poisoned and listed nowhere, but free all the same.
    quarantined: [usize; 4],
    /// Bytes the funnels have handed out, ever: the memory profiler's
    /// meter (profiler.rs), which table growth no longer measures.
    pub(crate) allocated: u64,
    /// The next collection compacts (`Vm::collect_compacting').
    pub(crate) compact: bool,
    /// Collections a due compaction has waited for an idle one.
    deferred: u8,
}
romacs_snapshot::snapshot_struct!(FreeEntries { conses, closures, cells, objects, quarantined, allocated, compact, deferred });

impl FreeEntries {
    /// Free entries, in all four tables.
    pub(crate) fn len(&self) -> usize {
        (0..4).map(|table| self.of(table)).sum()
    }
    /// Free entries of TABLE: 0 conses, 1 closures, 2 cells, 3 objects.
    pub(crate) fn of(&self, table: usize) -> usize {
        let listed = [&self.conses, &self.closures, &self.cells, &self.objects][table].len();
        listed + self.quarantined[table]
    }
    /// Empty, as a compaction leaves them (`shrink' then releases storage).
    fn clear(&mut self) {
        for list in [&mut self.conses, &mut self.closures, &mut self.cells, &mut self.objects] {
            list.clear();
        }
        self.quarantined = [0; 4];
    }
    /// Release grossly oversized lists: an empty one keeps no storage.
    fn shrink(&mut self) {
        for list in [&mut self.conses, &mut self.closures, &mut self.cells, &mut self.objects] {
            list.shrink_to(list.len().saturating_mul(2));
        }
    }
    /// The lists' storage, in bytes.
    fn capacity_bytes(&self) -> usize {
        [&self.conses, &self.closures, &self.cells, &self.objects].iter().map(|list| list.capacity() * size_of::<u32>()).sum()
    }
}

/// One table's part of `Vm::sweep_tables': cut TABLE after its last entry
/// MARKS keeps (unless VERIFY), visit each remaining entry with whether it
/// is live, and rebuild LIST, the table's free entries, in descending order
/// (unless VERIFY, which quarantines them). The entries quarantined.
fn sweep_table<T>(table: &mut Vec<T>, marks: &[bool], list: &mut Vec<u32>, verify: bool, mut visit: impl FnMut(&mut T, bool)) -> usize {
    list.clear();
    let end = if verify { marks.len() } else { marks.iter().rposition(|&live| live).map_or(0, |last| last + 1) };
    table.truncate(end);
    let mut quarantined = 0;
    for i in (0..end).rev() {
        let live = marks[i];
        visit(&mut table[i], live);
        match (live, verify) {
            (true, _) => {}
            (false, false) => list.push(i as u32),
            (false, true) => quarantined += 1,
        }
    }
    quarantined
}

/// What a freed cons or cell holds under `ROMACS_GC_VERIFY': a primitive
/// index no table has, which no operation accepts.
const POISON: Value = Value(((u32::MAX as u64) << 3) | 6);

/// The state nested Lisp runs push and pop on the VM: suspended engines and
/// their register stacks, root frames, collection holds, the running-engine
/// count, handler and backtrace marks. A panic unwinding a frame that took
/// this mark restores it, so no entry outlives the engine or frame it
/// describes (a suspended engine's code pointers would dangle) and the VM
/// stays usable once the panic is caught. Dynamic bindings stay as the
/// panic left them.
#[must_use = "the mark restores on unwinding only while it lives"]
pub(crate) struct UnwindMark {
    vm: *mut Vm,
    code: usize,
    stacks: usize,
    roots: usize,
    engines: usize,
    holds: usize,
    rooted_write: bool,
    stack_empty: bool,
    handlers: usize,
    backtrace: usize,
    eval_depth: usize,
}

impl Drop for UnwindMark {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        // SAFETY: the mark lives in a frame of the borrow it was taken from,
        // and an unwinding frame no longer uses that borrow.
        let vm = unsafe { &mut *self.vm };
        vm.suspended_code.truncate(self.code);
        vm.suspended_stacks.truncate(self.stacks);
        vm.external_roots.truncate(self.roots);
        vm.running_engines = self.engines;
        vm.gc_holds = self.holds;
        vm.rooted_write = self.rooted_write;
        if self.stack_empty {
            vm.stack = Vec::new();
        }
        vm.signal_handlers.truncate(self.handlers);
        vm.backtrace.leave(self.backtrace);
        vm.eval_depth = self.eval_depth;
    }
}

/// Bytes charged per heap object against `gc-cons-threshold': GNU's cons.
const CONS_BYTES: i64 = 16;
/// GNU's floor under `gc-cons-threshold' (alloc.c:5747, GC_DEFAULT_THRESHOLD / 10).
const MIN_CONS_THRESHOLD: i64 = 80_000;
/// A Romacs collection walks compiled code, and some compact, so it costs
/// more per traced unit than GNU's: `gc-cons-percentage' is scaled by this
/// before use, making GNU's 0.1 collect after about eleven traced heaps
/// (DIVERGENCES.md VM2-GC-PACING has the measured candidates).
const PERCENTAGE_SCALE: f64 = 17.0;
/// Unused free entries' bytes below which no table compacts for size: a
/// table of small entries (cells) is not worth moving the heap for.
const OUTGROWN_BYTES: usize = 8 << 20;
/// Whether the mark must visit V: a heap reference, or a symbol interned in
/// an obarray of its own, the only symbols a collection reclaims. Anything
/// else the mark would pop and discard.
#[inline(always)]
fn traced(v: Value, in_obarray: &[bool]) -> bool {
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
/// The scaled percentage's ceiling. GNU's formula never collects at 2 (it
/// counts half the allocation since as live); 1.8 holds a large setting at
/// 18 traced heaps, about 1.6 times the default's deferral.
const MAX_SCALED_PERCENTAGE: f64 = 1.8;

/// `gc-cons-percentage' P as the collector uses it: P times
/// PERCENTAGE_SCALE, at most MAX_SCALED_PERCENTAGE; a negative or NaN P
/// leaves only the threshold, as GNU's does.
pub(crate) fn scaled_percentage(p: f64) -> f64 {
    if p.is_nan() { 0.0 } else { (p * PERCENTAGE_SCALE).clamp(0.0, MAX_SCALED_PERCENTAGE) }
}

impl Stress {
    pub(crate) fn from_env() -> Self {
        let every = std::env::var("ROMACS_GC_STRESS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
        let verify = std::env::var_os("ROMACS_GC_VERIFY").is_some_and(|v| !v.is_empty() && v != "0");
        Self { every, verify, ..Self::default() }
    }
    /// This process's test mode, from its environment, for a VM a world
    /// carried here: the fillers the last collection left stay counted.
    pub(crate) fn rearm(&mut self) {
        let fresh = Self::from_env();
        (self.every, self.entries, self.verify) = (fresh.every, fresh.entries, fresh.verify);
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
    /// Mark the nested-run state for restoration should a panic unwind the
    /// caller's frame ([`UnwindMark`]).
    pub(crate) fn unwind_mark(&mut self) -> UnwindMark {
        UnwindMark {
            code: self.suspended_code.len(),
            stacks: self.suspended_stacks.len(),
            roots: self.external_roots.len(),
            engines: self.running_engines,
            holds: self.gc_holds,
            rooted_write: self.rooted_write,
            stack_empty: self.stack.is_empty(),
            handlers: self.signal_handlers.len(),
            backtrace: self.backtrace.frames.len(),
            eval_depth: self.eval_depth,
            vm: self,
        }
    }
    pub(crate) fn root_frame(&mut self, values: &[Value]) -> RootFrame {
        let base = self.external_roots.len();
        self.external_roots.extend_from_slice(values);
        RootFrame { base, len: values.len(), vm: self }
    }
    pub(crate) fn rooted(&self, frame: &RootFrame, i: usize) -> Value {
        assert!(i < frame.len, "root frame index");
        self.external_roots[frame.base + i]
    }
    pub(crate) fn set_rooted(&mut self, frame: &RootFrame, i: usize, value: Value) {
        assert!(i < frame.len, "root frame index");
        self.external_roots[frame.base + i] = value;
    }
    /// Release FRAME, returning its forwarded values.
    pub(crate) fn release_frame<const N: usize>(&mut self, frame: RootFrame) -> [Value; N] {
        assert_eq!(frame.len, N, "root frames are released whole");
        let values = std::array::from_fn(|i| self.rooted(&frame, i));
        self.drop_frame(frame);
        values
    }
    pub(crate) fn drop_frame(&mut self, frame: RootFrame) {
        assert_eq!(self.external_roots.len(), frame.base + frame.len, "root frames are released in reverse order");
        self.external_roots.truncate(frame.base);
    }
    /// GNU's `garbage_collection_inhibited`, derived from its owners: a
    /// collection may run only when no frame keeps heap Values in untraced
    /// Rust storage. Those are OWN unsuspended engines (the caller's own),
    /// no other running engine outside `suspended_code`, no open compiler
    /// builder and no watcher notification from an unrooted native writer.
    /// Every other native frame on the stack holds its Values in traced
    /// storage: a binding, a `#:` slot or a [`RootFrame`]
    /// (tests/fixtures/reentry-sites.txt lists each call site).
    pub(crate) fn may_collect(&self, own: usize) -> bool {
        self.gc_holds == 0 && self.running_engines.saturating_sub(self.suspended_code.len()) == own
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
        let Stress { verify, moving, .. } = self.gc_stress;
        self.gc_stress = Stress { every, verify, moving, ..Stress::default() };
    }
    /// Turn `ROMACS_GC_VERIFY' on or off, for a fixture that needs free
    /// entries reused (or quarantined).
    #[cfg(test)]
    pub(crate) fn set_gc_verify(&mut self, verify: bool) {
        self.gc_stress.verify = verify;
    }
    /// Make every collection compact, as stress collections do, or return
    /// to compacting on demand (`compacts').
    pub fn set_gc_moving(&mut self, moving: bool) {
        self.gc_stress.moving = moving;
    }
    /// GNU `maybe_garbage_collect' (alloc.c:5740-5809): collect once the
    /// allocation since the last collection reaches `gc-cons-threshold'
    /// (at least 80000 bytes) or, when `gc-cons-percentage' is a float, that
    /// fraction (scaled, PERCENTAGE_SCALE, and clamped, MAX_SCALED_PERCENTAGE)
    /// of the live heap plus half the allocation since. Allocation is counted
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
    /// the next automatic one due (GNU's `gc_threshold'). The proportional
    /// rule `A >= S (L + A/2)' over the live heap L is `A >= S L / (1 - S/2)',
    /// S being below 2.
    pub(crate) fn collection_budget(&self) -> (f64, f64) {
        let allocated = self.heap_objects().saturating_sub(self.gc_retained) as f64
            + (self.gc_payload_bytes / CONS_BYTES as usize) as f64;
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
            let s = scaled_percentage(p);
            // A cons, the unit, traces two slots.
            let live = self.gc_retained + self.gc_code + self.gc_retained_slots / 2;
            s * live as f64 / (1.0 - s / 2.0)
        });
        (allocated, threshold.max(proportional))
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
        self.gc_retained_slots = self.objects.iter().map(crate::data::traced_slots).sum();
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
        let (mut strings, mut string_bytes, mut vectors, mut vector_slots, mut vector_free, mut floats, mut buffers) = (0, 0, 0, 0, 0, 0, 0);
        for object in &self.objects {
            match object {
                data::Object::String(s) | data::Object::RawString(s) => {
                    strings += 1;
                    string_bytes += s.bytes().len();
                }
                data::Object::Vector(v) | data::Object::Record(v) => {
                    vectors += 1;
                    vector_slots += v.len();
                    vector_free += v.capacity() - v.len();
                }
                data::Object::Float(_) => floats += 1,
                data::Object::Buffer(_) | data::Object::ByteBuffer(_) => buffers += 1,
                _ => {}
            }
        }
        let rows: [(&str, usize, usize, Option<usize>); 8] = [
            ("conses", size_of::<(Value, Value)>(), self.conses.len() - self.free.of(0), Some(self.conses.capacity() - self.conses.len() + self.free.of(0))),
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
    fn survives(&self, marks: &[Vec<bool>; 4], symbols: &HashSet<usize>, regions: &[bool], v: Value) -> bool {
        if Self::image_value(v) {
            let id = crate::frozen::region_of(v.payload() as usize);
            return id == 0 || regions.get(id).copied().unwrap_or(false);
        }
        let i = v.payload() as usize;
        match v.tag() {
            0 if v != Value::NIL => marks[3][i - 1],
            3 => marks[0][i],
            4 => marks[1][i],
            5 => marks[2][i],
            7 if v != Value::UNBOUND && self.obarray_symbol_names.contains_key(&(i - 1)) => symbols.contains(&(i - 1)),
            _ => true,
        }
    }
    /// `romacs--run-finalizers' when a collection has doomed finalizers
    /// and the prelude defines it.
    pub(crate) fn finalizer_runner(&mut self) -> Option<Value> {
        if self.doomed_finalizers.is_empty() { return None; }
        let runner = Value::reference(self.slot("romacs--run-finalizers") + 1, 7);
        let slot = runner.payload() as usize - 1;
        (self.slots[slot].function != Value::NIL && self.slots[slot].function != Value::UNBOUND).then_some(runner)
    }
    /// alloc.c queue_doomed_finalizers and mark_finalizer_list: the
    /// finalizers marking has not reached are marked, with their functions,
    /// to survive until they run, and go to DOOMED, which the collection
    /// moves to the doomed queue once it can no longer give way to a key
    /// (`collect_marked'). True when any was doomed.
    fn doom_finalizers(&self, marks: &[Vec<bool>; 4], pending: &mut Vec<Value>, doomed: &mut Vec<Value>) -> bool {
        let found: Vec<Value> = self.finalizers.iter().copied()
            .filter(|&f| !(Self::image_value(f) || marks[3][f.payload() as usize - 1]))
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
        marks: &[Vec<bool>; 4],
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
                let (k, v) = (self.survives(marks, symbols, regions, key), self.survives(marks, symbols, regions, value));
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
        s.live_bytes = (self.conses.len() - self.free.of(0)) * std::mem::size_of::<(Value, Value)>()
            + (self.cells.len() - self.free.of(2)) * std::mem::size_of::<Value>()
            + (self.closures.len() - self.free.of(1)) * std::mem::size_of::<Closure>()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.len() + c.metadata.len()) * 8)
                .sum::<usize>();
        // Native nodes may be retained by both tables and suspended cursors.
        // Keep this accounting separate from the moving arena object counts.
        let mut native_nodes = HashSet::default();
        s.live_bytes += (self.objects.len() - self.free.of(3)) * std::mem::size_of::<data::Object>()
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
                    data::Object::Finalizer(_) | data::Object::Free => 0,
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
        s.capacity_bytes = self.free.capacity_bytes()
            + self.conses.capacity() * 16
            + self.cells.capacity() * 8
            + self.closures.capacity() * std::mem::size_of::<Closure>()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.capacity() + c.metadata.capacity()) * 8)
                .sum::<usize>();
        native_nodes.clear();
        s.capacity_bytes += self.objects.capacity() * std::mem::size_of::<data::Object>()
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
                    data::Object::Finalizer(_) | data::Object::Free => 0,
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
    /// and rewriting every reference: when asked (`collect_compacting'),
    /// always under the stress and moving modes, and once a table is
    /// oversized, at an idle collection (IDLE) or after two collections
    /// waiting for one. Every other collection frees in place
    /// (`sweep_tables'). Oversized: the free entries the last sweep left
    /// that allocation has not taken since, more than four times the
    /// table's live entries (COUNTS) and OUTGROWN_BYTES. Garbage a steady
    /// allocator makes between collections takes its entries back each
    /// time; a collapsed burst leaves them idle.
    fn compacts(&mut self, counts: &[usize; 4], idle: bool) -> bool {
        let entries = [size_of::<(Value, Value)>(), size_of::<Closure>(), size_of::<Value>(), size_of::<data::Object>()];
        let outgrown = (0..4).any(|table| {
            let unused = self.free.of(table);
            unused > counts[table].saturating_mul(4) && unused.saturating_mul(entries[table]) > OUTGROWN_BYTES
        });
        let due = outgrown && (idle || self.free.deferred >= 2);
        self.free.deferred = if outgrown && !due { self.free.deferred.saturating_add(1) } else { 0 };
        std::mem::take(&mut self.free.compact) || self.gc_stress.every > 0 || self.gc_stress.moving || due
    }
    /// Free every entry MARKS leaves unmarked, in place: a cons becomes
    /// (nil . nil), a cell nil, a closure one naming no function, an object
    /// `Object::Free', its payload dropped. Each table is first cut after
    /// its last marked entry, then its free list is rebuilt from the marks,
    /// lowest index on top. Under `ROMACS_GC_VERIFY' freed entries are
    /// poisoned instead and quarantined: never cut, never reused. A live
    /// obarray drops the dead symbols LIVE_SYMBOLS leaves out.
    fn sweep_tables(&mut self, marks: &[Vec<bool>; 4], live_symbols: &HashSet<usize>) {
        let verify = self.gc_stress.verify;
        let dead = if verify { POISON } else { Value::NIL };
        let free = &mut self.free;
        free.quarantined = [
            sweep_table(&mut self.conses, &marks[0], &mut free.conses, verify, |pair, live| if !live { *pair = (dead, dead) }),
            sweep_table(&mut self.closures, &marks[1], &mut free.closures, verify, |closure, live| {
                if !live {
                    *closure = Closure { function: usize::MAX, captures: vec![], metadata: vec![], document: dead };
                }
            }),
            sweep_table(&mut self.cells, &marks[2], &mut free.cells, verify, |cell, live| if !live { *cell = dead }),
            sweep_table(&mut self.objects, &marks[3], &mut free.objects, verify, |object, live| match object {
                _ if !live => *object = data::Object::Free,
                data::Object::Obarray(t) => t.reclaim_removed(live_symbols),
                _ => {}
            }),
        ];
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
        self.objects.shrink_to(self.objects.len().saturating_mul(2).max(1024));
        self.conses.shrink_to(self.conses.len().saturating_mul(2).max(16384));
        self.cells.shrink_to(self.cells.len().saturating_mul(2).max(1024));
        self.closures.shrink_to(self.closures.len().saturating_mul(2).max(1024));
        self.free.shrink();
    }
    /// Keep a finished collection's record, ELAPSED_NS long, IDLE as in
    /// `collect_until_key'. One look at the host and one at the command
    /// variable, per collection: a key pays nothing.
    pub(super) fn note_collection(&mut self, elapsed_ns: u64, idle: bool) {
        let explicit = std::mem::take(&mut self.collection_log.explicit);
        if self.collection_log.records.len() >= COLLECTION_RECORDS {
            self.collection_log.dropped += 1;
            return;
        }
        let command = if self.idle_since.is_none() && !idle { self.command_name() } else { None };
        let redisplay = command.is_none() && self.idle_since.is_some() && self.host_input.is_none();
        let key = command.is_none() && self.host_key_pending();
        let ms = (elapsed_ns / 1_000_000).min(u32::MAX as u64) as u32;
        self.collection_log.records.push(CollectionRecord { idle, explicit, ms, command, key, redisplay });
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
    /// RECORD (KIND MS MET COMMAND), KIND `idle', `explicit' or `automatic',
    /// MET `command', `key', `redisplay' or nil, COMMAND a string or nil.
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
            let ms = Value::integer(record.ms as i64).expect("milliseconds fit fixnum");
            let mut row = self.keymap_cons(command, Value::NIL);
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
        // Buffers move and local bindings go (`Vm::eval_limit_local').
        self.forwarded.local_epoch = self.forwarded.local_epoch.wrapping_add(1);
        // Patterns kept for a table name it by a Value this may move.
        self.clear_regexp_cache(true);
        self.mode_line_forms.clear();
        self.lambda_lists.clear();
        let before = self.heap_objects();
        let private_before = self.private_objects();
        // Frozen regions reached by marking (`frozen.rs`); region 0 is the
        // executable image, always live.
        let mut region_marks = vec![false; self.regions.len()];
        region_marks[0] = true;
        let mut marks = [
            vec![false; self.conses.len()],
            vec![false; self.closures.len()],
            vec![false; self.cells.len()],
            vec![false; self.objects.len()],
        ];
        // Finished nested runs' programs are caches: relowering what is used
        // again costs less than tracing, retiring and forwarding every copy.
        for program in &mut self.spare_programs {
            program.forget_code();
        }
        // Membership by slot, tested for every symbol and slot visited below.
        let in_obarray = self.slot_set(self.obarray_symbol_names.keys());
        let keep = |v: &Value| traced(*v, &in_obarray);
        let mut pending: Vec<Value> = roots.iter().copied().filter(keep).collect();
        pending.extend(self.stack.iter().copied().filter(keep));
        pending.extend(self.suspended_stacks.iter().flatten().copied().filter(keep));
        pending.extend_from_slice(&self.external_roots);
        pending.extend_from_slice(&self.doomed_finalizers);
        for code in &self.suspended_code {
            // SAFETY: each entry's engine is suspended in a nested call.
            unsafe { code.visit_values(&mut |v| if keep(v) { pending.push(*v) }) };
        }
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
        let mut obarray_symbols_live = HashSet::default();
        let local_binding = self.slot_set(self.local_binding_slots.iter());
        for (slot, s) in self.slots.iter().enumerate() {
            if in_obarray[slot] { continue; }
            if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                pending.push(Value::reference(target + 1, 7));
            }
            if keep(&s.function) { pending.push(s.function); }
            // GNU alloc.c:6673 marks `u.s.plist` with the symbol. VM2 symbols
            // are never reclaimed (DIVERGENCES VM2-BOOT4), so a plist and a
            // buffer-local table are roots for every slot, interned or not.
            if keep(&s.plist) { pending.push(s.plist); }
            if keep(&s.local) { pending.push(s.local); }
            if !local_binding[slot] && keep(&s.value) {
                pending.push(s.value);
            }
        }
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
        let mut code_marks = vec![false; self.functions.len()];
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
        // Entries marked, per table: conses, closures, cells, objects.
        let mut counts = [0usize; 4];
        let mut native_nodes = HashSet::default();
        // Weak hash tables reached by marking; their entries are not traced
        // until `weak_round` (the ephemeron fixpoint) decides which survive.
        let mut weak_tables = vec![];
        // GNU dooms finalizers once, after the roots and before the weak
        // tables' contents are marked.
        let mut finalizers_doomed = false;
        let mut doomed = Vec::new();
        let mut polled = 0usize;
        while !pending.is_empty()
            || !code_pending.is_empty()
            || (!std::mem::replace(&mut finalizers_doomed, true) && self.doom_finalizers(&marks, &mut pending, &mut doomed))
            || self.weak_round(&marks, &weak_tables, &obarray_symbols_live, &region_marks, &mut pending, &mut local_live)
        {
            while let Some(id) = code_pending.pop() {
                // Some low-level arena tests construct capture-only closures.
                if id >= code_marks.len() || std::mem::replace(&mut code_marks[id], true) {
                    continue;
                }
                // R215: a lazily held body is reachable exactly when its
                // function is, which may be through a closure alone.
                if let Some(lazy) = &self.functions[id].lazy {
                    pending.push(lazy.body);
                    for name in &lazy.names {
                        if let Name::Generated(id) = name {
                            if let Some(&slot) = self.generated_symbols.get(id) { pending.push(Value::reference(slot + 1, 7)); }
                        }
                    }
                }
                // A pending compiled-Lisp body: what its code will name.
                if let Some(body) = &self.functions[id].pending {
                    pending.extend(body.values.iter().copied());
                    code_pending.extend(body.children.iter().copied());
                }
                // The lambda list, which slot 0 answers, outlives the lazy body.
                let function = &self.functions[id];
                let mut nested = Vec::new();
                for form in function.lazy.iter().flat_map(|lazy| &lazy.args).chain(function.params.iter().flatten()) {
                    let mut next = Some(form);
                    while let Some(form) = next.take().or_else(|| nested.pop()) {
                        match form {
                            Form::Generated(id) => if let Some(&slot) = self.generated_symbols.get(id) { pending.push(Value::reference(slot + 1, 7)); },
                            Form::List(items) | Form::Vector(items) => nested.extend(items),
                            Form::Dotted(items, tail) => { nested.extend(items); nested.push(tail); }
                            _ => {}
                        }
                    }
                }
                for op in &self.functions[id].code {
                    let mut op = *op;
                    op.visit_values(&mut |v| if keep(v) { pending.push(*v) });
                    // Only a symbol interned in an obarray of its own can die.
                    let symbol = |slot: usize| in_obarray[slot].then(|| Value::reference(slot + 1, 7));
                    match op {
                        Op::Global(_, slot) | Op::SetGlobal(slot, _) | Op::Bind(slot, _) | Op::DeclareSpecial(slot)
                        | Op::BindLate(slot, _) | Op::ReadLate(_, _, slot) | Op::WriteLate(_, _, slot)
                        | Op::Define(slot, _) | Op::DefineMacro(slot, _)
                        | Op::Sealed(_, slot, _, _) => pending.extend(symbol(slot)),
                        Op::Closure(_, child, _) => code_pending.push(child),
                        Op::Direct(_, slot, cache, _, _) => {
                            pending.extend(symbol(slot));
                            pending.push(self.caches[cache].target);
                        }
                        _ => {}
                    }
                }
                for program in std::iter::once(&self.execution).chain(&self.spare_programs) {
                    program.trace_function(id, &mut pending, &mut code_pending);
                }
            }
            let Some(mut v) = pending.pop() else { continue };
            polled += 1;
            if until_key && polled % KEY_POLL == 0 && self.host_key_pending() {
                return false;
            }
            // Walk the cdr spine directly: a list is the common editor graph,
            // not two worklist pushes and a generic type dispatch per cons.
            while v.tag() == 3 && !Self::image_value(v) {
                let i = v.payload() as usize;
                if std::mem::replace(&mut marks[0][i], true) {
                    break;
                }
                counts[0] += 1;
                let (a, b) = self.conses[i];
                if keep(&a) {
                    pending.push(a);
                }
                v = b;
            }
            let tag = v.tag();
            if tag == 7 && v != Value::UNBOUND {
                let slot = v.payload() as usize - 1;
                if in_obarray.get(slot).copied().unwrap_or(false) && let Some(&name) = self.obarray_symbol_names.get(&slot) {
                    if obarray_symbols_live.insert(slot) {
                        let s = &self.slots[slot];
                        pending.extend([name, s.value, s.function, s.plist, s.local]);
                        if let Some(entry) = self.conditions.entries.get(&conditions::SymbolId::new(v).unwrap()) {
                            for &(key, value) in &entry.properties { pending.extend([key, value]); }
                        }
                        if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                            pending.push(Value::reference(target + 1, 7));
                        }
                    }
                }
                continue;
            }
            // A frozen value marks its region, whose symbols are then roots;
            // the region's own values are never traced (`frozen.rs`).
            if Self::image_value(v) {
                let id = crate::frozen::region_of(v.payload() as usize);
                if id != 0 && !std::mem::replace(&mut region_marks[id], true) {
                    pending.extend_from_slice(self.region_symbols(id));
                }
                continue;
            }
            let arena = match tag {
                // Mapped objects are untraced by contract: the collector must
                // never read an image page (PROT_NONE proofs assert it). Tag-3
                // image conses already fall through to `continue`; tag-0 image
                // STRINGS did not, and indexed marks[3] with the image bit as a
                // heap index. Neither lane could see this alone - mapped strings
                // did not exist when this walk was written.
                0 if v != Value::NIL && !Self::image_value(v) => 3,
                4 | 5 => (tag - 3) as usize,
                _ => continue,
            };
            let i = v.payload() as usize - usize::from(tag == 0);
            let mark = &mut marks[arena][i];
            if std::mem::replace(mark, true) {
                continue;
            }
            counts[arena] += 1;
            if tag == 0 {
                match &mut self.objects[i] {
                    data::Object::Window(w) => w.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Obarray(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Frame(f) => f.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Terminal(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::WindowConfiguration(c) => c.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Sqlite(_) | data::Object::Free => {}
                    data::Object::Treesit(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Finalizer(function) => pending.push(*function),
                    data::Object::Process(p) => p.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Vector(values) | data::Object::Record(values) => pending.extend(values.iter().copied().filter(keep)),
                    data::Object::CharTable(t) => t.visit_values_seen(&mut native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::SubCharTable(node) => char_table::visit_char_table_node_values(node, &mut native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::MapKeymapCursor(c) => c.visit_values(&mut native_nodes, &mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Buffer(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::ByteBuffer(t) => t.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Marker(m) => m.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::Overlay(o) => o.visit_values(&mut |v| if keep(v) { pending.push(*v) }),
                    data::Object::String(s) | data::Object::RawString(s) => {
                        s.properties.visit_values(&mut |v| if keep(v) { pending.push(*v) })
                    }
                    data::Object::HashTable(t) if t.weakness == hash::Weakness::None => {
                        t.visit_values(&mut |v| if keep(v) { pending.push(*v) })
                    }
                    data::Object::HashTable(_) => weak_tables.push(i),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
            } else if tag == 4 {
                pending.extend(self.closures[i].captures.iter().copied().filter(keep));
                pending.extend_from_slice(&self.closures[i].metadata);
                pending.push(self.closures[i].document);
                code_pending.push(self.closures[i].function);
            } else {
                if let Some(&pair) = self.cell_pairs.get(&i) {
                    self.cells[i] = self.cons_pair(pair).expect("live lexical pair").1;
                    pending.push(pair);
                } else {
                    pending.push(self.cells[i]);
                }
            }
        }
        // From here the collection changes the heap: the last point a key
        // press ends it.
        if until_key && self.host_key_pending() {
            return false;
        }
        if !doomed.is_empty() {
            let gone: HashSet<u64> = doomed.iter().map(|f| f.0).collect();
            self.finalizers.retain(|f| !gone.contains(&f.0));
            self.doomed_finalizers.extend_from_slice(&doomed);
        }
        self.gc_stats.allocations += before.saturating_sub(self.gc_retained) as u64;
        let pads = self.gc_stress.take_pads();
        // A throw's no-catch mark is weak: an error object nothing else
        // reaches is carried by no escape, so its mark goes.
        let thrown = std::mem::take(&mut self.thrown_no_catch);
        self.thrown_no_catch = thrown
            .into_iter()
            .filter(|&object| self.survives(&marks, &obarray_symbols_live, &region_marks, object))
            .collect();
        // GNU `sweep_weak_table (h, true)`: drop entries the rule rejects
        // before forwarding, which asserts that every rewritten Value is live.
        for &t in &weak_tables {
            let dead: Vec<_> = match &self.objects[t] {
                data::Object::HashTable(table) => table
                    .entries()
                    .filter(|&(_, key, value)| {
                        !(self.survives(&marks, &obarray_symbols_live, &region_marks, key)
                          && self.survives(&marks, &obarray_symbols_live, &region_marks, value))
                    })
                    .map(|(slot, _, _)| slot)
                    .collect(),
                _ => unreachable!("weak table index"),
            };
            if let data::Object::HashTable(table) = &mut self.objects[t] {
                for slot in dead {
                    table.sweep(slot);
                }
                table.compact_local_bindings();
            }
        }
        for (i, &live) in marks[3].iter().enumerate() {
            if live {
                continue;
            }
            let data::Object::Marker(m) = &self.objects[i] else {
                continue;
            };
            let (buffer, anchor) = (m.buffer, m.anchor);
            let Some(anchor) = anchor else { continue };
            let Ok(index) = buffer.index(0) else { continue };
            match self.objects.get_mut(index.saturating_sub(1)) {
                Some(data::Object::Buffer(b)) => { b.anchors.remove(anchor); }
                Some(data::Object::ByteBuffer(b)) => { b.anchors.remove(anchor); }
                _ => {}
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
        self.retire_code(&code_marks);
        // The code this collection traced, for the next one's pacing.
        self.gc_code = code_marks.iter().zip(&self.functions).filter(|&(&live, _)| live).map(|(_, f)| f.code.len()).sum::<usize>()
            + self.execution.code_len()
            // SAFETY: as in the mark phase.
            + self.suspended_code.iter().map(|code| unsafe { code.code_len() }).sum::<usize>();
        // Only a collection with no request running retires live request
        // code; one inside an engine must not postpone the next of those.
        if !active {
            self.reset_code_count();
        }
        // Unmarked regions go now: no surviving value names one, and no weak
        // entry into one is left (swept above), so their ids can be reused.
        self.sweep_regions(&region_marks);
        let compacting = self.compacts(&counts, until_key);
        // A completely live heap frees nothing; a compaction still closes
        // the free entries it has.
        let marked: usize = counts.iter().sum();
        if pads == 0 && marked == private_before && obarray_symbols_live.len() == self.obarray_symbol_names.len()
            && !(compacting && self.free.len() > 0)
        {
            self.gc_retained = self.heap_objects();
            self.settle_payload();
            self.gc_stats.collections += 1;
            return true;
        }
        if !compacting {
            self.reclaim_obarray_symbols(&obarray_symbols_live);
            self.sweep_tables(&marks, &obarray_symbols_live);
            self.cell_pairs.retain(|&i, _| marks[2][i]);
            self.referrers = None;
            self.local_owners = None;
            self.shrink_tables();
            self.gc_retained = self.heap_objects();
            self.settle_payload();
            self.gc_stats.reclaimed += before.saturating_sub(self.gc_retained) as u64;
            self.gc_stats.collections += 1;
            return true;
        }
        // Compaction: every live entry moves down, every reference is
        // rewritten, and no entry is left free.
        self.free.clear();
        // Stress fillers go only where live objects would move past them.
        let fill: Vec<usize> = marks.iter().map(|m| if m.contains(&true) { pads } else { 0 }).collect();
        let maps: Vec<Vec<usize>> = marks
            .iter()
            .zip(&fill)
            .map(|(m, &fill)| {
                let mut next = fill;
                m.iter()
                    .map(|&live| {
                        if live {
                            let n = next;
                            next += 1;
                            n
                        } else {
                            usize::MAX
                        }
                    })
                    .collect()
            })
            .collect();
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
        let mut i = 0;
        self.conses.retain_mut(|p| {
            let live = marks[0][i];
            i += 1;
            if live {
                rewrite(&mut p.0);
                rewrite(&mut p.1);
            }
            live
        });
        let mut i = 0;
        self.closures.retain_mut(|c| {
            let live = marks[1][i];
            i += 1;
            if live {
                for v in c.captures.iter_mut().chain(c.metadata.iter_mut()) {
                    rewrite(v);
                }
                rewrite(&mut c.document);
            }
            live
        });
        self.cell_pairs = std::mem::take(&mut self.cell_pairs).into_iter().filter_map(|(i, mut pair)| {
            if !marks[2][i] { return None; }
            let mut cell = Value::reference(i, 5);
            rewrite(&mut cell);
            rewrite(&mut pair);
            Some((cell.payload() as usize, pair))
        }).collect();
        let mut i = 0;
        self.cells.retain_mut(|v| {
            let live = marks[2][i];
            i += 1;
            if live {
                rewrite(v);
            }
            live
        });
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
        let mut i = 0;
        // Forward each physical native slot once, even when several live
        // cursors retain the same subtree (possibly detached from its table).
        native_nodes.clear();
        self.objects.retain_mut(|object| {
            let live = marks[3][i];
            i += 1;
            if live {
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
                    data::Object::Sqlite(_) | data::Object::Free => {}
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
                    data::Object::Obarray(t) => {
                        t.reclaim_removed(&obarray_symbols_live);
                        t.visit_values(&mut |v| rewrite(v));
                    },
                    data::Object::CharTable(t) => t.visit_values_seen(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::SubCharTable(node) => char_table::visit_char_table_node_values(node, &mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::MapKeymapCursor(c) => c.visit_values(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
            }
            live
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
        for code in &self.suspended_code {
            // SAFETY: as in the mark phase.
            unsafe { code.visit_values(&mut |v| rewrite(v)) };
        }
        self.conditions.visit_values(&mut |v| rewrite(v));
        // Stress fillers: unreachable, so the next collection drops them.
        self.conses.splice(0..0, std::iter::repeat_n((Value::NIL, Value::NIL), fill[0]));
        self.closures.splice(0..0, std::iter::repeat_with(|| Closure {
            function: 0, captures: vec![], metadata: vec![], document: Value::NIL,
        }).take(fill[1]));
        self.cells.splice(0..0, std::iter::repeat_n(Value::NIL, fill[2]));
        self.objects.splice(0..0, std::iter::repeat_with(|| {
            data::Object::String(Box::new(data::LispString::from_bytes(vec![], true)))
        }).take(fill[3]));
        self.shrink_tables();
        self.gc_retained = self.heap_objects();
        self.settle_payload();
        self.gc_stats.reclaimed += before.saturating_sub(self.gc_retained) as u64;
        self.gc_stats.collections += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Traced heaps allocated before a collection at scaled percentage S:
    /// GNU's rule `a >= S * (live + a / 2)` solved for `a / live`.
    fn deferral(s: f64) -> f64 { s / (1.0 - s / 2.0) }

    #[test]
    fn gc_cons_percentage_is_scaled_and_clamped_without_a_cliff() {
        assert_eq!(PERCENTAGE_SCALE, 17.0, "the measured choice (VM2-GC-PACING)");
        assert_eq!(MAX_SCALED_PERCENTAGE, 1.8);
        assert!((scaled_percentage(0.1) - 1.7).abs() < 1e-12);
        let default = deferral(scaled_percentage(0.1));
        assert!((default - 34.0 / 3.0).abs() < 1e-9, "GNU's default defers to about eleven traced heaps: {default}");
        assert!((scaled_percentage(0.05) - 0.85).abs() < 1e-12);
        assert!((deferral(MAX_SCALED_PERCENTAGE) - 18.0).abs() < 1e-9, "the ceiling defers to 18 traced heaps");
        for large in [0.11, 0.2, 0.6, 1.0, 2.0, 1e300, f64::INFINITY] {
            assert_eq!(scaled_percentage(large), MAX_SCALED_PERCENTAGE, "{large}");
        }
        assert!(MAX_SCALED_PERCENTAGE < 2.0, "below GNU's never-collect point");
        assert_eq!(scaled_percentage(-0.5), 0.0);
        assert_eq!(scaled_percentage(f64::NAN), 0.0);
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
        let mut second = char_table::CharTable::new(Value::NIL);
        second.root = Arc::clone(&first.root);
        second.ascii = first.ascii.clone();
        let native_bytes = char_table::CharTableNode::storage_bytes(
            &first.root, false, &mut HashSet::default(),
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
        assert_eq!(vm.free.conses, [2, 0], "the free entries, lowest on top");
        assert_eq!(vm.conses[0], (Value::NIL, Value::NIL));
        assert_eq!(vm.free.objects, [0]);
        assert!(vm.object(string).is_err(), "a free entry is no object");
        assert_eq!(vm.heap_objects(), 4);
        let before = vm.free.allocated;
        let fresh = conses(&mut vm, 3);
        assert_eq!(fresh, [list[0], list[2], list[5]]);
        assert_eq!(vm.free.allocated - before, 3 * 16, "reused entries count as allocation");
        assert_eq!(vm.allocate_object(data::Object::Float(2.0)), string);
        assert_eq!(vm.free.len(), 0);
        assert_eq!(vm.heap_objects(), 8);
        assert_eq!(vm.cons_pair(list[1]).unwrap().0, Value::integer(1).unwrap());
    }

    /// Each sweep rebuilds the free lists from its marks: an entry is listed
    /// once however many collections it stays free, and a table nothing
    /// survives in is cut to nothing.
    #[test]
    fn repeated_sweeps_list_each_free_entry_once() {
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
            assert_eq!(vm.free.conses, [6, 5, 4, 3, 2, 1, 0], "round {round}");
            assert_eq!(roots[0], list[7]);
        }
        roots[0] = Value::NIL;
        vm.collect(&mut roots);
        assert!(vm.conses.is_empty() && vm.free.conses.is_empty(), "a dead table is cut to nothing");
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
        assert_eq!(vm.free.conses, [4, 3, 2, 0]);
        vm.free.compact = true;
        vm.collect(&mut roots);
        assert_eq!(roots, [Value::reference(0, 3), Value::reference(1, 3)]);
        assert_eq!((vm.conses.len(), vm.free.len()), (2, 0));
        assert_eq!(vm.cons_pair(roots[1]).unwrap().0, Value::integer(5).unwrap());
        assert!(!vm.free.compact, "a request compacts once");
        conses(&mut vm, 2);
        vm.collect(&mut roots);
        assert_eq!(roots, [Value::reference(0, 3), Value::reference(1, 3)], "back to freeing in place");
        assert_eq!((vm.conses.len(), vm.free.len()), (2, 0));
    }

    /// A table oversized by a collapsed burst, its free entries unused
    /// since the sweep that freed them, compacts at the next idle
    /// collection or after waiting two collections for one. Garbage a
    /// steady allocator takes back each cycle, and small idle tables (a
    /// few MB of cells), do not compact it.
    #[test]
    fn an_oversized_table_compacts_at_idle_or_after_two_collections() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let n = (OUTGROWN_BYTES / 16 + 1000) as i64;
        let list = conses(&mut vm, n);
        let mut roots = [list[n as usize - 1]];
        vm.collect(&mut roots);
        for _ in 0..3 {
            conses(&mut vm, n - 1);
            vm.collect(&mut roots);
            assert_eq!(roots[0], list[n as usize - 1], "a steady allocator reuses the entries");
        }
        for wait in 0..2 {
            vm.collect(&mut roots);
            assert_eq!(roots[0], list[n as usize - 1], "collection {wait} waits for an idle one");
        }
        vm.collect(&mut roots);
        assert_eq!(roots[0], Value::reference(0, 3), "the third compacts");
        assert!(vm.conses.capacity() <= 16384, "and releases the table");
        let list = conses(&mut vm, n);
        roots[0] = list[n as usize - 1];
        vm.collect(&mut roots);
        assert!(vm.collect_until_key(&mut roots, true));
        assert_eq!(roots[0], Value::reference(0, 3), "an idle collection compacts at once");
        let cells: Vec<usize> = (0..200_000).map(|_| vm.alloc_cell(Value::NIL)).collect();
        let mut roots = [roots[0], Value::reference(cells[199_999], 5)];
        vm.collect(&mut roots);
        assert!(vm.collect_until_key(&mut roots, true));
        assert_eq!(roots[1], Value::reference(cells[199_999], 5), "1.6 MB of idle cells is not worth a compaction");
    }

    /// A burst swept while its last entry survives, then released: tables
    /// and free lists shrink back whatever the burst's size.
    #[test]
    fn a_collapsed_burst_leaves_no_capacity_behind() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_gc_verify(false);
        let mut after = vec![];
        for n in [40_000, 80_000, 160_000] {
            let list = conses(&mut vm, n);
            let mut roots = [list[n as usize - 1]];
            vm.collect(&mut roots);
            // The third burst compacts instead: its table waited two collections.
            let lists = vm.free.conses.capacity() * 4;
            assert!(lists == 0 || vm.heap_stats().capacity_bytes >= lists + vm.conses.capacity() * 16, "the lists count as capacity");
            roots[0] = Value::NIL;
            vm.collect(&mut roots);
            assert!(vm.free.conses.capacity() <= 1024, "burst {n}: the free list kept {}", vm.free.conses.capacity());
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
        assert!(vm.free.conses.is_empty() && vm.free.cells.is_empty(), "quarantined, not listed");
        assert_eq!((vm.free.len(), vm.heap_objects()), (4, 1), "but counted free");
        assert_eq!((vm.conses.len(), vm.cells.len(), vm.objects.len()), (2, 1, 1), "never cut");
        assert_eq!(vm.alloc_cons(Value::NIL, Value::NIL), Value::reference(2, 3), "never reused");
    }
}
