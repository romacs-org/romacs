use crate::{Op, SetId};
use hashbrown::HashTable;
use std::{
    cell::Cell,
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hash},
};

const TRANSITIONS: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Transition<V> {
    Add { from: SetId, key: V, value: V },
    Remove { from: SetId, key: V },
}

struct Entry<V> {
    pairs: Box<[(V, V)]>,
    flags: u8,
    mark: Cell<u32>,
    born: u32,
    list: Option<V>,
}
/// Heap-global ordered plists. Values must implement Lisp `eq` equality.
/// `intern` renews one-collection grace, including hits. Attach returned IDs to
/// traced Props before a second collection; other returned IDs have no new grace.
/// Cached Lisp lists are owner-controlled read-only values shared by all runs.
/// The owner must enforce immutable conses or copy before exposing them to Lisp;
/// arbitrary `setcar`/`setcdr` would invalidate the cache without updating `pairs`.
pub struct PropSets<V> {
    entries: Vec<Option<Entry<V>>>,
    // Only IDs live in the index; each ordered pairs slice has a single owner.
    index: HashTable<SetId>,
    hasher: RandomState,
    free: Vec<u32>,
    epoch: u32,
    classify: fn(V, V) -> u8,
    transitions: [Option<(Transition<V>, SetId)>; TRANSITIONS],
}
impl<V: Copy + Eq + Hash> Default for PropSets<V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<V: Copy + Eq + Hash> PropSets<V> {
    pub const FRONT_STICKY: u8 = 1 << 0;
    pub const REAR_NONSTICKY: u8 = 1 << 1;
    pub const READ_ONLY: u8 = 1 << 2;
    pub const INVISIBLE: u8 = 1 << 3;
    pub const HAS_SYNTAX_OR_CATEGORY: u8 = 1 << 4;
    pub const HAS_DISPLAY: u8 = 1 << 5;
    pub const HAS_FACE: u8 = 1 << 6;

