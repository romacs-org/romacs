//! Unwired L1 memory core. Allocation never collects; callbacks carry frames,
//! not borrowed values. Collection is full, eager and stop-the-world.
// The storage constructors and sweep are consumed by S3/S4; default builds
// intentionally expose no uninitialized object constructor.
#[cfg_attr(not(feature = "test-drivers"), allow(dead_code))]
mod alloc;
mod gc;
mod special;
pub use special::{WeakId, WeakPairsRef, WeakRef};
mod handles;
pub use gc::{CycleReport, GcEvent, GcRecord, GcStats, SymbolMode, Trace, Trigger};
mod objects;
#[cfg_attr(not(feature = "test-drivers"), allow(dead_code))]
mod os;
#[cfg(test)]
mod perf_tests;
pub mod raw;
mod value;
pub use objects::{ConsRef, Payload, Slot, StringRef, SymbolCell, SymbolRef, VectorRef};

pub use alloc::{AllocationStats, Counters};
pub use handles::{Args, Frame, Root, ValueStack};

/// Storage allocation failure; Lisp signal conversion belongs to the evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    MemoryFull,
    InvalidSize,
    UnsupportedPageSize,
    SymbolLimit,
}
impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AllocError {}
use std::{
    rc::Rc,
    thread::{self, JoinHandle},
};
pub use value::{Kind, PvecType, Value, sym};

/// Explicit isolate configuration; no environment or process-global heap.
#[derive(Clone, Debug)]
pub struct HeapConfig {
    pub gc_cons_threshold: usize,
    pub gc_cons_percentage: f64,
    /// Maximum live symbol slots, including builtins (3..=2^30). Pages are
    /// committed on demand within a stable reservation; exhaustion returns
    /// [`AllocError::SymbolLimit`], and collection makes retired slots reusable.
    pub max_symbols: usize,
    pub external_symbols: bool,
}
impl Default for HeapConfig {
    fn default() -> Self {
        Self {
            gc_cons_threshold: 800_000,
            gc_cons_percentage: 0.1,
            max_symbols: 8 * 1024 * 1024,
            external_symbols: false,
        }
    }
}

