//! profiler.c: the CPU and memory profilers. Each keeps a log mapping
//! backtraces, vectors of the `profiler-max-stack-depth` innermost called
//! functions, to counts.
//!
//! The backtrace changes only where a call enters or leaves, so both
//! profilers sample there (`threaded::hooked_call`, `Vm::hooked_leave`): what a
//! meter accumulated since the last boundary was spent under the stack
//! the boundary sees. The CPU meter is a tick count a host thread adds to
//! each sampling interval at which the Lisp thread is running
//! (`host::cpu_sampler`), GNU's SIGPROF on CLOCK_THREAD_CPUTIME_ID as a
//! sample of running time; the memory meter is the heap's growth. A collection is GNU's `Automatic GC' frame: the
//! ticks it takes are the log's GC count.
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What a log compares backtraces by: GNU's `function-equal'. A closure is
/// its code (CLOSURE_CODE: VM2's compiled function, or an explicit
/// closure's body), anything else itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Identity {
    Code(usize),
    Body(Value),
    Itself(Value),
}
impl std::hash::Hash for Identity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match *self {
            Identity::Code(function) => (0u8, function as u64).hash(state),
            Identity::Body(body) => (1u8, body.0).hash(state),
            Identity::Itself(f) => (2u8, f.0).hash(state),
        }
    }
}

/// profiler.c's `struct profiler_log`: a bounded table of backtraces and
/// the samples taken in a collection or evicted on overflow.
struct Log {
    depth: usize,
    size: usize,
    entries: Vec<(Box<[Value]>, i64)>,
    /// Entry by its backtrace's identities, rebuilt after a collection
    /// moves the values identities hold.
    index: HashMap<Box<[Identity]>, usize>,
    stale: bool,
    gc: i64,
    discarded: i64,
}

impl Log {
    fn new(depth: usize, size: usize) -> Self {
        Log { depth, size, entries: Vec::new(), index: HashMap::default(), stale: false, gc: 0, discarded: 0 }
    }
    /// Add COUNT to the entry of the backtrace IDENTITIES name, making one
    /// of TRACE (evicting first when full).
    fn record(&mut self, identities: &[Identity], count: i64, trace: impl FnOnce() -> Box<[Value]>, identity: impl Fn(Value) -> Identity) {
        if self.stale {
            self.reindex(&identity);
        }
        if let Some(&at) = self.index.get(identities) {
            self.entries[at].1 = saturated_add(self.entries[at].1, count);
            return;
        }
        if self.size == 0 {
            self.discarded = saturated_add(self.discarded, count);
            return;
        }
        if self.entries.len() >= self.size {
            self.evict_lower_half(&identity);
        }
        self.index.insert(identities.into(), self.entries.len());
        self.entries.push((trace(), count));
    }
    fn reindex(&mut self, identity: &impl Fn(Value) -> Identity) {
        self.index = self.entries.iter().enumerate()
            .map(|(at, (trace, _))| (trace.iter().map(|&f| identity(f)).collect(), at))
            .collect();
        self.stale = false;
    }
    /// profiler.c evict_lower_half: drop every entry counted no more than
    /// the median, which always frees room.
    fn evict_lower_half(&mut self, identity: &impl Fn(Value) -> Identity) {
        let mut counts: Vec<i64> = self.entries.iter().map(|&(_, count)| count).collect();
        let middle = counts.len() / 2;
        let median = *counts.select_nth_unstable(middle).1;
        let evicted: i64 = self.entries.iter().map(|&(_, c)| c).filter(|&c| c <= median).fold(0, saturated_add);
        self.discarded = saturated_add(self.discarded, evicted);
        self.entries.retain(|&(_, count)| count > median);
        self.reindex(identity);
    }
}

/// The allocation funnels' byte count where the memory meter last read it
/// (gc.rs `Allocation::allocated').
#[derive(Clone, Copy)]
struct HeapMark(u64);

struct Cpu {
    /// The `profiler-cpu-start' argument, for `profiler-cpu-log''s restart.
    interval: i64,
    ticks: Arc<AtomicU64>,
    /// Dropping it ends the sampler thread.
    _sampler: std::sync::mpsc::Sender<()>,
}

#[derive(Default)]
pub(crate) struct Profiler {
    /// A call boundary hook is armed: a profiler runs (call boundaries
    /// sample, `Vm::profile_boundary'), `debug-on-next-call' is set, or an
    /// activation is flagged debug-on-exit. One test on the call path.
    pub active: bool,
    /// `debug-on-next-call' is non-nil.
    pub next_call: bool,
    /// Some activation is flagged debug-on-exit.
    pub exit_flags: bool,
    /// The Lisp call just entered stops for `debug-on-next-call': its
    /// first poll calls the debugger (`threaded::debug_on_call').
    pub debug_call: bool,
    /// Romacs's own Lisp is running where GNU runs C (the debugger's
    /// eligibility test, compiling a quoted lambda): its calls are no
    /// `debug-on-next-call' stops, and the variable stays as it is.
    pub held: u32,
    cpu: Option<Cpu>,
    cpu_log: Option<Log>,
    memory: Option<HeapMark>,
    memory_log: Option<Log>,
    /// The sampled backtrace's identities, kept to spare an allocation.
    scratch: Vec<Identity>,
}

