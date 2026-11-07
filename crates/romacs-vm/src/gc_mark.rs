//! The collection's threads (design note gc-smooth.md 4.5). Marking reads the
//! heap and writes only colours, so it runs on N threads, or inline at N = 1,
//! which is the same code. Threads trace the kinds that hold plain Values:
//! conses, cells, closures, vectors, records, leaf objects and compiled code.
//! Every other kind is claimed and handed to the coordinator, the collecting
//! thread, whose visitors take it `&mut'.
//!
//! What a thread reaches is what the compiler lets it reach: a `MarkView' of
//! `Sync' fields, and the tables through `RawTable', the one `unsafe' boundary
//! here. Threads live inside a scope (`Marking::crew', `share_out') that ends
//! before anything can take `&mut Vm', so no phase of a collection overlaps a
//! thread it did not start. What a sweep frees that owns memory is a `Grave',
//! `Send' by construction, dropped after the pause.
use super::*;
use crate::gc::{Census, traced};
use crate::table::RawTable;
use crossbeam_deque::{Injector, Steal};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

/// Values in a chunk; a thread holding twice this offers one.
const CHUNK: usize = 1024;
/// Entries of a table in a chunk of roots or of the sweep; a function holds
/// about 32 instructions, so a chunk of them is 1/32 of this.
const RANGE: usize = 1 << 14;
/// Looks a waiting thread makes before it yields, and before it parks.
const SPINS: u32 = 1024;
const YIELDS: u32 = 16;

/// Threads a mark may use, set by the process's own entry point.
static WORKERS: AtomicUsize = AtomicUsize::new(1);

/// No collection in this process uses threads, whatever anything asks.
static FORBIDDEN: AtomicBool = AtomicBool::new(false);

/// Mark on N threads from now on: what a session's entry point asks. Nothing
/// about threads is in a `Vm' or a world.
pub fn set_workers(n: usize) {
    WORKERS.store(n.max(1), Ordering::Relaxed);
}

/// This process may fork, and a zygote must hold no thread
/// (`isolate_fork_refusal'): the rule that overrides every setting,
/// `ROMACS_GC_WORKERS' included, which an isolate inherits.
pub fn forbid_threads() {
    FORBIDDEN.store(true, Ordering::Release);
}

/// The threads a collection uses, ASKED being a test mode's (0: none): one
/// where it polls for a key (ALONE), which needs the host, in a build, in a
/// process that may fork, and in a builder, whose pool would land in the heap
/// that becomes its image.
pub(super) fn threads(asked: usize, alone: bool) -> usize {
    if alone || crate::host::mode() != crate::host::Mode::Off || FORBIDDEN.load(Ordering::Acquire) || crate::world::arena::heap_is_image() {
        return 1;
    }
    match asked {
        0 => WORKERS.load(Ordering::Relaxed),
        n => n,
    }
}

/// A pool of its own, as the global one runs the user's fuzzy matching. Made
/// by the first collection that wants threads, at its N: the number is the
/// process's (`set_workers'), and a test asking for more gets these. Every VM
/// in the process shares it, so one collection's scope can wait behind
/// another's tasks: a longer pause, never a deadlock.
fn pool(n: usize) -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| threads_named("romacs-gc", n))
}

/// The thread that drops graves, apart from the pool, so that no collection
/// after it waits for a drop.
fn undertaker() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| threads_named("romacs-gc-graves", 1))
}

fn threads_named(name: &'static str, n: usize) -> rayon::ThreadPool {
    let pool = crate::host::with_signals_blocked(|| rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(move |i| format!("{name}-{i}")).build());
    pool.expect("collector threads")
}

/// What a mark's threads may read of an object, and all of it: nothing a
/// `Cell' or a lock guards (`Object' holds both: a string's cursors, a
/// buffer's text), which `RawTable::get' refuses `Object' for.
enum Seen<'a> {
    Free,
    Float,
    /// A vector or record: its slots, and its spare capacity.
    Values(&'a [Value], usize),
    /// A string: its bytes, and whether it has properties to hand over.
    Text { bytes: usize, properties: bool },
    /// A bignum, bytes, a bool vector: memory, and no Values.
    Memory,
    /// A syntax table, an SQLite handle, the thread: no Values, and side
    /// effects when dropped.
    Inert,
    Finalizer(Value),
    Buffer,
    /// Every other kind, handed to the coordinator.
    Other,
}

impl Seen<'_> {
    /// Whether the sweep may take the object as it finds it: memory it owns
    /// and no more (`Grave'). Every other kind has its side effects at a known
    /// time, on the collecting thread, before the sweep.
    fn buried(&self) -> bool {
        matches!(self, Seen::Free | Seen::Float | Seen::Values(..) | Seen::Text { .. } | Seen::Memory)
    }
}

impl RawTable<data::Object> {
    /// Object I, as `Seen'. SAFETY: as `RawTable::get'.
    unsafe fn seen(&self, i: usize) -> Seen<'_> {
        use data::Object::*;
        // SAFETY: as the caller's; only plain data is read below.
        match unsafe { self.get_unsync(i) } {
            Free => Seen::Free,
            Float(_) => Seen::Float,
            Vector(values) | Record(values) => Seen::Values(values, values.capacity() - values.len()),
            String(s) | RawString(s) => Seen::Text { bytes: s.bytes().len(), properties: !s.properties.is_empty() },
            Bignum(_) | Bytes(..) | BoolVector(_) => Seen::Memory,
            SyntaxTable(_) | Sqlite(_) | Thread => Seen::Inert,
            Finalizer(function) => Seen::Finalizer(*function),
            Buffer(_) | ByteBuffer(_) => Seen::Buffer,
            _ => Seen::Other,
        }
    }
}

