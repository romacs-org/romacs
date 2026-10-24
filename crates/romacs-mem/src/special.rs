//! GNU-ordered special edges. All decisions happen inside the exclusive stop.
use crate::{
    AllocError, Heap, Kind, PvecType, Root, Value,
    objects::*,
    raw::{RawValue, Weakness},
};
use std::{
    rc::Rc,
    sync::atomic::{AtomicU8, Ordering},
};

/// Heap-bound weak identity. IDs are never recycled; a cleared ID cannot alias a new cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WeakId {
    heap: u64,
    entry: u64,
}
impl WeakId {
    /// Resolve a copied identity; foreign heaps and retired entries return None.
    /// Sendable IDs identify handle entries, not objects and not rooting authority.
    pub fn resolve(self, heap: &Heap) -> Option<Value<'_>> {
        if self.heap != heap.tables.id {
            return None;
        }
        heap.tables
            .weak_refs
            .borrow()
            .get(&self.entry)
            .copied()
            .map(Value::word)
    }
}
/// Non-owning handle. Registry entries are cleared before reclamation.
pub struct WeakRef {
    tables: Rc<crate::handles::Tables>,
    id: WeakId,
}
impl WeakRef {
    pub fn new(heap: &Heap, value: Value<'_>) -> Self {
        heap.assert_value(value.0);
        let next = heap
            .tables
            .next_weak
            .get()
            .checked_add(1)
            .expect("weak identity exhausted");
        heap.tables.next_weak.set(next);
        let id = WeakId {
            heap: heap.tables.id,
            entry: next,
        };
        heap.tables.weak_refs.borrow_mut().insert(id.entry, value.0);
        Self {
            tables: heap.tables.clone(),
            id,
        }
    }
    pub fn id(&self) -> WeakId {
        self.id
    }
    pub fn upgrade<'h>(&self, heap: &'h Heap) -> Option<Value<'h>> {
        assert!(
            Rc::ptr_eq(&self.tables, &heap.tables) && self.tables.alive.get(),
            "foreign/dead weak handle"
        );
        self.id.resolve(heap)
    }
    pub fn is(&self, heap: &Heap, value: Value<'_>) -> bool {
        self.upgrade(heap) == Some(value)
    }
}
impl Drop for WeakRef {
    fn drop(&mut self) {
        self.tables.weak_refs.borrow_mut().remove(&self.id.entry);
    }
}