impl Profiler {
    /// Nothing running and no log kept: what a world capture requires.
    pub fn is_idle(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none() && self.cpu_log.is_none() && self.memory_log.is_none()
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for log in self.cpu_log.iter_mut().chain(self.memory_log.iter_mut()) {
            log.stale = true;
            for (trace, _) in &mut log.entries {
                trace.iter_mut().for_each(&mut *visit);
            }
        }
    }
    fn update_active(&mut self) {
        self.active = self.cpu.is_some() || self.memory.is_some() || self.next_call || self.exit_flags;
    }
}

fn saturated_add(a: i64, b: i64) -> i64 {
    a.saturating_add(b).min(Value::MAX_FIXNUM)
}

#[derive(Clone, Copy)]
enum Which {
    Cpu,
    Memory,
}

impl Vm {
    /// Leave the backtrace entries from MARK, after the profilers sample
    /// the stack they leave.
    #[inline(always)]
    pub(super) fn trace_leave(&mut self, mark: usize) {
        if self.profiler.active {
            self.hooked_leave(mark);
        }
        self.backtrace.leave(mark)
    }
    /// The hooks of leaving the entries from MARK: the profilers sample,
    /// and the leaving activations' debug-on-exit flags drop.
    #[cold]
    #[inline(never)]
    pub(super) fn hooked_leave(&mut self, mark: usize) {
        self.profile_boundary();
        if self.profiler.exit_flags {
            self.backtrace.drop_exit_flags_from(mark);
            self.debug_hooks_changed();
        }
    }
    /// Rearm the call hooks after `debug-on-next-call' or a debug-on-exit
    /// flag changed. Arming the next call bumps the world version: no
    /// speculative inline body, which has no frame, hides that call.
    pub(super) fn debug_hooks_changed(&mut self) {
        let next_call = self.forwarded.debug_on_next_call();
        if next_call && !self.profiler.next_call {
            self.world_version = self.world_version.saturating_add(1);
        }
        self.profiler.next_call = next_call;
        self.profiler.exit_flags = self.backtrace.has_exit_flags();
        self.profiler.update_active();
    }
    /// Charge what the meters took since the last boundary to the stack.
    #[cold]
    #[inline(never)]
    pub(super) fn profile_boundary(&mut self) {
        // A plain load first: most boundaries see no tick, and need no swap.
        if let Some(cpu) = &self.profiler.cpu && cpu.ticks.load(Ordering::Relaxed) > 0 {
            let ticks = cpu.ticks.swap(0, Ordering::Relaxed);
            self.profile_record(Which::Cpu, ticks.min(i64::MAX as u64) as i64);
        }
        if let Some(mark) = self.profiler.memory {
            let bytes = self.allocated_since(mark);
            self.profiler.memory = Some(self.heap_mark());
            if bytes > 0 {
                self.profile_record(Which::Memory, bytes);
            }
        }
    }
    /// Before a collection: the stack it interrupts takes what came first.
    pub(super) fn profile_collection_begin(&mut self) {
        if self.profiler.active {
            self.profile_boundary();
        }
    }
    /// After it: its ticks are GNU's `Automatic GC' samples, and the heap
    /// it compacted is the memory meter's new base.
    pub(super) fn profile_collection_end(&mut self) {
        if let Some(cpu) = &self.profiler.cpu {
            let ticks = cpu.ticks.swap(0, Ordering::Relaxed).min(i64::MAX as u64) as i64;
            if let Some(log) = &mut self.profiler.cpu_log {
                log.gc = saturated_add(log.gc, ticks);
            }
        }
        if self.profiler.memory.is_some() {
            self.profiler.memory = Some(self.heap_mark());
        }
    }
    fn profile_record(&mut self, which: Which, count: i64) {
        let Vm { profiler, backtrace, closures, .. } = self;
        let Profiler { cpu_log, memory_log, scratch, .. } = profiler;
        let Some(log) = (match which { Which::Cpu => cpu_log, Which::Memory => memory_log }) else { return };
        // profiler.c get_backtrace: innermost first, padded with nil.
        let depth = log.depth;
        let trace = || backtrace.frames.iter().rev().map(|frame| frame.function).chain(std::iter::repeat(Value::NIL)).take(depth);
        let identity = |f: Value| code_identity(closures, f).unwrap_or(Identity::Itself(f));
        scratch.clear();
        scratch.extend(trace().map(identity));
        log.record(scratch, count, || trace().collect(), identity);
    }
    fn heap_mark(&self) -> HeapMark {
        HeapMark(self.allocation.allocated)
    }
    /// Bytes allocated since MARK: GNU's malloc_probe sizes, an entry and
    /// its payload as the allocation funnels count them (data.rs).
    fn allocated_since(&self, mark: HeapMark) -> i64 {
        self.allocation.allocated.saturating_sub(mark.0).min(i64::MAX as u64) as i64
    }
    fn profiler_log(&self) -> Log {
        let depth = self.forwarded.integer("profiler-max-stack-depth").clamp(0, i32::MAX as i64) as usize;
        let size = self.forwarded.integer("profiler-log-size").clamp(0, i32::MAX as i64) as usize;
        Log::new(depth, size)
    }