/// What a dead object owned, for any thread to drop. `Object' is not `Send',
/// and each of these is, so a kind that holds more cannot get in here.
enum Grave {
    Bignum(#[allow(dead_code)] Box<num_bigint::BigInt>),
    String(#[allow(dead_code)] Box<data::LispString>),
    Values(#[allow(dead_code)] Vec<Value>),
    Bytes(#[allow(dead_code)] Vec<u8>),
    Bits(#[allow(dead_code)] Box<data::BoolVector>),
}

impl Grave {
    /// What OBJECT, which `Seen::buried' takes, leaves to drop.
    fn of(object: data::Object) -> Option<Self> {
        use data::Object::*;
        match object {
            Free | Float(_) => None,
            Bignum(n) => Some(Self::Bignum(n)),
            String(s) | RawString(s) => Some(Self::String(s)),
            Vector(v) | Record(v) => Some(Self::Values(v)),
            Bytes(b) => Some(Self::Bytes(b)),
            BoolVector(b) => Some(Self::Bits(b)),
            other => {
                // Not dropped here: off the collecting thread, it could free an
                // `Rc' that thread holds.
                std::mem::forget(other);
                unreachable!("a kind `Seen::buried' refuses")
            }
        }
    }
}

/// All a mark's threads read of the VM. Shared borrows of `Sync' fields, so
/// this is everything they can reach, and while one lives no `&mut Vm' can.
pub(super) struct MarkView<'a> {
    slots: &'a [Slot],
    variable_redirects: &'a [Option<VariableRedirect>],
    functions: &'a [Arc<Function>],
    generated_symbols: &'a HashMap<GeneratedName, usize>,
    caches: &'a [Cache],
    cell_pairs: &'a HashMap<usize, Value>,
    execution: &'a threaded::Program,
    spare_programs: &'a [threaded::Program],
    /// The running engine's register stack, then the suspended ones'.
    stacks: Vec<&'a [Value]>,
    in_obarray: &'a [bool],
    local_binding: &'a [bool],
    code_marks: &'a [AtomicBool],
}

impl Vm {
    pub(super) fn mark_view<'a>(&'a self, in_obarray: &'a [bool], local_binding: &'a [bool], code_marks: &'a [AtomicBool]) -> MarkView<'a> {
        MarkView {
            slots: &self.slots, variable_redirects: &self.variable_redirects, functions: &self.functions,
            generated_symbols: &self.generated_symbols, caches: &self.caches, cell_pairs: &self.cell_pairs,
            execution: &self.execution, spare_programs: &self.spare_programs,
            stacks: std::iter::once(self.stack.as_slice()).chain(self.suspended_stacks.iter().map(Vec::as_slice)).collect(),
            in_obarray, local_binding, code_marks,
        }
    }
}

/// A chunk of a mark: one thing to do.
enum Work {
    Trace { values: Vec<Value>, code: Vec<usize> },
    Slots(Range<usize>),
    /// Part of a register stack, by its place in `MarkView::stacks'.
    Registers(usize, Range<usize>),
    /// Part of a claimed vector or record too long to trace in one go.
    VectorPart(usize, Range<usize>),
}

/// One thread's state.
#[derive(Default)]
pub(super) struct Worker {
    stack: Vec<Value>,
    code: Vec<usize>,
    /// Claimed entries for the coordinator.
    serial: Vec<Value>,
    census: Census,
    graves: Vec<Grave>,
}

/// What a collection's threads share, but for the `MarkView' of a mark.
pub(super) struct Marking {
    pub(super) conses: RawTable<(Value, Value)>,
    pub(super) closures: RawTable<Closure>,
    pub(super) cells: RawTable<Value>,
    pub(super) objects: RawTable<data::Object>,
    verify: bool,
    n: usize,
    queue: Injector<Work>,
    /// The VM's planted faults (`gc::Faults'), for its own threads only, the
    /// worker panic taken by the first worker to come to it.
    #[cfg(test)]
    faults: crate::gc::Faults,
    #[cfg(test)]
    worker_panics: AtomicBool,
    /// Graves this collection buried (`Stress::buried').
    #[cfg(test)]
    pub(crate) buried: AtomicUsize,
    /// Chunks published and not finished: none when no thread holds one.
    outstanding: AtomicUsize,
    stop: AtomicBool,
    /// Where a thread with nothing to do waits for a chunk.
    parked: AtomicUsize,
    park: Mutex<()>,
    wake: Condvar,
    /// Claimed entries for the coordinator. The queue itself is the truth: a
    /// separate count of it can be overwritten between take and clear.
    serial: Mutex<Vec<Value>>,
    census: Mutex<Census>,
    /// Run just after the coordinator takes a hand-over: a worker's moment.
    #[cfg(test)]
    after_take: Mutex<Option<Box<dyn FnOnce(&Marking) + Send>>>,
}

/// Ends a chunk however its thread leaves it, so that a panicking worker
/// strands nobody: the scope carries its panic to the coordinator.
struct Holding<'a>(&'a Marking);

impl Drop for Holding<'_> {
    fn drop(&mut self) {
        if self.0.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 { self.0.wake_parked(); }
    }
}

/// Sends a mark's threads home however its coordinator leaves it.
struct Leaving<'a>(&'a Marking);

impl Drop for Leaving<'_> {
    fn drop(&mut self) {
        while let Some(work) = self.0.take() { drop((work, Holding(self.0))); }
        self.0.stop.store(true, Ordering::Release);
        self.0.wake_parked();
    }
}