/// Fixed-capacity native weak entries; Lisp edges stay in native Slots, not Rust payloads.
#[derive(Clone, Copy)]
pub struct WeakPairsRef<'h> {
    heap: &'h Heap,
    word: u64,
}
impl<'h> WeakPairsRef<'h> {
    fn header(self) -> &'h VectorHeader {
        // SAFETY: constructed from a validated live WEAK vector and protected by heap borrow.
        unsafe { &*((self.word & !7) as *const VectorHeader) }
    }
    pub fn capacity(self) -> usize {
        self.header().lisp_slots as usize / 2
    }
    fn occupied(self, index: usize) -> &'h AtomicU8 {
        assert!(index < self.capacity());
        // SAFETY: raw tail has weakness byte followed by capacity initialized AtomicU8s.
        unsafe { &*raw_ptr(self.word).add(index + 1).cast::<AtomicU8>() }
    }
    pub fn get(self, index: usize) -> Option<(Value<'h>, Value<'h>)> {
        if index >= self.capacity() || self.occupied(index).load(Ordering::Relaxed) == 0 {
            return None;
        }
        let v = self
            .heap
            .as_vectorlike(Value::word(self.word))
            .expect("weak vector");
        Some((v.get(index * 2)?, v.get(index * 2 + 1)?))
    }
    pub fn set(self, index: usize, key: Value<'h>, value: Value<'h>) -> bool {
        if index >= self.capacity() {
            return false;
        }
        let v = self
            .heap
            .as_vectorlike(Value::word(self.word))
            .expect("weak vector");
        v.set(index * 2, key);
        v.set(index * 2 + 1, value);
        self.occupied(index).store(1, Ordering::Relaxed);
        true
    }
    pub fn len(self) -> usize {
        (0..self.capacity())
            .filter(|i| self.get(*i).is_some())
            .count()
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    fn weakness(self) -> Weakness {
        // SAFETY: constructor wrote a valid discriminant in the immutable first tail byte.
        match unsafe { *raw_ptr(self.word) } {
            0 => Weakness::Key,
            1 => Weakness::Value,
            2 => Weakness::KeyOrValue,
            3 => Weakness::KeyAndValue,
            _ => unreachable!(),
        }
    }
}
impl Heap {
    pub fn make_weak_pairs(
        &self,
        weakness: Weakness,
        capacity: usize,
    ) -> Result<Value<'_>, AllocError> {
        let n = capacity.checked_mul(2).ok_or(AllocError::MemoryFull)?;
        let raw = capacity.checked_add(1).ok_or(AllocError::MemoryFull)?;
        if n > u32::MAX as usize {
            return Err(AllocError::MemoryFull);
        }
        let value = self.vector(PvecType::HashTable, &vec![self.nil(); n], raw, WEAK)?;
        // SAFETY: fresh tail fits one discriminant and capacity zero occupancy bytes.
        unsafe {
            *raw_ptr(value.0) = match weakness {
                Weakness::Key => 0,
                Weakness::Value => 1,
                Weakness::KeyOrValue => 2,
                Weakness::KeyAndValue => 3,
            };
        }
        Ok(value)
    }
    pub fn as_weak_pairs<'h>(&'h self, value: Value<'h>) -> Option<WeakPairsRef<'h>> {
        let ptr = self.allocator.borrow().address(value.0)?;
        if value.kind() != Kind::Vectorlike {
            return None;
        }
        // SAFETY: validated vectorlike allocation has an initialized header.
        if unsafe { ptr.cast::<VectorHeader>().as_ref().flags } != WEAK {
            return None;
        }
        Some(WeakPairsRef {
            heap: self,
            word: value.0,
        })
    }
    /// Native finalizer. The function is a strong edge only when the finalizer is marked.
    pub fn make_finalizer<'h>(&'h self, function: Value<'h>) -> Result<Value<'h>, AllocError> {
        let value = self.vector(PvecType::Finalizer, &[function], 0, 0)?;
        self.gc.borrow_mut().finalizers.push(value.0);
        Ok(value)
    }
    /// Run after the sweep, in the owner wrapper. Dequeue and clear before callbacks,
    /// including recursive collections; the callback receives a rooted function.
    pub fn run_finalizers(&mut self, mut run: impl FnMut(&mut Heap, &Root)) -> usize {
        let mut count = 0;
        while let Some(word) = self.gc.get_mut().doomed.pop_front() {
            let function = self.finalizer_function(word);
            let root = Root::new(self, function, "finalizer callback");
            if !self.is_foreign(RawValue::from_value(Value::word(word))) {
                self.as_vectorlike(Value::word(word))
                    .expect("finalizer")
                    .set(0, self.nil());
            }
            if !function.is_nil() {
                count += 1;
                run(self, &root);
            }
        }
        count
    }
    fn finalizer_function(&self, w: u64) -> Value<'_> {
        if self.is_foreign(RawValue::from_value(Value::word(w))) {
            let function = self
                .foreign_hooks
                .borrow()
                .as_ref()
                .expect("foreign hooks")
                .finalizer_function(RawValue::from_value(Value::word(w)));
            Value::word(function.0)
        } else {
            self.as_vectorlike(Value::word(w))
                .expect("finalizer")
                .get(0)
                .expect("function")
        }
    }
    pub(crate) fn survives(&self, w: u64) -> bool {
        if self.external_symbol(w) {
            return w >> 3 < crate::sym::BUILTIN_COUNT as u64
                || self
                    .gc
                    .borrow()
                    .external_marks
                    .get(&(w >> 9))
                    .is_some_and(|bits| bits & (1 << ((w >> 3) & 63)) != 0)
                || self
                    .foreign_hooks
                    .borrow()
                    .as_ref()
                    .is_some_and(|h| h.symbol_pinned(RawValue::from_value(Value::word(w))));
        }
        w & 3 == 2 || self.static_leaf(w) || self.allocator.borrow().marked(w)
    }
    pub(crate) fn drain_mark(&mut self, work: &mut Vec<RawValue>, weak: &mut Vec<u64>) -> bool {
        let mut changed = false;
        while let Some(v) = work.pop() {
            let w = v.0;
            if w & 3 == 2 {
                continue;
            }
            if self.external_symbol(w) {
                let bits = self.gc.get_mut().external_marks.entry(w >> 9).or_default();
                let mask = 1 << ((w >> 3) & 63);
                changed |= *bits & mask == 0;
                *bits |= mask;
                continue;
            }
            let ptr = self.allocator.borrow().address(w);
            let Some(ptr) = ptr else {
                assert!(self.valid_word(w), "Dangling edge {{word: {w}}}");
                continue;
            };
            if self.allocator.borrow().marked(w) {
                continue;
            }
            assert!(self.allocator.borrow_mut().mark(w));
            changed = true;
            // SAFETY: allocator address above validated this live cell.
            unsafe { self.edges_at(w, ptr, work) };
            if w & 7 == 5 {
                // SAFETY: newly marked initialized vector; exclusive stop permits legacy weak access.
                unsafe {
                    let flags = ptr.cast::<VectorHeader>().as_ref().flags;
                    if flags == WEAK || flags & FOREIGN != 0 && flags & 8 != 0 {
                        weak.push(w);
                    }
                }
            }
        }
        changed
    }
    fn undo_value(&self, word: u64) -> Value<'_> {
        assert!(self.valid_word(word), "Dangling {{word: {word}}}");
        Value::word(word)
    }
    pub(crate) fn compact_undo(&self, head: &mut RawValue) {
        let mut tail = head.0;
        let mut previous = None;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(cell) = self.as_cons(self.undo_value(tail)) {
            assert!(seen.insert(tail), "cyclic undo list");
            let entry = self.undo_value(cell.car().0);
            let marker = self.as_cons(entry).map(|c| self.undo_value(c.car().0));
            let dead = marker.is_some_and(|v| {
                let is_marker = self.as_vectorlike(v).is_some_and(|view| {
                    if self.is_foreign(RawValue::from_value(v)) {
                        self.foreign_hooks
                            .borrow()
                            .as_ref()
                            .expect("foreign hooks")
                            .is_marker(RawValue::from_value(v))
                    } else {
                        view.pvec_type() == PvecType::Marker
                    }
                });
                is_marker && !self.survives(v.0)
            });
            let next = self.undo_value(cell.cdr().0).0;
            if dead {
                if let Some(prev) = previous {
                    self.as_cons(Value::word(prev))
                        .expect("previous cons")
                        .set_cdr(Value::word(next));
                } else {
                    *head = RawValue::from_value(Value::word(next));
                }
            } else {
                previous = Some(tail);
            }
            tail = next;
        }
    }
    pub(crate) fn doom_finalizers(&mut self, work: &mut Vec<RawValue>) {
        // Discovery must finish before any doomed function graph is marked.
        let all = std::mem::take(&mut self.gc.get_mut().finalizers);
        for w in all {
            if self.survives(w) {
                self.gc.get_mut().finalizers.push(w);
            } else if !self.finalizer_function(w).is_nil() {
                self.gc.get_mut().doomed.push_back(w);
            }
        }
        work.extend(
            self.gc
                .get_mut()
                .doomed
                .iter()
                .map(|w| RawValue::from_value(Value::word(*w))),
        );
    }
    pub(crate) fn weak_entries(&mut self, w: u64) -> (Weakness, Vec<(RawValue, RawValue)>) {
        let mut pairs = Vec::new();
        if let Some(native) = self.as_weak_pairs(Value::word(w)) {
            for i in 0..native.capacity() {
                if let Some((k, v)) = native.get(i) {
                    pairs.push((RawValue::from_value(k), RawValue::from_value(v)));
                }
            }
            (native.weakness(), pairs)
        } else {
            let mut policy = None;
            self.foreign_hooks
                .get_mut()
                .as_mut()
                .expect("foreign hooks")
                .with_weak(RawValue::from_value(Value::word(w)), &mut |table| {
                    assert!(policy.is_none(), "one weak table per foreign cell");
                    policy = Some(table.weakness());
                    table.entries(&mut |k, v| pairs.push((k, v)));
                });
            (policy.expect("registered weak container"), pairs)
        }
    }
    pub(crate) fn weak_fixpoint(&mut self, weak: &mut Vec<u64>) {
        loop {
            let mut work = Vec::new();
            for w in weak.clone() {
                let (policy, pairs) = self.weak_entries(w);
                for (k, v) in pairs {
                    if keep_entry(policy, self.survives(k.0), self.survives(v.0)) {
                        work.extend([k, v]);
                    }
                }
            }
            if !self.drain_mark(&mut work, weak) {
                break;
            }
        }
        for &w in weak.iter() {
            if let Some(native) = self.as_weak_pairs(Value::word(w)) {
                for i in 0..native.capacity() {
                    if let Some((k, v)) = native.get(i)
                        && !keep_entry(native.weakness(), self.survives(k.0), self.survives(v.0))
                    {
                        let vector = self.as_vectorlike(Value::word(w)).expect("weak vector");
                        vector.set(i * 2, self.nil());
                        vector.set(i * 2 + 1, self.nil());
                        native.occupied(i).store(0, Ordering::Relaxed);
                    }
                }
            } else {
                let (policy, pairs) = self.weak_entries(w);
                let kept: std::collections::BTreeSet<_> = pairs
                    .into_iter()
                    .filter(|(k, v)| keep_entry(policy, self.survives(k.0), self.survives(v.0)))
                    .map(|(k, v)| (k.bits(), v.bits()))
                    .collect();
                self.foreign_hooks
                    .get_mut()
                    .as_mut()
                    .expect("foreign hooks")
                    .with_weak(RawValue::from_value(Value::word(w)), &mut |table| {
                        table.remove_dead(&|k, v| kept.contains(&(k.bits(), v.bits())));
                    });
                self.account_resource(Resource::Foreign(w));
            }
        }
    }
}
fn keep_entry(weakness: Weakness, key: bool, value: bool) -> bool {
    match weakness {
        Weakness::Key => key,
        Weakness::Value => value,
        Weakness::KeyOrValue => key || value,
        Weakness::KeyAndValue => key && value,
    }
}
#[cfg(test)]
#[path = "special_tests.rs"]
mod tests;
