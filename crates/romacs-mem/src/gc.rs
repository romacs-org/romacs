//! Full, eager stop-the-world collection in GNU special-edge order.
use crate::{
    Heap, Value,
    objects::*,
    raw::{Marker, RawValue},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

pub trait Trace {
    fn trace(&self, marker: &mut Marker<'_>);
    /// Visit each live buffer undo owner once; undo must not be strongly traced.
    fn undo_heads(&mut self, _: &mut dyn FnMut(&mut RawValue)) {}
    /// Detach unmarked markers from non-owning buffer chains. No Lisp callbacks.
    fn unchain_markers(&mut self, _: &dyn Fn(RawValue) -> bool) {}
}
impl Trace for () {
    fn trace(&self, _: &mut Marker<'_>) {}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GcEvent {
    None,
    WantCycle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Explicit,
    Threshold,
    Idle,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SymbolMode {
    Interim,
    #[default]
    Full,
}

#[derive(Clone, Copy, Debug)]
pub struct GcRecord {
    pub seq: u64,
    pub trigger: Trigger,
    pub core_stop_ns: u64,
    pub gc_elapsed_ns: u64,
    pub input_blocking_ns: u64,
    pub accounted_before: usize,
    pub accounted_after: usize,
    pub overshoot: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct GcStats {
    pub count: u64,
    pub sum_ns: u128,
    pub max_ns: u64,
    pub max_overshoot: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct CycleReport {
    pub freed_objects: usize,
    pub dropped_payloads: usize,
    pub record: GcRecord,
}
#[derive(Default)]
pub(crate) struct State {
    external: BTreeMap<u64, usize>,
    pub(crate) external_marks: BTreeMap<u64, u64>,
    external_debt: usize,
    external_total: usize,
    last_live: usize,
    mode: SymbolMode,
    pinned: BTreeSet<u64>,
    trigger: Option<Trigger>,
    records: Vec<GcRecord>,
    stats: GcStats,
    sweep_poisoned: bool,
    pub(crate) finalizers: Vec<u64>,
    pub(crate) doomed: VecDeque<u64>,
}
impl State {
    pub(crate) fn new() -> Self {
        Self {
            last_live: crate::sym::BUILTIN_COUNT as usize * 48,
            ..Self::default()
        }
    }
    pub(crate) fn mode_is_interim(&self) -> bool {
        self.mode == SymbolMode::Interim
    }
}
impl Resource {
    fn word(self) -> u64 {
        match self {
            Self::String(w) | Self::Payload(w) | Self::Foreign(w) => w,
        }
    }
    fn bytes(self) -> usize {
        // SAFETY: resources only name initialized allocated cells, until retired.
        unsafe {
            match self {
                Self::String(w) => (*((w & !7) as *const LispString)).cap,
                Self::Payload(w) => (**raw_ptr(w).cast::<Box<dyn crate::Payload>>()).bytes(),
                Self::Foreign(_) => unreachable!("foreign bytes require hooks"),
            }
        }
    }
}
impl Heap {
    /// Call at the external resize, before the next allocation/safepoint.
    /// Positive growth increases debt; shrinking never cancels allocation debt.
    pub fn account_external(&self, value: Value<'_>) {
        let resource = self
            .resources
            .borrow()
            .iter()
            .copied()
            .find(|r| r.word() == value.0)
            .expect("resource required");
        self.account_resource(resource);
    }
    // Strings have immutable backing storage: their header's capacity is the
    // accounting authority, so they need no per-object external-size tree entry.
    pub(crate) fn account_new_string(&self, bytes: usize) {
        let mut state = self.gc.borrow_mut();
        state.external_total += bytes;
        state.external_debt += bytes;
        self.debt.set(self.debt.get().saturating_add(bytes));
    }
    pub(crate) fn account_resource(&self, resource: Resource) {
        if matches!(resource, Resource::String(_)) {
            return;
        }
        let bytes = if let Resource::Foreign(w) = resource {
            self.foreign_hooks
                .borrow()
                .as_ref()
                .expect("foreign hooks")
                .bytes(RawValue::from_value(Value::word(w)))
        } else {
            resource.bytes()
        };
        let mut state = self.gc.borrow_mut();
        let old = state.external.insert(resource.word(), bytes).unwrap_or(0);
        state.external_total = state.external_total - old + bytes;
        state.external_debt += bytes.saturating_sub(old);
        self.debt
            .set(self.debt.get().saturating_add(bytes.saturating_sub(old)));
    }
    pub fn accounted_bytes(&self) -> usize {
        if self.foreign_hooks.borrow().is_some() {
            for r in self.resources.borrow().iter().copied() {
                if matches!(r, Resource::Foreign(_)) {
                    self.account_resource(r);
                }
            }
        }
        self.allocation_stats().live_bytes + self.gc.borrow().external_total
    }
    #[inline]
    pub fn allocation_debt(&self) -> usize {
        self.debt.get()
    }
    /// Constant-time allocation threshold test, with no allocator borrow.
    #[inline]
    pub fn gc_due(&self) -> bool {
        self.debt.get() >= self.threshold.get()
    }
    pub fn threshold(&self) -> usize {
        self.threshold.get()
    }
    /// Explicit, growth, and input-wait idle triggers all request the same full path.
    pub fn gc_event(&self, explicit: bool, idle: Duration) -> GcEvent {
        let d = self.allocation_debt();
        let t = self.threshold();
        let trigger = if explicit {
            Some(Trigger::Explicit)
        } else if d >= t {
            Some(Trigger::Threshold)
        } else if idle >= Duration::from_secs(1) && d >= t.div_ceil(2) {
            Some(Trigger::Idle)
        } else {
            None
        };
        if let Some(trigger) = trigger {
            self.gc.borrow_mut().trigger = Some(trigger);
            GcEvent::WantCycle
        } else {
            GcEvent::None
        }
    }
    pub fn set_symbol_mode(&mut self, mode: SymbolMode) {
        self.gc.get_mut().mode = mode;
    }
    pub fn gc_records(&self) -> std::cell::Ref<'_, [GcRecord]> {
        std::cell::Ref::map(self.gc.borrow(), |s| s.records.as_slice())
    }
    pub fn gc_stats(&self) -> GcStats {
        self.gc.borrow().stats
    }
    /// The switch's owner wrapper replaces core-only timings with its real intervals.
    pub fn finish_gc_timings(&mut self, seq: u64, elapsed: Duration, blocking: Duration) {
        let s = self.gc.get_mut();
        let r = s.records.last_mut().expect("cycle required");
        assert_eq!(r.seq, seq);
        assert_eq!(r.gc_elapsed_ns, r.core_stop_ns, "already finished");
        let ns = elapsed.as_nanos() as u64;
        assert!(ns >= r.core_stop_ns);
        assert!(blocking >= elapsed, "blocking interval includes elapsed");
        r.gc_elapsed_ns = ns;
        r.input_blocking_ns = blocking.as_nanos() as u64;
        s.stats.sum_ns += (ns - r.core_stop_ns) as u128;
        s.stats.max_ns = s.stats.max_ns.max(ns);
    }
    fn root_words(&self, roots: &dyn Trace) -> Vec<RawValue> {
        let mut work = Vec::new();
        for r in self.tables.roots.borrow().iter().flatten() {
            work.push(RawValue::from_value(Value::word(r.word)));
        }
        for stack in self
            .tables
            .stacks
            .borrow()
            .iter()
            .filter_map(|s| s.storage.as_ref())
        {
            stack.borrow().trace(&mut work);
        }
        roots.trace(&mut Marker { work: &mut work });
        work.extend(
            self.gc
                .borrow()
                .doomed
                .iter()
                .map(|w| RawValue::from_value(Value::word(*w))),
        );
        for w in self.allocator.borrow().symbol_words() {
            let builtin = w >> 3 < crate::sym::BUILTIN_COUNT as u64;
            let pinned = self.gc.borrow().mode == SymbolMode::Interim && self.symbol_pinned(w);
            if builtin || pinned {
                work.push(RawValue::from_value(Value::word(w)));
            }
        }
        work
    }
    /// Transition bridge for audited raw symbol-id holders.
    pub fn pin_symbol(&self, value: Value<'_>) {
        assert_eq!(value.kind(), crate::Kind::Symbol);
        assert!(
            self.allocator.borrow().address(value.0).is_some(),
            "allocated symbol"
        );
        self.gc.borrow_mut().pinned.insert(value.0);
    }
    fn symbol_pinned(&self, w: u64) -> bool {
        self.gc.borrow().pinned.contains(&w)
    }
    #[cfg(any(test, feature = "verify"))]
    pub(crate) fn edges(&self, word: u64, work: &mut Vec<RawValue>) {
        let ptr = self
            .allocator
            .borrow()
            .address(word)
            .expect("Dangling edge");
        // SAFETY: address just validated this allocated cell.
        unsafe { self.edges_at(word, ptr, work) };
    }
    fn foreign_edges(&self, word: u64, work: &mut Vec<RawValue>) {
        let hooks = self.foreign_hooks.borrow();
        let hooks = hooks.as_ref().expect("foreign hooks");
        hooks.trace(
            RawValue::from_value(Value::word(word)),
            &mut Marker { work },
        );
        let s = self.gc.borrow();
        if self.foreign_flags(word) & 16 != 0
            && (s.finalizers.contains(&word) || s.doomed.contains(&word))
        {
            work.push(hooks.finalizer_function(RawValue::from_value(Value::word(word))));
        }
    }
    /// # Safety
    /// `ptr` must be the validated live allocation for `word` in this heap.
    pub(crate) unsafe fn edges_at(
        &self,
        word: u64,
        ptr: std::ptr::NonNull<u8>,
        work: &mut Vec<RawValue>,
    ) {
        let mut push = |slot: &Slot| work.push(RawValue::from_value(Value::word(slot.word())));
        // SAFETY: allocation validated before following any edge; constructors publish initialized layouts.
        unsafe {
            match word & 7 {
                0 => {
                    let s = ptr.cast::<Symbol>().as_ref();
                    for v in [&s.name, &s.value, &s.function, &s.plist, &s.next] {
                        push(v);
                    }
                }
                3 => {
                    let c = ptr.cast::<Cons>().as_ref();
                    push(&c.car);
                    push(&c.cdr);
                }
                4 => {
                    if crate::alloc::Allocator::foreign_string(word) {
                        self.foreign_edges(word, work);
                    } else {
                        push(&ptr.cast::<LispString>().as_ref().intervals);
                    }
                }
                5 => {
                    let h = ptr.cast::<VectorHeader>().as_ref();
                    if h.flags & FOREIGN != 0 {
                        self.foreign_edges(word, work);
                        return;
                    }
                    for i in 0..if h.flags == WEAK {
                        0
                    } else {
                        h.lisp_slots as usize
                    } {
                        push(&*ptr.as_ptr().add(16).cast::<Slot>().add(i));
                    }
                    if h.pvec == crate::PvecType::Closure {
                        push(&*code_slot(word));
                    }
                }
                7 => {}
                _ => unreachable!(),
            }
        }
    }
    /// Collect strong and special references, then eagerly destroy unreachable resources.
    ///
    /// If tracing, an owner hook, or destruction unwinds, the panic propagates and subsequent collection
    /// panics before tracing. Discard the heap: teardown remains safe, but the
    /// undestroyed resources retired by the interrupted sweep are leaked.
    pub fn collect(&mut self, roots: &mut dyn Trace) -> CycleReport {
        assert!(
            !self.gc.get_mut().sweep_poisoned,
            "collection poisoned by interrupted sweep"
        );
        // An owner hook can also unwind after marks or special-edge writes.
        // Never let a partially completed cycle's marks enter another cycle.
        self.gc.get_mut().sweep_poisoned = true;
        let start = Instant::now();
        let before = self.accounted_bytes();
        let overshoot = self.allocation_debt().saturating_sub(self.threshold());
        let mut work = self.root_words(roots);
        let mut weak = Vec::new();
        self.drain_mark(&mut work, &mut weak);
        let mut undo = Vec::new();
        roots.undo_heads(&mut |head| {
            self.compact_undo(head);
            undo.push(*head);
            work.push(*head);
            self.drain_mark(&mut work, &mut weak);
        });
        self.doom_finalizers(&mut work);
        self.drain_mark(&mut work, &mut weak);
        self.weak_fixpoint(&mut weak);
        roots.unchain_markers(&|v| self.survives(v.0));
        self.tables
            .weak_refs
            .borrow_mut()
            .retain(|_, w| self.survives(*w));
        #[cfg(feature = "verify")]
        {
            self.allocator.get_mut().clear_verify();
            let mut work = self.root_words(roots);
            work.extend(undo);
            for w in weak {
                let (_, pairs) = self.weak_entries(w);
                for (k, v) in pairs {
                    work.extend([k, v]);
                }
            }
            while let Some(v) = work.pop() {
                if v.0 & 3 == 2 || self.external_symbol(v.0) || self.static_leaf(v.0) {
                    continue;
                }
                assert!(
                    self.allocator.borrow().address(v.0).is_some(),
                    "Dangling {{word: {}}}",
                    v.0
                );
                assert!(
                    self.allocator.borrow().marked(v.0),
                    "Lost {{object: {}}}",
                    v.0
                );
                if self.allocator.borrow_mut().verify_visit(v.0) {
                    self.edges(v.0, &mut work);
                }
            }
        }
        let count_before = self.allocator.borrow().allocated_count();
        let mut dropped = 0;
        let mut retired = Vec::new();
        self.resources.get_mut().retain(|r| {
            if self.allocator.borrow().marked(r.word()) {
                true
            } else {
                let state = self.gc.get_mut();
                let bytes = match r {
                    Resource::String(_) => r.bytes(),
                    _ => state
                        .external
                        .remove(&r.word())
                        .expect("accounted resource"),
                };
                state.external_total -= bytes;
                retired.push(*r);
                false
            }
        });
        // Retire every dead resource before user Drop can unwind. Resource has
        // no Rust destructor, so unwinding leaks the remaining local entries.
        // Until sweep completes, stale marks/cells must not enter another cycle.
        self.gc.get_mut().sweep_poisoned = true;
        for r in retired {
            // SAFETY: registry ownership was removed, cell not yet swept.
            unsafe {
                if let Resource::Foreign(w) = r {
                    crate::raw::ForeignHooks::drop(
                        &mut **self
                            .foreign_hooks
                            .get_mut()
                            .as_mut()
                            .expect("foreign hooks"),
                        RawValue::from_value(Value::word(w)),
                    );
                } else {
                    r.drop();
                }
            }
            if !matches!(r, Resource::String(_)) {
                dropped += 1;
            }
        }
        self.allocator
            .get_mut()
            .sweep()
            .expect("page release failed");
        self.epoch = self.epoch.checked_add(1).expect("epoch exhausted");
        // Weak removal reaccounts backing storage; retirement settles all dead
        // resources. No owner allocation/resize is allowed elsewhere in a cycle.
        let after = self.allocation_stats().live_bytes + self.gc.borrow().external_total;
        let freed_objects = count_before - self.allocator.borrow().allocated_count();
        let s = self.gc.get_mut();
        s.pinned
            .retain(|w| self.allocator.borrow().address(*w).is_some());
        s.last_live = after;
        s.external_debt = 0;
        s.external_marks.clear();
        self.debt.set(0);
        self.threshold.set(
            self.config
                .gc_cons_threshold
                .max((self.config.gc_cons_percentage * after as f64) as usize),
        );
        let mut record = GcRecord {
            seq: self.epoch,
            trigger: s.trigger.take().unwrap_or(Trigger::Explicit),
            core_stop_ns: 0,
            gc_elapsed_ns: 0,
            input_blocking_ns: 0,
            accounted_before: before,
            accounted_after: after,
            overshoot,
        };
        s.records.push(record);
        // Include eager drops, pin cleanup and event-vector growth in core stop.
        let stop = start.elapsed().as_nanos() as u64;
        record.core_stop_ns = stop;
        record.gc_elapsed_ns = stop;
        record.input_blocking_ns = stop;
        *s.records.last_mut().expect("just appended") = record;
        s.stats.count += 1;
        s.stats.sum_ns += stop as u128;
        s.stats.max_ns = s.stats.max_ns.max(stop);
        s.stats.max_overshoot = s.stats.max_overshoot.max(overshoot);
        s.sweep_poisoned = false;
        CycleReport {
            freed_objects,
            dropped_payloads: dropped,
            record,
        }
    }
}
#[cfg(test)]
#[path = "gc_tests.rs"]
mod tests;