impl Marking {
    /// A collection's shared state over VM's tables. Whoever uses it after
    /// the VM has been written asks `Vm::tables_held' first.
    pub(super) fn new(vm: &mut Vm, n: usize) -> Self {
        // SAFETY: a collection's tables neither go nor change but through
        // these until its sweep, which `Vm::tables_held' asserts.
        let (conses, closures, cells, objects) = unsafe { (vm.conses.raw(), vm.closures.raw(), vm.cells.raw(), vm.objects.raw()) };
        Self {
            conses, closures, cells, objects,
            verify: vm.gc_stress.verify, n,
            queue: Injector::new(), outstanding: AtomicUsize::new(0),
            stop: AtomicBool::new(false), parked: AtomicUsize::new(0), park: Mutex::new(()), wake: Condvar::new(),
            serial: Mutex::new(Vec::new()), census: Mutex::default(),
            #[cfg(test)]
            faults: vm.gc_stress.faults,
            #[cfg(test)]
            worker_panics: AtomicBool::new(vm.gc_stress.faults.worker_panics),
            #[cfg(test)]
            buried: AtomicUsize::new(0),
            #[cfg(test)]
            after_take: Mutex::default(),
        }
    }
    /// Run WORK here, the mark's threads beside it, until it and they are done.
    pub(super) fn crew<R>(&self, view: &MarkView<'_>, work: impl FnOnce() -> R) -> R {
        if self.n == 1 { return work(); }
        pool(self.n - 1).in_place_scope(|scope| {
            for _ in 1..self.n { scope.spawn(|_| self.serve(view)); }
            let _leaving = Leaving(self);
            work()
        })
    }
    /// Run EACH on 0..COUNT, this thread as OWN and the rest beside it, each
    /// taking the next until none is left.
    fn share_out(&self, own: &mut Worker, count: usize, each: impl Fn(&mut Worker, usize) + Sync) {
        let next = AtomicUsize::new(0);
        let run = |w: &mut Worker| {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= count { break; }
                each(w, i);
            }
        };
        if self.n == 1 { return run(own); }
        pool(self.n - 1).in_place_scope(|scope| {
            for _ in 1..self.n {
                scope.spawn(|_| {
                    let mut w = Worker::default();
                    run(&mut w);
                    self.bury(w.graves);
                });
            }
            run(own);
        });
    }
    /// Sweep and drop on N threads: what a verified collection's re-mark,
    /// alone, hands on to.
    pub(super) fn sweep_on(&mut self, n: usize) {
        self.n = n;
    }
    /// What was counted, once the mark is done.
    pub(super) fn census(&self, own: &Worker) -> Census {
        let mut census = *self.census.lock().unwrap();
        census.add(&own.census);
        census
    }
    /// The collection is over but for its coordinator's graves.
    pub(super) fn finish(&self, own: Worker) {
        self.bury(own.graves);
    }
    /// GRAVES go to be dropped, off the pause, each thread's as it is, so no
    /// list of the dead is copied, by a thread no collection waits for.
    fn bury(&self, graves: Vec<Grave>) {
        if graves.is_empty() { return; }
        #[cfg(test)]
        self.buried.fetch_add(graves.len(), Ordering::AcqRel);
        #[cfg(test)]
        if test::HOLD_GRAVES.load(Ordering::Acquire) { return test::GRAVES.lock().unwrap().push(graves); }
        undertaker().spawn(move || {
            #[cfg(test)]
            test::slow_drop();
            drop(graves);
        });
    }
    fn publish(&self, work: Work) {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
        self.queue.push(work);
        self.wake_parked();
    }
    fn take(&self) -> Option<Work> {
        loop {
            match self.queue.steal() {
                Steal::Success(work) => {
                    #[cfg(test)]
                    test::reschedule();
                    return Some(work);
                }
                Steal::Empty => return None,
                Steal::Retry => {}
            }
        }
    }
    /// Whether every chunk published was finished. A chunk is published by a
    /// thread holding one, or by the only thread that asks this.
    fn drained(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) == 0
    }
    /// A thread with nothing to do, for the Nth time running: spin briefly,
    /// then yield, for a descheduled thread may hold what it waits for, then
    /// park unless READY, asked under the lock every waker takes, so that what
    /// it waits for cannot land unseen between its look and its sleep. The one
    /// wait of every loop here; the timeout is a bound, not the wake.
    fn wait(&self, n: &mut u32, ready: impl Fn() -> bool) {
        *n += 1;
        if *n < SPINS {
            std::hint::spin_loop();
        } else if *n < SPINS + YIELDS {
            std::thread::yield_now();
        } else {
            self.parked.fetch_add(1, Ordering::AcqRel);
            // With `wake_parked''s fence: either this sees what the waker
            // wrote before it looked, or the waker sees this thread parked.
            std::sync::atomic::fence(Ordering::SeqCst);
            let guard = self.park.lock().unwrap();
            if !self.stop.load(Ordering::Acquire) && !ready() {
                drop(self.wake.wait_timeout(guard, Duration::from_micros(200)).unwrap());
            }
            self.parked.fetch_sub(1, Ordering::AcqRel);
        }
    }
    /// Wake what `wait' parked. Through the lock, so that a thread between
    /// its look and its sleep cannot miss it.
    fn wake_parked(&self) {
        std::sync::atomic::fence(Ordering::SeqCst);
        if self.parked.load(Ordering::Relaxed) > 0 {
            drop(self.park.lock().unwrap());
            self.wake.notify_all();
        }
    }
    /// Publish the roots: VALUES, CODE and the ranges of symbol slots and
    /// register stacks.
    pub(super) fn publish_roots(&self, view: &MarkView<'_>, values: Vec<Value>, code: Vec<usize>) {
        for chunk in values.chunks(CHUNK) {
            self.publish(Work::Trace { values: chunk.to_vec(), code: Vec::new() });
        }
        for chunk in code.chunks(CHUNK / 8) {
            self.publish(Work::Trace { values: Vec::new(), code: chunk.to_vec() });
        }
        for from in (0..view.slots.len()).step_by(RANGE) {
            self.publish(Work::Slots(from..(from + RANGE).min(view.slots.len())));
        }
        for (stack, registers) in view.stacks.iter().enumerate() {
            for from in (0..registers.len()).step_by(RANGE) {
                self.publish(Work::Registers(stack, from..(from + RANGE).min(registers.len())));
            }
        }
    }
    /// Do WORK, or, for tracing, move it into W: the entries it looked at.
    fn adopt(&self, view: &MarkView<'_>, w: &mut Worker, work: Work) -> usize {
        let keep = |v: &Value| traced(*v, view.in_obarray);
        match work {
            Work::Trace { values, code } => {
                w.stack.extend(values);
                w.code.extend(code);
                0
            }
            Work::Slots(slots) => {
                let units = slots.len();
                for slot in slots {
                    if view.in_obarray[slot] { continue; }
                    let s = &view.slots[slot];
                    if let Some(Some(VariableRedirect::Alias(target))) = view.variable_redirects.get(slot) {
                        w.stack.push(Value::reference(target + 1, 7));
                    }
                    if keep(&s.function) { w.stack.push(s.function); }
                    // GNU alloc.c:6673 marks `u.s.plist` with the symbol. VM2
                    // symbols are never reclaimed (DIVERGENCES VM2-BOOT4), so a
                    // plist and a buffer-local table are roots for every slot,
                    // interned or not.
                    if keep(&s.plist) { w.stack.push(s.plist); }
                    if keep(&s.local) { w.stack.push(s.local); }
                    if !view.local_binding[slot] && keep(&s.value) { w.stack.push(s.value); }
                }
                units
            }
            Work::Registers(stack, range) => {
                let units = range.len();
                // A stack is sized for the widest function there is, so it is
                // mostly NIL: a block of them is passed over by a test that
                // vectorizes.
                for block in view.stacks[stack][range].chunks(64) {
                    if block.iter().fold(false, |any, &v| any | (v != Value::NIL)) {
                        w.stack.extend(block.iter().copied().filter(keep));
                    }
                }
                units
            }
            Work::VectorPart(i, range) => {
                let units = range.len();
                // SAFETY: claimed, and nothing writes it during a mark.
                if let Seen::Values(values, _) = unsafe { self.objects.seen(i) } {
                    w.stack.extend(values[range].iter().copied().filter(keep));
                }
                units
            }
        }
    }
    /// Give the coordinator what W claimed for it.
    fn hand_over(&self, w: &mut Worker) {
        #[cfg(test)]
        test::reschedule();
        if !w.serial.is_empty() {
            self.serial.lock().unwrap().append(&mut w.serial);
            // The coordinator may be parked, and this is its work.
            self.wake_parked();
        }
    }
    /// A mark thread's loop.
    fn serve(&self, view: &MarkView<'_>) {
        let mut w = Worker::default();
        let mut waits = 0;
        loop {
            if let Some(work) = self.take() {
                let holding = Holding(self);
                #[cfg(test)]
                if self.worker_panics.swap(false, Ordering::AcqRel) { test::worker_panics(); }
                let _ = self.adopt(view, &mut w, work);
                self.drain(view, &mut w, usize::MAX);
                self.hand_over(&mut w);
                // Before it ends: the coordinator reads them once none is held.
                self.census.lock().unwrap().add(&std::mem::take(&mut w.census));
                drop(holding);
                waits = 0;
                continue;
            }
            if self.stop.load(Ordering::Acquire) { return; }
            self.wait(&mut waits, || !self.queue.is_empty());
        }
    }
    /// Trace all there is, the coordinator's part, running SERIAL on each
    /// entry handed to it, and asking KEY after each POLL units of its work.
    /// False when KEY said to stop, nothing being held but what W keeps.
    pub(super) fn round(&self, view: &MarkView<'_>, w: &mut Worker, poll: usize, serial: &mut dyn FnMut(Value, &mut Vec<Value>), key: &mut dyn FnMut() -> bool) -> bool {
        let slice = if self.n > 1 { CHUNK } else { poll };
        let (mut done, mut waits) = (0, 0);
        loop {
            let batch = std::mem::take(&mut *self.serial.lock().unwrap());
            #[cfg(test)]
            if let Some(hook) = self.after_take.lock().unwrap().take() { hook(self); }
            let handed = !batch.is_empty();
            for v in batch { serial(v, &mut w.stack); }
            let worked = if !w.stack.is_empty() || !w.code.is_empty() {
                let left = self.drain(view, w, slice);
                self.hand_over(w);
                slice - left
            } else if let Some(work) = self.take() {
                // Its accounting ends here: the rest of it is this thread's.
                let _holding = Holding(self);
                self.adopt(view, w, work)
            } else {
                // Finished, and a hand-over comes before its chunk's end, so
                // what is in the queue now is all there will be.
                if !handed && self.drained() && self.serial.lock().unwrap().is_empty() { return true; }
                // What it waits for: a chunk, a hand-over, or the last chunk's end.
                self.wait(&mut waits, || !self.queue.is_empty() || !self.serial.lock().unwrap().is_empty() || self.drained());
                continue;
            };
            waits = 0;
            done += worked;
            if done >= poll {
                done = 0;
                if key() { return false; }
            }
        }
    }
    /// VALUES for the next round, in W.
    pub(super) fn add(&self, w: &mut Worker, values: &mut Vec<Value>) {
        w.stack.append(values);
    }
    /// The dead objects `buried' refuses, found by all threads.
    pub(super) fn dead_objects(&self, own: &mut Worker) -> Vec<u32> {
        let len = self.objects.len();
        let dead = Mutex::new(Vec::new());
        self.share_out(own, len.div_ceil(RANGE), |_, i| {
            let mut found = Vec::new();
            // SAFETY: a dead entry is no one's, and nothing writes the heap now.
            self.objects.dead_in(i * RANGE..((i + 1) * RANGE).min(len), |i| !unsafe { self.objects.seen(i) }.buried(), &mut found);
            if !found.is_empty() { dead.lock().unwrap().append(&mut found); }
        });
        dead.into_inner().unwrap()
    }
    /// Sweep the tables: their freed counts and ends.
    pub(super) fn sweep(&self, own: &mut Worker) -> [(usize, usize); 4] {
        let lens = [self.conses.len(), self.closures.len(), self.cells.len(), self.objects.len()];
        let chunks: Vec<(usize, Range<usize>)> = lens.iter().enumerate()
            .flat_map(|(table, &len)| (0..len).step_by(RANGE).map(move |from| (table, from..(from + RANGE).min(len))))
            .collect();
        let swept = Mutex::new([(0, 0); 4]);
        self.share_out(own, chunks.len(), |w, i| {
            let (table, range) = chunks[i].clone();
            let (freed, last) = self.sweep_range(w, table, range);
            let mut swept = swept.lock().unwrap();
            swept[table] = (swept[table].0 + freed, swept[table].1.max(last));
        });
        swept.into_inner().unwrap()
    }
    /// One range of a sweep: its freed count and the end of its last entry in
    /// use. Conses and cells own nothing; a closure's vectors are small and
    /// few; what an object owns goes to W's graves, or at once on one thread.
    fn sweep_range(&self, w: &mut Worker, table: usize, range: Range<usize>) -> (usize, usize) {
        let dead = if self.verify { crate::gc::POISON } else { Value::NIL };
        // SAFETY: each range is one chunk's, and starts on a multiple of 8.
        unsafe {
            match table {
                0 => self.conses.sweep_range(range, self.verify, |pair| *pair = (dead, dead)),
                1 => self.closures.sweep_range(range, self.verify, |c| *c = Closure { function: usize::MAX, captures: vec![], metadata: vec![], document: dead }),
                2 => self.cells.sweep_range(range, self.verify, |cell| *cell = dead),
                _ => self.objects.sweep_range(range, self.verify, |object| {
                    let grave = Grave::of(std::mem::replace(object, data::Object::Free));
                    if self.n > 1 { w.graves.extend(grave); }
                }),
            }
        }
    }
    fn claim_code(view: &MarkView<'_>, id: usize) -> bool {
        // Some low-level arena tests construct capture-only closures.
        view.code_marks.get(id).is_some_and(|mark| !mark.load(Ordering::Relaxed) && !mark.swap(true, Ordering::Relaxed))
    }
    fn share(&self, w: &mut Worker) {
        if self.n == 1 || self.queue.len() >= self.n { return; }
        let values = if w.stack.len() > 2 * CHUNK { w.stack.split_off(w.stack.len() - CHUNK) } else { Vec::new() };
        let code = if w.code.len() > CHUNK / 32 { w.code.split_off(w.code.len() / 2) } else { Vec::new() };
        if !values.is_empty() || !code.is_empty() { self.publish(Work::Trace { values, code }); }
    }
    #[cold]
    fn refuse_stale(&self, table: &str, i: usize) -> ! {
        panic!("a live reference to free {table} entry {i}");
    }
    /// Trace from W until it is empty or LIMIT units of work are done: a cons,
    /// a value, an op. One function is walked whole, and a vector over RANGE
    /// in chunks. The units left.
    fn drain(&self, view: &MarkView<'_>, w: &mut Worker, mut limit: usize) -> usize {
        let in_obarray = view.in_obarray;
        let keep = |v: &Value| traced(*v, in_obarray);
        loop {
            while let Some(id) = w.code.pop() {
                if Self::claim_code(view, id) { limit = limit.saturating_sub(self.trace_code(view, id, w)); }
                if w.code.len() > CHUNK / 32 { self.share(w); }
                if limit == 0 { return 0; }
            }
            if limit == 0 { return 0; }
            limit -= 1;
            let Some(mut v) = w.stack.pop() else { return limit };
            if w.stack.len() > 2 * CHUNK { self.share(w); }
            // Walk the cdr spine directly: a list is the common editor graph,
            // not two worklist pushes and a generic type dispatch per cons.
            while v.tag() == 3 && !Vm::image_value(v) {
                if limit == 0 {
                    w.stack.push(v);
                    return 0;
                }
                limit -= 1;
                let i = v.payload() as usize;
                if !self.conses.claim(i) {
                    if self.verify && self.conses.is_free(i) { self.refuse_stale("cons", i); }
                    break;
                }
                // SAFETY: nothing writes a cons during a mark.
                let (a, b) = *unsafe { self.conses.get(i) };
                if keep(&a) {
                    w.stack.push(a);
                    if w.stack.len() > 2 * CHUNK { self.share(w); }
                }
                v = b;
            }
            let tag = v.tag();
            if tag == 7 {
                if v != Value::UNBOUND && in_obarray.get(v.payload() as usize - 1).copied().unwrap_or(false) { w.serial.push(v); }
                continue;
            }
            // Mapped objects are untraced by contract: the collector must
            // never read an image page (PROT_NONE proofs assert it).
            if Vm::image_value(v) {
                if crate::frozen::region_of(v.payload() as usize) != 0 { w.serial.push(v); }
                continue;
            }
            match tag {
                0 if v != Value::NIL => {
                    let i = v.payload() as usize - 1;
                    if !self.objects.claim(i) {
                        if self.verify && self.objects.is_free(i) { self.refuse_stale("object", i); }
                        continue;
                    }
                    // SAFETY: a claimed entry is this thread's.
                    match unsafe { self.objects.seen(i) } {
                        Seen::Values(values, spare) => {
                            w.census.vectors += 1;
                            w.census.vector_slots += values.len();
                            w.census.vector_free += spare;
                            if values.len() <= RANGE {
                                limit = limit.saturating_sub(values.len());
                                w.stack.extend(values.iter().copied().filter(keep));
                            } else {
                                // In parts, which threads share and a key poll
                                // comes between.
                                for from in (0..values.len()).step_by(RANGE) {
                                    self.publish(Work::VectorPart(i, from..(from + RANGE).min(values.len())));
                                }
                            }
                        }
                        Seen::Text { bytes, properties } => {
                            w.census.strings += 1;
                            w.census.string_bytes += bytes;
                            #[cfg(test)]
                            let properties = properties && !self.faults.skip_properties;
                            if properties { w.serial.push(v); }
                        }
                        Seen::Float => w.census.floats += 1,
                        Seen::Memory | Seen::Inert | Seen::Free => {}
                        Seen::Finalizer(function) => w.stack.push(function),
                        Seen::Buffer => {
                            w.census.buffers += 1;
                            w.serial.push(v);
                        }
                        Seen::Other => w.serial.push(v),
                    }
                    // A hand-over waits for no chunk's end: the coordinator
                    // runs beside this thread.
                    if w.serial.len() >= 32 { self.hand_over(w); }
                }
                4 => {
                    let i = v.payload() as usize;
                    if !self.closures.claim(i) {
                        if self.verify && self.closures.is_free(i) { self.refuse_stale("closure", i); }
                        continue;
                    }
                    // SAFETY: as for an object.
                    let closure = unsafe { self.closures.get(i) };
                    w.stack.extend(closure.captures.iter().copied().filter(keep));
                    w.stack.extend_from_slice(&closure.metadata);
                    w.stack.push(closure.document);
                    w.code.push(closure.function);
                }
                5 => {
                    let i = v.payload() as usize;
                    if !self.cells.claim(i) {
                        if self.verify && self.cells.is_free(i) { self.refuse_stale("cell", i); }
                        continue;
                    }
                    // SAFETY: as for an object. A cell of a lexical pair reads
                    // the pair, and takes its value after the mark.
                    w.stack.push(view.cell_pairs.get(&i).copied().unwrap_or(*unsafe { self.cells.get(i) }));
                }
                _ => {}
            }
        }
    }
    /// What function ID holds: its literals, the symbols and functions it
    /// names, and its lowered code in every program. The work it took.
    fn trace_code(&self, view: &MarkView<'_>, id: usize, w: &mut Worker) -> usize {
        let in_obarray = view.in_obarray;
        let keep = |v: &Value| traced(*v, in_obarray);
        let function = &view.functions[id];
        // R215: a lazily held body is reachable exactly when its function is,
        // which may be through a closure alone.
        if let Some(lazy) = &function.lazy {
            w.stack.push(lazy.body);
            for name in &lazy.names {
                if let Name::Generated(id) = name && let Some(&slot) = view.generated_symbols.get(id) {
                    w.stack.push(Value::reference(slot + 1, 7));
                }
            }
        }
        // A pending compiled-Lisp body: what its code will name.
        if let Some(body) = &function.pending {
            w.stack.extend(body.values.iter().copied());
            w.code.extend(body.children.iter().copied());
        }
        // The lambda list, which slot 0 answers, outlives the lazy body.
        let mut nested = Vec::new();
        for form in function.lazy.iter().flat_map(|lazy| &lazy.args).chain(function.params.iter().flatten()) {
            let mut next = Some(form);
            while let Some(form) = next.take().or_else(|| nested.pop()) {
                match form {
                    Form::Generated(id) => if let Some(&slot) = view.generated_symbols.get(id) { w.stack.push(Value::reference(slot + 1, 7)); },
                    Form::List(items) | Form::Vector(items) => nested.extend(items),
                    Form::Dotted(items, tail) => { nested.extend(items); nested.push(tail); }
                    _ => {}
                }
            }
        }
        for op in &function.code {
            let mut op = *op;
            op.visit_values(&mut |v| if keep(v) { w.stack.push(*v) });
            // Only a symbol interned in an obarray of its own can die.
            let symbol = |slot: usize| in_obarray[slot].then(|| Value::reference(slot + 1, 7));
            match op {
                Op::Global(_, slot) | Op::SetGlobal(slot, _) | Op::Bind(slot, _) | Op::DeclareSpecial(slot)
                | Op::BindLate(slot, _) | Op::ReadLate(_, _, slot) | Op::WriteLate(_, _, slot)
                | Op::Define(slot, _) | Op::DefineMacro(slot, _)
                | Op::Sealed(_, slot, _, _) => w.stack.extend(symbol(slot)),
                Op::Closure(_, child, _) => w.code.push(child),
                Op::Direct(_, slot, cache, _, _) => {
                    w.stack.extend(symbol(slot));
                    w.stack.push(view.caches[cache].target);
                }
                _ => {}
            }
        }
        for program in std::iter::once(view.execution).chain(view.spare_programs) {
            program.trace_function(id, &mut w.stack, &mut w.code);
        }
        // A lazily held body has no ops yet, but a queue of them is still work.
        function.code.len() + 1
    }
}