    /// Construct an interner without property classification (all flags zero).
    pub fn new() -> Self {
        Self::with_flag_classifier(|_, _| 0)
    }
    /// Classify each key/value pair; a set's flags are the bitwise OR of them.
    ///
    /// The classifier must be deterministic for a pair throughout this interner's
    /// lifetime. The Lisp owner supplies symbol/nil semantics; generic values
    /// cannot identify property names. Every operation, including remove and
    /// union, reclassifies the resulting pairs on an interning miss.
    pub fn with_flag_classifier(classify: fn(V, V) -> u8) -> Self {
        let mut s = Self {
            entries: Vec::new(),
            index: HashTable::new(),
            hasher: RandomState::new(),
            free: Vec::new(),
            epoch: 1,
            classify,
            transitions: [None; TRANSITIONS],
        };
        s.intern(&[]);
        s
    }
    pub fn live(&self) -> usize {
        self.index.len()
    }
    /// Collection generation for consumers caching data by SetId. Advances on
    /// every sweep, even if no sets are freed; wraps from u32::MAX to 1.
    #[inline]
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    #[inline]
    pub fn flags(&self, id: SetId) -> u8 {
        self.entry(id).flags
    }
    pub fn intern(&mut self, pairs: &[(V, V)]) -> SetId {
        let hash = self.hasher.hash_one(pairs);
        if let Some(&id) = self.index.find(hash, |id| {
            self.entries[id.0 as usize]
                .as_ref()
                .expect("live set")
                .pairs
                .as_ref()
                == pairs
        }) {
            self.entries[id.0 as usize].as_mut().expect("live set").born = self.epoch;
            return id;
        }
        let flags = pairs
            .iter()
            .fold(0, |flags, &(k, v)| flags | (self.classify)(k, v));
        let id = SetId(self.free.pop().unwrap_or_else(|| {
            let n = u32::try_from(self.entries.len()).expect("set id exhaustion");
            self.entries.push(None);
            n
        }));
        self.entries[id.0 as usize] = Some(Entry {
            pairs: pairs.into(),
            flags,
            mark: Cell::new(0),
            born: self.epoch,
            list: None,
        });
        let entries = &self.entries;
        let hasher = &self.hasher;
        self.index.insert_unique(hash, id, |id| {
            hasher.hash_one(
                entries[id.0 as usize]
                    .as_ref()
                    .expect("live set")
                    .pairs
                    .as_ref(),
            )
        });
        id
    }
    fn entry(&self, id: SetId) -> &Entry<V> {
        self.entries[id.0 as usize].as_ref().expect("dead set")
    }
    #[inline]
    pub fn pairs(&self, id: SetId) -> &[(V, V)] {
        &self.entry(id).pairs
    }
    #[inline]
    pub fn get(&self, id: SetId, key: V) -> Option<V> {
        self.pairs(id).iter().find(|p| p.0 == key).map(|p| p.1)
    }
    #[inline]
    pub fn set_eq(&self, a: SetId, b: SetId) -> bool {
        a == b
            || (self.pairs(a).len() == self.pairs(b).len()
                && self
                    .pairs(a)
                    .iter()
                    .all(|&(k, v)| self.get(b, k) == Some(v)))
    }
    pub fn list(&self, id: SetId) -> Option<V> {
        self.entry(id).list
    }
    pub fn set_list(&mut self, id: SetId, list: V) {
        self.entries[id.0 as usize].as_mut().expect("dead set").list = Some(list);
    }
    fn transition_slot(&self, op: Transition<V>) -> usize {
        (self.hasher.hash_one(op) & (TRANSITIONS as u64 - 1)) as usize
    }
    pub fn with(&mut self, id: SetId, key: V, value: V) -> SetId {
        let op = Transition::Add {
            from: id,
            key,
            value,
        };
        let slot = self.transition_slot(op);
        if let Some((cached, to)) = self.transitions[slot]
            && cached == op
        {
            return to;
        }
        let to = if self.get(id, key) == Some(value) {
            self.intern_existing(id)
        } else {
            let mut out = self.pairs(id).to_vec();
            if let Some(p) = out.iter_mut().find(|p| p.0 == key) {
                p.1 = value;
            } else {
                out.insert(0, (key, value));
            }
            self.intern(&out)
        };
        self.transitions[slot] = Some((op, to));
        to
    }
    fn intern_existing(&mut self, id: SetId) -> SetId {
        self.entries[id.0 as usize].as_mut().expect("dead set").born = self.epoch;
        id
    }
    pub fn with_pairs(&mut self, id: SetId, pairs: &[(V, V)]) -> SetId {
        if let &[(key, value)] = pairs {
            return self.with(id, key, value);
        }
        let mut out = self.pairs(id).to_vec();
        for &(k, v) in pairs {
            if let Some(p) = out.iter_mut().find(|p| p.0 == k) {
                p.1 = v;
            } else {
                out.insert(0, (k, v));
            }
        }
        self.intern(&out)
    }
    pub fn without(&mut self, id: SetId, keys: &[V]) -> SetId {
        let cached = if let &[key] = keys {
            let op = Transition::Remove { from: id, key };
            let slot = self.transition_slot(op);
            if let Some((cached, to)) = self.transitions[slot]
                && cached == op
            {
                return to;
            }
            Some((slot, op))
        } else {
            None
        };
        let to = if !keys.iter().any(|&key| self.get(id, key).is_some()) {
            self.intern_existing(id)
        } else {
            let out: Vec<_> = self
                .pairs(id)
                .iter()
                .copied()
                .filter(|p| !keys.contains(&p.0))
                .collect();
            self.intern(&out)
        };
        if let Some((slot, op)) = cached {
            self.transitions[slot] = Some((op, to));
        }
        to
    }
    pub fn union_under(&mut self, target: SetId, src: SetId) -> SetId {
        let mut out = self.pairs(target).to_vec();
        for &(k, v) in self.pairs(src) {
            if !out.iter().any(|p| p.0 == k) {
                out.insert(0, (k, v));
            }
        }
        self.intern(&out)
    }
    pub(crate) fn changed(&self, id: SetId, op: &Op<'_, V>) -> bool {
        match op {
            Op::Set(to) => id != *to,
            Op::Add(ps) => ps.iter().any(|&(k, v)| self.get(id, k) != Some(v)),
            Op::Remove(ks) => ks.iter().any(|&k| self.get(id, k).is_some()),
        }
    }
    pub(crate) fn transform(&mut self, id: SetId, op: &Op<'_, V>) -> SetId {
        match op {
            Op::Set(to) => *to,
            Op::Add(ps) => self.with_pairs(id, ps),
            Op::Remove(ks) => self.without(id, ks),
        }
    }
    pub(crate) fn trace_set(&self, id: SetId, mark: &mut impl FnMut(V)) {
        let e = self.entry(id);
        if e.mark.replace(self.epoch) == self.epoch {
            return;
        }
        for &(k, v) in &e.pairs {
            mark(k);
            mark(v);
        }
        if let Some(l) = e.list {
            mark(l);
        }
    }
    pub fn trace_young(&self, mark: &mut impl FnMut(V)) {
        for (i, e) in self.entries.iter().enumerate() {
            if e.as_ref().is_some_and(|e| e.born == self.epoch) {
                self.trace_set(SetId(i as u32), mark);
            }
        }
    }
    pub fn sweep(&mut self) -> usize {
        // Cached IDs are not roots, and must never survive reuse of a free slot.
        self.transitions.fill(None);
        let mut freed = 0;
        for (i, slot) in self.entries.iter_mut().enumerate().skip(1) {
            if slot
                .as_ref()
                .is_some_and(|e| e.mark.get() != self.epoch && e.born != self.epoch)
            {
                let e = slot.as_ref().expect("live entry");
                let hash = self.hasher.hash_one(e.pairs.as_ref());
                self.index
                    .find_entry(hash, |id| id.0 as usize == i)
                    .expect("indexed live entry")
                    .remove();
                *slot = None;
                self.free.push(i as u32);
                freed += 1;
            }
        }
        if self.epoch == u32::MAX {
            for e in self.entries.iter_mut().flatten() {
                e.mark.set(0);
                e.born = 0;
            }
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(key: u32, value: u32) -> u8 {
        if value == 0 {
            return 0;
        }
        match key {
            1 => PropSets::<u32>::HAS_FACE,
            2 => PropSets::<u32>::READ_ONLY,
            3 => PropSets::<u32>::HAS_DISPLAY,
            _ => 0,
        }
    }

    #[test]
    fn index_grows_deduplicates_and_preserves_order() {
        let mut sets = PropSets::new();
        let ids: Vec<_> = (1..1024).map(|v| sets.intern(&[(1u32, v)])).collect();
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(sets.intern(&[(1, i as u32 + 1)]), id);
        }
        assert_eq!(sets.live(), 1024);
        let a = sets.intern(&[(1, 2), (3, 4)]);
        let b = sets.intern(&[(3, 4), (1, 2)]);
        assert_ne!(a, b);
        assert!(sets.set_eq(a, b));
        let c = sets.with_pairs(a, &[(1, 5), (6, 7), (8, 9), (6, 10)]);
        assert_eq!(sets.pairs(c), &[(8, 9), (6, 10), (1, 5), (3, 4)]);
    }

    #[test]
    fn flags_follow_all_transformations() {
        let mut sets = PropSets::with_flag_classifier(classify);
        assert_eq!(sets.flags(SetId::EMPTY), 0);
        let a = sets.intern(&[(1, 1), (2, 0)]);
        assert_eq!(sets.flags(a), PropSets::<u32>::HAS_FACE);
        let b = sets.with(a, 2, 1);
        assert_eq!(
            sets.flags(b),
            PropSets::<u32>::HAS_FACE | PropSets::<u32>::READ_ONLY
        );
        let c = sets.without(b, &[1]);
        assert_eq!(sets.flags(c), PropSets::<u32>::READ_ONLY);
        let d = sets.with_pairs(c, &[(2, 0), (3, 1)]);
        assert_eq!(sets.flags(d), PropSets::<u32>::HAS_DISPLAY);
        let e = sets.union_under(d, a);
        assert_eq!(
            sets.flags(e),
            PropSets::<u32>::HAS_DISPLAY | PropSets::<u32>::HAS_FACE
        );
        let mut unclassified = PropSets::<u32>::new();
        let id = unclassified.intern(&[(1, 1)]);
        assert_eq!(unclassified.flags(id), 0);
    }

    #[test]
    fn direct_mapped_cache_checks_full_keys_and_retains_multiple_slots() {
        let mut sets = PropSets::<u32>::new();
        let first = Transition::Add {
            from: SetId::EMPTY,
            key: 1,
            value: 1,
        };
        let slot = sets.transition_slot(first);
        let mut collision = None;
        let mut different = None;
        for key in 2..10000 {
            let op = Transition::Add {
                from: SetId::EMPTY,
                key,
                value: key,
            };
            if sets.transition_slot(op) == slot {
                collision = Some(key);
            } else {
                different = Some(key);
            }
            if collision.is_some() && different.is_some() {
                break;
            }
        }
        let collision = collision.expect("direct-map collision");
        let different = different.expect("distinct direct-map slot");
        let a = sets.with(SetId::EMPTY, 1, 1);
        let b = sets.with(SetId::EMPTY, different, different);
        assert!(sets.transitions[slot].is_some_and(|(op, to)| op == first && to == a));
        assert_eq!(sets.with(SetId::EMPTY, 1, 1), a);
        let c = sets.with(SetId::EMPTY, collision, collision);
        assert_eq!(sets.pairs(c), &[(collision, collision)]);
        assert_eq!(sets.with(SetId::EMPTY, 1, 1), a);
        assert_eq!(sets.with(SetId::EMPTY, different, different), b);
        let changed = sets.with(a, 1, 2);
        assert_eq!(sets.pairs(changed), &[(1, 2)]);
        assert_eq!(sets.without(changed, &[1]), SetId::EMPTY);
        assert_eq!(sets.without(changed, &[1]), SetId::EMPTY);
        let other = sets.with(b, 1, 1);
        assert_eq!(sets.get(other, different), Some(different));
    }

    #[test]
    fn sweep_clears_cache_before_ids_are_reused() {
        let mut sets = PropSets::with_flag_classifier(classify);
        let a = sets.with(SetId::EMPTY, 1, 1);
        assert_eq!(sets.sweep(), 0);
        let epoch = sets.epoch();
        assert_eq!(sets.sweep(), 1);
        assert_ne!(sets.epoch(), epoch);
        assert!(sets.transitions.iter().all(Option::is_none));
        let b = sets.intern(&[(2, 1)]);
        assert_eq!(a, b);
        assert_eq!(sets.flags(b), PropSets::<u32>::READ_ONLY);
        let c = sets.with(SetId::EMPTY, 1, 1);
        assert_ne!(b, c);
        assert_eq!(sets.pairs(c), &[(1, 1)]);
        assert_eq!(sets.live(), 3);
    }

    #[test]
    fn gc_grace_trace_once_lists_and_epoch_wrap() {
        let mut sets = PropSets::<u32>::new();
        let id = sets.intern(&[(1, 2)]);
        sets.set_list(id, 99);
        let mut traced = Vec::new();
        sets.trace_young(&mut |v| traced.push(v));
        sets.trace_set(id, &mut |v| traced.push(v));
        assert_eq!(traced, [1, 2, 99]);
        assert_eq!(sets.sweep(), 0);
        assert_eq!(sets.intern(&[(1, 2)]), id);
        assert_eq!(sets.sweep(), 0);
        sets.trace_set(id, &mut |_| {});
        assert_eq!(sets.sweep(), 0);
        assert_eq!(sets.sweep(), 1);
        sets.epoch = u32::MAX;
        let live = sets.intern(&[(3, 4)]);
        sets.trace_set(live, &mut |_| {});
        assert_eq!(sets.sweep(), 0);
        assert_eq!(sets.epoch(), 1);
        assert_eq!(sets.entry(live).mark.get(), 0);
        assert_eq!(sets.entry(live).born, 0);
        assert_eq!(sets.sweep(), 1);
        assert_eq!(sets.intern(&[]), SetId::EMPTY);
    }
}