    /// Fprofiler_cpu_start: sample every INTERVAL nanoseconds of the
    /// calling thread's CPU time.
    pub(super) fn profiler_cpu_start(&mut self, interval: i64) -> std::result::Result<(), String> {
        if self.profiler.cpu.is_some() {
            return Err("CPU profiler is already running".into());
        }
        if interval < 1 {
            return Err("Invalid sampling interval".into());
        }
        let ticks = Arc::new(AtomicU64::new(0));
        let sampler = crate::host::cpu_sampler(interval as u64, ticks.clone())
            .map_err(|_| "Unable to start profiler timer".to_string())?;
        if self.profiler.cpu_log.is_none() {
            self.profiler.cpu_log = Some(self.profiler_log());
        }
        self.profiler.cpu = Some(Cpu { interval, ticks, _sampler: sampler });
        self.profiler.update_active();
        Ok(())
    }
    /// Fprofiler_cpu_stop: whether it was running. The log stays.
    pub(super) fn profiler_cpu_stop(&mut self) -> bool {
        let was = self.profiler.cpu.take().is_some();
        self.profiler.update_active();
        was
    }
    pub(super) fn profiler_cpu_running(&self) -> bool {
        self.profiler.cpu.is_some()
    }
    /// Fprofiler_cpu_log: the log as a hash table, or nil when none was
    /// kept since the last call; a running profiler starts a new one.
    pub(super) fn profiler_cpu_log(&mut self) -> std::result::Result<Value, String> {
        let restart = self.profiler.cpu.as_ref().map(|cpu| cpu.interval);
        self.profiler_cpu_stop();
        let log = self.profiler.cpu_log.take();
        let table = self.export_log(log);
        if let Some(interval) = restart {
            self.profiler_cpu_start(interval)?;
        }
        Ok(table)
    }
    /// Fprofiler_memory_start.
    pub(super) fn profiler_memory_start(&mut self) -> std::result::Result<(), String> {
        if self.profiler.memory.is_some() {
            return Err("Memory profiler is already running".into());
        }
        if self.profiler.memory_log.is_none() {
            self.profiler.memory_log = Some(self.profiler_log());
        }
        self.profiler.memory = Some(self.heap_mark());
        self.profiler.update_active();
        Ok(())
    }
    /// Fprofiler_memory_stop: whether it was running. The log stays.
    pub(super) fn profiler_memory_stop(&mut self) -> bool {
        let was = self.profiler.memory.take().is_some();
        self.profiler.update_active();
        was
    }
    pub(super) fn profiler_memory_running(&self) -> bool {
        self.profiler.memory.is_some()
    }
    /// Fprofiler_memory_log.
    pub(super) fn profiler_memory_log(&mut self) -> std::result::Result<Value, String> {
        let restart = self.profiler_memory_stop();
        let log = self.profiler.memory_log.take();
        let table = self.export_log(log);
        if restart {
            self.profiler_memory_start()?;
        }
        Ok(table)
    }
    /// profiler.c export_log: an `equal' table from fresh backtrace
    /// vectors to counts, with GNU's `Automatic GC' and `Discarded
    /// Samples' entries.
    fn export_log(&mut self, log: Option<Log>) -> Value {
        let Some(log) = log else { return Value::NIL };
        let table = hash::HashTable::new(hash::Test::Equal, hash::Weakness::None, log.entries.len() + 2);
        let table = self.allocate_object(data::Object::HashTable(Box::new(table)));
        let special = [("Automatic GC", log.gc), ("Discarded Samples", log.discarded)];
        let special: Vec<(Vec<Value>, i64)> = special.into_iter()
            .filter(|&(_, count)| count > 0)
            .map(|(name, count)| (vec![self.intern(name), Value::NIL], count))
            .collect();
        let entries = log.entries.into_iter().map(|(trace, count)| (trace.into_vec(), count)).chain(special);
        for (trace, count) in entries {
            let key = self.allocate_object(data::Object::Vector(trace));
            let count = Value::integer(count).expect("profiler counts are fixnums");
            self.declaration_hash_put(table, key, count).expect("a fresh equal table takes vector keys");
        }
        table
    }
}

/// A closure's code, which `function-equal' compares.
pub(super) fn code_identity(closures: &crate::table::Table<Closure>, f: Value) -> Option<Identity> {
    if f.tag() != 4 {
        return None;
    }
    let closure = closures.get(f.payload() as usize)?;
    Some(match closure.metadata.get(1) {
        Some(&body) => Identity::Body(body),
        None => Identity::Code(closure.function),
    })
}

impl Vm {
    /// Ffunction_equal: F1 and F2 are one function, or closures of one code.
    pub(super) fn function_equal(&self, a: Value, b: Value) -> bool {
        a == b || code_identity(&self.closures, a).is_some_and(|x| code_identity(&self.closures, b) == Some(x))
    }
}

#[cfg(test)]
#[path = "profiler_tests.rs"]
mod profiler_tests;