/// What a test does to the threads' timing, and the faults it plants.
#[cfg(test)]
pub(crate) mod test {
    use super::*;
    /// A seed for a forced yield at each scheduling boundary; 0 is none.
    pub(super) static SEED: AtomicUsize = AtomicUsize::new(0);
    /// Graves stay here while this is set: every VM's, which only delays
    /// the drop of another test's.
    pub(super) static HOLD_GRAVES: AtomicBool = AtomicBool::new(false);
    pub(super) static GRAVES: Mutex<Vec<Vec<Grave>>> = Mutex::new(Vec::new());
    /// The dropper waits while this is set, having counted the drops begun.
    pub(super) static SLOW_DROPS: AtomicBool = AtomicBool::new(false);
    pub(super) static DROPS_STARTED: AtomicUsize = AtomicUsize::new(0);
    /// Which thread a planted worker panic happened on.
    pub(super) static PANICKED_ON: Mutex<Option<String>> = Mutex::new(None);
    /// One test at a time of those that set the statics above.
    static ALONE: Mutex<()> = Mutex::new(());
    pub(crate) fn alone() -> std::sync::MutexGuard<'static, ()> {
        ALONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(super) fn slow_drop() {
        DROPS_STARTED.fetch_add(1, Ordering::AcqRel);
        while SLOW_DROPS.load(Ordering::Acquire) { std::thread::yield_now(); }
    }
    pub(super) fn worker_panics() {
        *PANICKED_ON.lock().unwrap() = std::thread::current().name().map(str::to_owned);
        panic!("a worker's panic");
    }
    pub(super) fn reschedule() {
        let seed = SEED.load(Ordering::Relaxed);
        if seed == 0 { return; }
        let turn = SEED.fetch_add(0x9e37_79b9, Ordering::Relaxed);
        for _ in 0..(turn >> 7) % 6 { std::thread::yield_now(); }
    }
    pub(super) fn release_graves() {
        HOLD_GRAVES.store(false, Ordering::Release);
        GRAVES.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-over that lands while the coordinator is taking the last one
    /// must still reach it: a lost one leaves a claimed entry's children
    /// untraced, to be swept while reachable.
    #[test]
    fn a_hand_over_during_a_take_is_not_lost() {
        let mut vm = Vm::new();
        let (first, second) = (Value::integer(1).unwrap(), Value::integer(2).unwrap());
        let in_obarray = vec![false; vm.slots.len()];
        let local_binding = vec![false; vm.slots.len()];
        let code_marks: Vec<AtomicBool> = vm.functions.iter().map(|_| AtomicBool::new(false)).collect();
        let marking = Marking::new(&mut vm, 1);
        let view = vm.mark_view(&in_obarray, &local_binding, &code_marks);
        let mut own = Worker::default();
        own.serial.push(first);
        marking.hand_over(&mut own);
        *marking.after_take.lock().unwrap() = Some(Box::new(move |marking| {
            let mut worker = Worker::default();
            worker.serial.push(second);
            marking.hand_over(&mut worker);
        }));
        let mut seen = Vec::new();
        assert!(marking.round(&view, &mut own, usize::MAX, &mut |v, _| seen.push(v), &mut || false));
        assert_eq!(seen, [first, second], "every entry handed over reached the coordinator");
    }

    /// Every kind the collector knows, reachable through the ones workers
    /// trace and the ones they hand back, in a heap big enough to share out.
    const EVERY_KIND: &str = "(progn
      (defvar pk-kept nil)
      (fset 'pk-closure (lambda (x) (lambda () x)))
      (let ((i 0) (weak (make-hash-table :test 'eq :weakness 'key)) (strong (make-hash-table :test 'equal))
            (table (make-char-table 'pk-kind)) (array (obarray-make)))
        (put 'pk-kind 'char-table-extra-slots 0)
        (while (< i 3000)
          (let ((string (make-string 8 ?x)) (key (cons i nil)))
            (put-text-property 0 4 'face (list i 'property) string)
            (puthash key (cons i 'weak) weak)
            (puthash (format \"%d\" i) (vector i (cons i 'strong) string) strong)
            (aset table (+ 1000 i) (list i 'char))
            (intern (format \"pk-%d\" i) array)
            (setq pk-kept (cons (list string (pk-closure (cons i 'captured)) (make-record 'pk 2 i) key (* 1.5 i)) pk-kept)))
          (setq i (1+ i)))
        (setq pk-kept (list pk-kept weak strong table array))
        (with-temp-buffer
          (insert \"some text\")
          (setq pk-kept (cons (list (point-marker) (make-overlay 1 2) (current-buffer)) pk-kept))
          (put-text-property 1 3 'face (list 'in-a-buffer))))
      t)";

    /// Under verify, each collection marks on the threads and then alone, and
    /// compares what each reached (`Vm::verify_mark'): nothing is missed or
    /// invented, whichever thread reached it. Across many, racing differently.
    #[test]
    fn threads_reach_what_one_thread_reaches() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_threads(4);
        vm.set_gc_verify(true);
        vm.eval_source_raw(EVERY_KIND).unwrap();
        for _ in 0..40 {
            vm.eval_source_raw("(garbage-collect)").unwrap();
        }
        assert!(vm.gc_times.marked[3] > 10_000, "the fixture filled the heap: {:?}", vm.gc_times.marked);
        let length = vm.eval_source_raw("(length (car (cdr pk-kept)))").unwrap();
        assert_eq!(length.as_integer().unwrap(), 3000);
    }

    /// The whole story of a heap, told the same however it was marked: the
    /// graph, weak tables, finalizers, garbage in quantity, then a compaction.
    /// Under verify each collection also checks itself against a mark on one
    /// thread, and nothing freed is reused (for that, the test after this).
    fn story(threads: usize, seed: usize) -> Vec<String> {
        test::SEED.store(seed, Ordering::Relaxed);
        test::HOLD_GRAVES.store(threads > 1, Ordering::Release);
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_threads(threads);
        vm.set_gc_verify(true);
        // The story is the sweep's, which moving stress would replace.
        vm.set_gc_stress(0);
        vm.eval_source_raw(EVERY_KIND).unwrap();
        vm.eval_source_raw(STORY_SETUP).unwrap();
        vm.eval_source_raw("(pk-doom)").unwrap();
        let mut told = Vec::new();
        for step in STORY {
            told.push(vm.eval_source(&format!("(prin1-to-string {step})")).unwrap().display().unwrap());
        }
        if threads > 1 {
            assert!(vm.gc_stress.buried > 0, "this VM's threaded sweeps buried graves, verify or not");
        }
        test::release_graves();
        vm.set_gc_verify(false);
        vm.collect_compacting();
        told.push(vm.eval_source("(prin1-to-string (pk-graph))").unwrap().display().unwrap());
        vm.set_gc_moving(false);
        told.push(format!("{:?}", vm.gc_census));
        told.push(format!("{}", vm.heap_stats().live_objects));
        told
    }

    /// Weak keys, values and pairs that die and that stay, finalizers that
    /// run and that wait, an obarray losing symbols, a vector long enough to
    /// go in parts.
    const STORY_SETUP: &str = "(progn
      (defvar pk-long (make-vector 40000 nil))
      (defvar pk-weak-keys (make-hash-table :test 'eq :weakness 'key))
      (defvar pk-weak-values (make-hash-table :test 'eq :weakness 'value))
      (defvar pk-weak-both (make-hash-table :test 'eq :weakness 'key-and-value))
      (defvar pk-weak-either (make-hash-table :test 'eq :weakness 'key-or-value))
      (defvar pk-kept-keys nil)
      (defvar pk-ran nil)
      (defvar pk-array (obarray-make))
      (let ((i 0))
        (while (< i 40000)
          (aset pk-long i (if (= 0 (% i 3)) (cons i 'long) (make-string 2 ?l)))
          (setq i (1+ i))))
      (let ((i 0))
        (while (< i 2000)
          (let ((key (cons i 'key)) (value (cons i 'value)))
            (when (= 0 (% i 2)) (setq pk-kept-keys (cons key pk-kept-keys)))
            (puthash key value pk-weak-keys)
            (puthash value key pk-weak-values)
            (puthash key value pk-weak-both)
            (puthash key value pk-weak-either))
          (intern (format \"pk-%d\" i) pk-array)
          (setq i (1+ i))))
      (fset 'pk-doom (lambda () (let ((i 0)) (while (< i 500) (let ((n i)) (make-finalizer (lambda () (setq pk-ran (cons n pk-ran))))) (setq i (1+ i)))) nil))
      (fset 'pk-graph (lambda ()
        (list (length (car (cdr pk-kept))) (hash-table-count pk-weak-keys) (hash-table-count pk-weak-values)
              (hash-table-count pk-weak-both) (hash-table-count pk-weak-either)
              (length pk-ran) (length pk-kept-keys)
              (let ((sum 0) (i 0)) (while (< i 40000) (let ((v (aref pk-long i))) (setq sum (+ sum (if (consp v) (car v) (length v))))) (setq i (1+ i))) sum)
              (let ((n 0)) (mapatoms (lambda (s) (setq n (1+ n))) pk-array) n)
              (length (car (car (car (cdr pk-kept))))))))
      t)";

    /// Each a collection, what it should leave, or what reuses its entries.
    const STORY: &[&str] = &[
        "(progn (garbage-collect) (pk-graph))",
        "(progn (setq pk-kept-keys (nthcdr 500 pk-kept-keys)) (unintern \"pk-7\" pk-array) (garbage-collect) (pk-graph))",
        "(let ((i 0) (fresh nil)) (while (< i 200000) (setq fresh (cons (cons i (make-string 3 ?f)) (if (= 0 (% i 1000)) nil fresh))) (setq i (1+ i))) (length fresh))",
        "(progn (garbage-collect) (pk-graph))",
        "(progn (setq pk-kept-keys nil) (garbage-collect) (garbage-collect) (pk-graph))",
    ];

    /// A freed entry goes to its new tenant at once, while a worker still holds
    /// what the old one owned: they share nothing, so the tenant comes to no
    /// harm when the grave is dropped.
    #[test]
    fn a_freed_entry_is_reused_before_its_grave_is_dropped() {
        let _alone = test::alone();
        const N: i64 = 20_000;
        let list = |what: char| format!("(let ((i 0) (all nil)) (while (< i {N}) (setq all (cons (make-string 64 ?{what}) all)) (setq i (1+ i))) all)");
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_threads(4);
        // Reuse in place, which verify's quarantine and moving stress forbid.
        vm.set_gc_verify(false);
        vm.set_gc_stress(0);
        test::HOLD_GRAVES.store(true, Ordering::Release);
        // The string above them keeps the sweep from cutting them off.
        let mut at = vm.eval_source_raw(&format!("(defvar pk-doomed {}) (defvar pk-above (make-string 1 ?a)) pk-doomed", list('d'))).unwrap();
        let mut doomed = Vec::new();
        while at != Value::NIL {
            let (string, next) = vm.cons_pair(at).unwrap();
            doomed.push(string.payload() as usize - 1);
            at = next;
        }
        vm.eval_source_raw("(setq pk-doomed nil) (garbage-collect)").unwrap();
        assert!(doomed.iter().all(|&i| vm.objects.is_free(i)), "the sweep freed them");
        vm.eval_source_raw(&format!("(defvar pk-tenants {})", list('t'))).unwrap();
        let reused = doomed.iter().filter(|&&i| !vm.objects.is_free(i)).count();
        assert!(reused > doomed.len() / 2, "{reused} of {} entries have new tenants", doomed.len());
        let held: usize = test::GRAVES.lock().unwrap().iter().map(Vec::len).sum();
        assert!(held >= doomed.len(), "the old tenants' graves are still held: {held}");
        test::release_graves();
        vm.eval_source_raw("(garbage-collect)").unwrap();
        let whole = vm.eval_source_raw("(let ((n 0) (at pk-tenants) (tenant (make-string 64 ?t))) (while at (when (equal (car at) tenant) (setq n (1+ n))) (setq at (cdr at))) n)").unwrap();
        assert_eq!(whole.as_integer().unwrap(), N);
    }

    /// One thread and four tell the same story, under forced scheduling.
    #[test]
    fn threads_and_one_thread_tell_the_same_story() {
        let _alone = test::alone();
        let alone = story(1, 0);
        for seed in [0, 1, 0x5eed, 0xbeef] {
            assert_eq!(story(4, seed), alone, "seed {seed}");
        }
    }

    /// Verify's panic at a stale reference, on whichever thread traces it,
    /// ends the collection with nothing swept, and the next starts clean.
    #[test]
    fn a_panic_on_a_thread_ends_the_collection() {
        let _alone = test::alone();
        let mut vm = Vm::new();
        vm.set_gc_threads(4);
        vm.set_gc_verify(true);
        let stale = vm.alloc_cons(Value::NIL, Value::NIL);
        vm.collect_garbage();
        let collections = vm.heap_stats().collections;
        vm.external_roots.push(stale);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.collect_garbage()));
        assert!(unwound.is_err(), "verify refuses a stale reference");
        assert_eq!(vm.heap_stats().collections, collections, "nothing was collected");
        vm.external_roots.pop();
        vm.collect_garbage();
        assert_eq!(vm.heap_stats().collections, collections + 1);
    }

    /// A pool worker's panic ends its chunk as it unwinds, so the coordinator
    /// waits for nothing, and the scope carries the panic to it: the
    /// collection ends with nothing swept, and the next starts clean.
    #[test]
    fn a_workers_panic_reaches_the_coordinator() {
        let _alone = test::alone();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_threads(4);
        vm.eval_source_raw(EVERY_KIND).unwrap();
        vm.collect_garbage();
        let collections = vm.heap_stats().collections;
        *test::PANICKED_ON.lock().unwrap() = None;
        vm.gc_stress.faults.worker_panics = true;
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.collect_garbage()));
        vm.gc_stress.faults.worker_panics = false;
        let on = test::PANICKED_ON.lock().unwrap().clone().unwrap_or_default();
        assert!(on.starts_with("romacs-gc-") && !on.starts_with("romacs-gc-graves"), "a pool worker panicked: {on:?}");
        assert!(unwound.is_err(), "the panic reached the coordinator");
        assert_eq!(vm.heap_stats().collections, collections, "nothing was collected");
        vm.collect_garbage();
        assert_eq!(vm.heap_stats().collections, collections + 1);
        let length = vm.eval_source_raw("(length (car (cdr pk-kept)))").unwrap();
        assert_eq!(length.as_integer().unwrap(), 3000);
    }

    /// A drop job that has started and is held stalls nothing: under it,
    /// freed entries are reused, the graph, weak tables and finalizers come out
    /// as with one thread and no drop held, and the collections finish.
    #[test]
    fn a_held_drop_holds_up_nothing() {
        let _alone = test::alone();
        let alone = story(1, 0);
        test::DROPS_STARTED.store(0, Ordering::Release);
        test::SLOW_DROPS.store(true, Ordering::Release);
        // Lets the drops go if a collection waits for one, and says so.
        let watchdog = std::thread::spawn(|| {
            let start = std::time::Instant::now();
            while test::SLOW_DROPS.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(30) { std::thread::sleep(Duration::from_millis(10)); }
            test::SLOW_DROPS.swap(false, Ordering::AcqRel)
        });
        let held = held_story();
        test::SLOW_DROPS.store(false, Ordering::Release);
        assert!(!watchdog.join().unwrap(), "a collection waited for a drop");
        assert_eq!(held, alone);
    }

    /// `story' with reuse in place, a drop job started and held from the first
    /// sweep on, and the reuse under it checked.
    fn held_story() -> Vec<String> {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_gc_threads(4);
        vm.set_gc_verify(false);
        vm.set_gc_stress(0);
        vm.eval_source_raw(EVERY_KIND).unwrap();
        vm.eval_source_raw(STORY_SETUP).unwrap();
        vm.eval_source_raw("(pk-doom)").unwrap();
        let mut told = Vec::new();
        for (n, step) in STORY.iter().enumerate() {
            let reused = vm.objects.reused;
            told.push(vm.eval_source(&format!("(prin1-to-string {step})")).unwrap().display().unwrap());
            if n == 0 {
                let start = std::time::Instant::now();
                while test::DROPS_STARTED.load(Ordering::Acquire) == 0 && start.elapsed() < Duration::from_secs(10) { std::thread::yield_now(); }
                assert!(test::DROPS_STARTED.load(Ordering::Acquire) > 0, "a drop job started and is held");
            }
            // The step that allocates after a collection takes freed entries.
            if n == 2 { assert!(vm.objects.reused > reused, "freed entries were reused under the held drop"); }
        }
        vm.collect_compacting();
        told.push(vm.eval_source("(prin1-to-string (pk-graph))").unwrap().display().unwrap());
        told.push(format!("{:?}", vm.gc_census));
        told.push(format!("{}", vm.heap_stats().live_objects));
        told
    }

    /// What a panic says.
    fn said(unwound: std::thread::Result<()>) -> String {
        match unwound {
            Ok(()) => String::new(),
            Err(payload) => payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default(),
        }
    }

    /// The reference marker sees what both marks miss in code they share:
    /// here, no string's properties handed over, at one thread and four.
    #[test]
    fn the_reference_marker_catches_a_shared_omission() {
        let _alone = test::alone();
        for threads in [1, 4] {
            let mut vm = Vm::new();
            vm.install_lisp_prelude().unwrap();
            vm.set_gc_threads(threads);
            vm.set_gc_verify(true);
            vm.eval_source_raw(EVERY_KIND).unwrap();
            vm.gc_stress.faults.skip_properties = true;
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.collect_garbage()));
            vm.gc_stress.faults.skip_properties = false;
            let said = said(unwound);
            assert!(said.contains("the reference marker reached"), "at {threads}: {said:?}");
        }
    }

    /// A table changed between a mark and its scan, or its sweep, is
    /// refused before either reaches it, and the next collection is clean.
    #[test]
    fn a_table_changed_before_its_sweep_is_refused() {
        for scan in [true, false] {
            let mut vm = Vm::new();
            // The sweep's, which moving stress would replace.
            vm.set_gc_stress(0);
            vm.eval_source_raw("(defvar kept (make-list 1000 nil))").unwrap();
            if scan { vm.gc_stress.faults.allocate_before_scan = true } else { vm.gc_stress.faults.allocate_before_sweep = true }
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.collect_garbage()));
            let said = said(unwound);
            assert!(said.contains("a table changed between its mark and its sweep"), "before the {}: {said:?}", if scan { "scan" } else { "sweep" });
            vm.collect_garbage();
            assert_eq!(vm.eval_source_raw("(length kept)").unwrap().as_integer().unwrap(), 1000);
        }
    }

    /// The budget stops code tracing too, however many functions are queued.
    #[test]
    fn a_budget_stops_at_code() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(let ((i 0)) (while (< i 2000) (eval (list 'defun (intern (format \"f%d\" i)) nil (list '+ i 1))) (setq i (1+ i))))").unwrap();
        let in_obarray = vec![false; vm.slots.len()];
        let local_binding = vec![false; vm.slots.len()];
        let code_marks: Vec<AtomicBool> = vm.functions.iter().map(|_| AtomicBool::new(false)).collect();
        let marking = Marking::new(&mut vm, 1);
        let view = vm.mark_view(&in_obarray, &local_binding, &code_marks);
        let mut own = Worker::default();
        own.code = (0..vm.functions.len()).collect();
        let queued = own.code.len();
        assert!(queued > 1000, "the fixture has functions: {queued}");
        assert_eq!(marking.drain(&view, &mut own, 1), 0);
        assert!(own.code.len() > queued / 2, "stopped at the budget: {} of {queued} left", own.code.len());
    }
}
