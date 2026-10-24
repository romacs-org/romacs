//! Temporary, explicitly unsafe bridge for inherited object representations.
//! Adapters are counted at the switch and removed as their area is rewritten.
use crate::{Heap, Value};
use std::marker::PhantomData;

/// Unbranded edge for owner records and legacy adapters. Not a rooting handle.
/// Mutator-local (!Send + !Sync), so native Payloads cannot hide these edges.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawValue(pub(crate) u64, PhantomData<*const ()>);
impl RawValue {
    pub fn bits(self) -> u64 {
        self.0
    }
    /// # Safety
    /// The word must be valid in the receiving heap.
    pub unsafe fn from_bits(word: u64) -> Self {
        Self(word, PhantomData)
    }
    pub fn from_value(value: Value<'_>) -> Self {
        Self(value.0, PhantomData)
    }
    /// Reborrow a live edge from its owning heap.
    /// # Safety
    /// The word must belong to `heap` and remain allocated (or be immediate).
    pub unsafe fn value(self, heap: &Heap) -> Value<'_> {
        heap.assert_value(self.0);
        Value::word(self.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ForeignKind {
    String,
    Vectorlike,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForeignFlags(u8);
impl ForeignFlags {
    pub const NONE: Self = Self(0);
    pub const WEAK_TABLE: Self = Self(8);
    pub const FINALIZER: Self = Self(16);
}
/// Per-heap owner dispatch for in-place inherited cells.
/// # Safety
/// Report all strong edges, no weak edges. Never allocate, invoke Lisp or retain
/// callbacks. Access only initialized owner bytes, never overwrite mem headers.
/// Do not resize owner backing storage during trace; weak removal may shrink it.
/// Destruction must not access the heap. No interior borrows may cross collection.
/// For FINALIZER cells, exclude the function from trace: mem traces it only
/// while registered or doomed. with_weak must invoke its callback exactly once.
/// Initialize all owner fields immediately after alloc_foreign, before calling
/// any other heap operation (including accounting or teardown).
pub unsafe trait ForeignHooks: 'static {
    fn trace(&self, word: RawValue, marker: &mut Marker<'_>);
    fn with_weak(&mut self, word: RawValue, f: &mut dyn FnMut(&mut dyn LegacyWeak));
    fn finalizer_function(&self, word: RawValue) -> RawValue;
    fn drop(&mut self, word: RawValue);
    fn bytes(&self, word: RawValue) -> usize;
    fn symbol_pinned(&self, word: RawValue) -> bool;
    /// Classify an initialized foreign cell using the owner's layout, not mem's
    /// `Other` overlay. Undo compaction reads current-cycle liveness from mem's
    /// marks before tracing undo entries; this hook must not mark any edges.
    /// The default is valid only for owners with no foreign markers.
    fn is_marker(&self, _word: RawValue) -> bool {
        false
    }
}
impl Heap {
    /// # Safety
    /// The owner confines this heap and all its words to the current thread.
    pub unsafe fn for_legacy_owner(config: crate::HeapConfig) -> Heap {
        Heap::create(config)
    }
    /// # Safety
    /// Hooks must implement the layouts of every foreign cell in this heap.
    pub unsafe fn set_foreign_hooks(&mut self, hooks: Box<dyn ForeignHooks>) {
        assert!(
            !self
                .resources
                .get_mut()
                .iter()
                .any(|r| matches!(r, crate::objects::Resource::Foreign(_))),
            "foreign cells already allocated"
        );
        *self.foreign_hooks.get_mut() = Some(hooks);
    }
    /// # Safety
    /// Initialize owner bytes before any heap operation; preserve the first
    /// 16 bytes of vectorlikes. All edges and flags must match the hooks.
    pub unsafe fn alloc_foreign(
        &self,
        kind: ForeignKind,
        bytes: usize,
        flags: ForeignFlags,
    ) -> Result<RawValue, crate::AllocError> {
        use crate::objects::*;
        assert!(
            self.foreign_hooks.borrow().is_some(),
            "install foreign hooks first"
        );
        if bytes < 24 || (matches!(kind, ForeignKind::String) && flags.0 != 0) {
            return Err(crate::AllocError::InvalidSize);
        }
        let word = self.allocator.borrow_mut().alloc_foreign(kind, bytes)?;
        if matches!(kind, ForeignKind::Vectorlike) {
            // SAFETY: fresh cell fits the overlay; owner initializes the remainder.
            unsafe {
                ((word & !7) as *mut VectorHeader).write(VectorHeader {
                    pvec: crate::PvecType::Other,
                    flags: FOREIGN | flags.0,
                    reserved: 0,
                    lisp_slots: 0,
                    raw_bytes: (bytes - 16) as u64,
                });
            }
        }
        self.resources.borrow_mut().push(Resource::Foreign(word));
        if flags.0 & ForeignFlags::FINALIZER.0 != 0 {
            self.gc.borrow_mut().finalizers.push(word);
        }
        Ok(RawValue::from_value(Value::word(word)))
    }
    /// # Safety
    /// Both edges must be valid words belonging to this heap.
    pub unsafe fn alloc_cons_raw(
        &self,
        car: RawValue,
        cdr: RawValue,
    ) -> Result<RawValue, crate::AllocError> {
        self.cons(Value::word(car.0), Value::word(cdr.0))
            .map(RawValue::from_value)
    }
    pub fn is_foreign(&self, word: RawValue) -> bool {
        let allocator = self.allocator.borrow();
        let Some(ptr) = allocator.address(word.0) else {
            return false;
        };
        match word.0 & 7 {
            // SAFETY: address validated the live string allocation above.
            4 => unsafe { crate::alloc::Allocator::foreign_string(word.0) },
            // SAFETY: allocated vectorlike has a mem-owned initialized header.
            5 => unsafe {
                ptr.cast::<crate::objects::VectorHeader>().as_ref().flags & crate::objects::FOREIGN
                    != 0
            },
            _ => false,
        }
    }
    pub(crate) fn external_symbol(&self, word: u64) -> bool {
        self.config.external_symbols && word & 7 == 0
    }
    pub(crate) fn static_leaf(&self, word: u64) -> bool {
        self.static_ranges
            .iter()
            .any(|r| r.contains(&((word & !7) as usize)))
    }
    pub(crate) fn foreign_flags(&self, word: u64) -> u8 {
        if word & 7 != 5 {
            return 0;
        }
        let allocator = self.allocator.borrow();
        let ptr = allocator.address(word).expect("Dangling edge");
        // SAFETY: live vectorlike, header belongs to mem.
        unsafe { ptr.cast::<crate::objects::VectorHeader>().as_ref().flags }
    }
    /// # Safety
    /// Range must contain only permanent initialized leaf objects, not heap cells.
    pub unsafe fn add_static_range(&mut self, start: usize, len: usize) {
        self.static_ranges
            .push(start..start.checked_add(len).expect("static range overflow"));
    }
}

/// Collector work sink. No heap access or Lisp callbacks are exposed.
pub struct Marker<'a> {
    pub(crate) work: &'a mut Vec<RawValue>,
}
impl Marker<'_> {
    pub fn mark(&mut self, value: RawValue) {
        self.work.push(value);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weakness {
    Key,
    Value,
    KeyOrValue,
    KeyAndValue,
}

/// GNU-shaped weak-entry adapter; policy belongs to the collector, not adapters.
/// # Safety
/// Entries must be valid words from the containing heap. Removal must preserve
/// count/index/free-list consistency and use stored hashes, never rehash keys.
/// No heap allocation, Lisp calls, or retained callbacks are allowed.
pub unsafe trait LegacyWeak {
    fn weakness(&self) -> Weakness;
    fn entries(&self, visit: &mut dyn FnMut(RawValue, RawValue));
    fn remove_dead(&mut self, keep: &dyn Fn(RawValue, RawValue) -> bool) -> u32;
}