/// A thread-confined heap, normally created by [`spawn_isolate`].
/// The unsafe [`Heap::for_legacy_owner`] bridge also permits current-thread creation.
/// Values borrow it, preventing mutable safepoints while values remain in use.
pub struct Heap {
    pub(crate) tables: Rc<handles::Tables>,
    pub(crate) symbol_base: std::ptr::NonNull<u8>,
    pub(crate) allocator: std::cell::RefCell<alloc::Allocator>,
    pub(crate) resources: std::cell::RefCell<Vec<objects::Resource>>,
    pub(crate) foreign_hooks: std::cell::RefCell<Option<Box<dyn raw::ForeignHooks>>>,
    pub(crate) static_ranges: Vec<std::ops::Range<usize>>,
    config: HeapConfig,
    debt: Rc<std::cell::Cell<usize>>,
    threshold: std::cell::Cell<usize>,
    epoch: u64,
    pub(crate) gc: std::cell::RefCell<gc::State>,
}
impl Heap {
    fn create(config: HeapConfig) -> Self {
        assert!(config.gc_cons_percentage.is_finite() && config.gc_cons_percentage >= 0.0);
        let allocator = alloc::Allocator::new(config.max_symbols).expect("isolate memory-full");
        let debt = allocator.debt.clone();
        Self {
            tables: Rc::new(handles::Tables::new()),
            symbol_base: allocator.symbol_base(),
            allocator: std::cell::RefCell::new(allocator),
            resources: std::cell::RefCell::new(Vec::new()),
            foreign_hooks: std::cell::RefCell::new(None),
            static_ranges: Vec::new(),
            debt,
            threshold: std::cell::Cell::new(config.gc_cons_threshold.max(
                (config.gc_cons_percentage * (sym::BUILTIN_COUNT as usize * 48) as f64) as usize,
            )),
            config,
            epoch: 0,
            gc: std::cell::RefCell::new(gc::State::new()),
        }
    }
    pub fn config(&self) -> &HeapConfig {
        &self.config
    }
    /// Borrow a fixnum from this heap (range is signed 62 bits).
    pub fn fixnum(&self, n: i64) -> Option<Value<'_>> {
        Value::fixnum(n)
    }
    pub fn nil(&self) -> Value<'_> {
        sym::NIL
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    /// Allocation never collects; the owner calls collect after WantCycle.
    pub fn safepoint(&mut self) -> GcEvent {
        self.gc_event(false, std::time::Duration::ZERO)
    }
    pub fn allocation_stats(&self) -> AllocationStats {
        self.allocator.borrow().stats()
    }
    /// S2 driver only: reserve zeroed storage, with no S3 typed object layout.
    /// Bypasses initialized typed constructors; reserved for storage measurements.
    #[cfg(feature = "test-drivers")]
    /// # Safety
    /// Returned storage is uninitialized. Do not use typed object accessors on it.
    pub unsafe fn reserve_storage(
        &self,
        kind: Kind,
        bytes: usize,
    ) -> Result<Value<'_>, AllocError> {
        self.allocator
            .borrow_mut()
            .alloc(kind, bytes)
            .map(Value::word)
    }
    /// Mark one storage cell in the S2 allocator driver (not graph tracing).
    #[cfg(feature = "test-drivers")]
    pub fn mark_storage(&self, value: Value<'_>) -> bool {
        self.allocator.borrow_mut().mark(value.0)
    }
    /// S2 test sweep. Does not trace roots, stack slots, or child references.
    #[cfg(feature = "test-drivers")]
    /// # Safety
    /// No handles or edges may refer to storage reclaimed by this non-tracing sweep.
    pub unsafe fn sweep_storage(&mut self) -> Result<(), AllocError> {
        assert!(
            self.resources.borrow().is_empty(),
            "storage driver cannot sweep typed resources"
        );
        self.allocator.borrow_mut().sweep()
    }
    /// Live roots grouped by their owner tags, for leak/plateau diagnostics.
    /// This reads the sole root table; it creates no additional roots.
    pub fn root_census(&self) -> Vec<(&'static str, usize)> {
        let mut counts = std::collections::BTreeMap::new();
        for entry in self.tables.roots.borrow().iter().flatten() {
            *counts.entry(entry.owner).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }
    pub fn root_count(&self) -> usize {
        self.tables
            .roots
            .borrow()
            .iter()
            .filter(|r| r.is_some())
            .count()
    }
}
impl Drop for Heap {
    fn drop(&mut self) {
        self.tables.alive.set(false);
        for resource in self.resources.get_mut().drain(..) {
            // SAFETY: heap teardown owns every resource; all mappings still exist.
            unsafe {
                if let objects::Resource::Foreign(w) = resource {
                    raw::ForeignHooks::drop(
                        &mut **self
                            .foreign_hooks
                            .get_mut()
                            .as_mut()
                            .expect("foreign hooks"),
                        raw::RawValue::from_value(Value::word(w)),
                    );
                } else {
                    resource.drop();
                }
            }
        }
        debug_assert!(
            self.root_count() == 0
                && self
                    .tables
                    .stacks
                    .borrow()
                    .iter()
                    .all(|s| s.storage.is_none()),
            "heap dropped with live handles"
        );
    }
}
/// Starts a fresh OS thread and constructs its only heap. Heap-bound types
/// cannot cross the `Send` boundary, including as the callback's return value.
pub fn spawn_isolate<F, R>(config: HeapConfig, f: F) -> JoinHandle<R>
where
    F: FnOnce(Heap) -> R + Send + 'static,
    R: Send + 'static,
{
    thread::spawn(move || f(Heap::create(config)))
}

/// Pure subr signal: copy its data into a frame before a host callback.
#[derive(Clone, Copy, Debug)]
pub struct Signal<'h> {
    pub symbol: Value<'h>,
    pub data: Value<'h>,
}
pub type PureFn = for<'h> fn(Args<'h>) -> Result<Value<'h>, Signal<'h>>;
/// Non-local control travels as an unbranded discriminator; Lisp data remains
/// in the registered frame. Area/evaluator semantics are outside this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Signal,
    Throw,
}

#[cfg(test)]
mod foreign_tests;
#[cfg(test)]
mod tests;
