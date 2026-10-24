//! Standalone character-keyed markers. This does not change `Text`'s marker owner.
//!
//! Slots are exclusively owned handles, not generational IDs: after `free`, a
//! caller must discard the handle before another allocation can reuse it.
use std::{cell::Cell, ops::Range};

const CAP: usize = 64;
const DEAD: u32 = u32::MAX;

/// An exclusively owned slot in one `MarkerTree`. Never use it after freeing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MarkerSlot(u32);

/// Proposed P5 heap object layout, not the editor's current marker object.
#[repr(C)]
pub struct ProposedMarkerObject {
    pub header: u64,
    pub buffer: u32,
    pub slot: u32,
}
const _: () = assert!(std::mem::size_of::<ProposedMarkerObject>() == 16);

#[derive(Clone)]
struct Leaf {
    base: u64,
    rel: [u32; CAP],
    slot: [u32; CAP],
    after: u64,
    n: usize,
}
impl Default for Leaf {
    fn default() -> Self {
        Self {
            base: 0,
            rel: [0; CAP],
            slot: [0; CAP],
            after: 0,
            n: 0,
        }
    }
}
impl Leaf {
    fn key(&self, i: usize) -> (u64, bool) {
        (self.base + u64::from(self.rel[i]), self.after >> i & 1 != 0)
    }
    fn entries(&self) -> Vec<(u64, bool, u32)> {
        (0..self.n)
            .map(|i| {
                let (p, a) = self.key(i);
                (p, a, self.slot[i])
            })
            .collect()
    }
}

/// Sorted 64-entry relative leaves, with stable leaf IDs and O(1) slot lookup.
///
/// Edits shift suffix leaf bases, not suffix markers. Structural changes move
/// leaf IDs in `order`, leaving slots outside the changed leaves untouched.
/// Coordinates are zero-based characters; callers own buffer bounds validation.
#[derive(Clone, Default)]
pub struct MarkerTree {
    leaves: Vec<Leaf>,
    order: Vec<u32>,
    free_leaves: Vec<u32>,
    slots: Vec<u32>,
    free_slots: Vec<u32>,
    len: usize,
    boundary: Cell<Option<usize>>,
    #[cfg(test)]
    work: usize,
}
impl MarkerTree {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Allocated tree storage, excluding the separately owned 16-byte objects.
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.leaves.capacity() * std::mem::size_of::<Leaf>()
            + (self.order.capacity()
                + self.free_leaves.capacity()
                + self.slots.capacity()
                + self.free_slots.capacity())
                * 4
    }
    fn location(&self, slot: MarkerSlot) -> Option<(usize, usize)> {
        let &packed = self.slots.get(slot.0 as usize)?;
        (packed != DEAD).then_some(((packed >> 6) as usize, (packed & 63) as usize))
    }
    pub fn position(&self, slot: MarkerSlot) -> Option<u64> {
        let (l, i) = self.location(slot)?;
        Some(self.leaves[l].key(i).0)
    }
    pub fn insertion_type(&self, slot: MarkerSlot) -> Option<bool> {
        let (l, i) = self.location(slot)?;
        Some(self.leaves[l].key(i).1)
    }
    fn leaf(&self, rank: usize) -> &Leaf {
        &self.leaves[self.order[rank] as usize]
    }
    fn lower_leaf(&self, key: (u64, bool)) -> usize {
        // A rank is only a hint: splits, merges, moves and edits may all change
        // its meaning. Validate against today's two bounding last keys, including
        // strict predecessor comparison for ties spanning multiple leaves.
        if let Some(rank) = self.boundary.get()
            && rank <= self.order.len()
            && (rank == 0 || self.leaf(rank - 1).key(self.leaf(rank - 1).n - 1) < key)
            && (rank == self.order.len() || self.leaf(rank).key(self.leaf(rank).n - 1) >= key)
        {
            return rank;
        }
        let rank = self.order.partition_point(|&l| {
            let l = &self.leaves[l as usize];
            l.key(l.n - 1) < key
        });
        self.boundary.set(Some(rank));
        rank
    }
    fn alloc_leaf(&mut self) -> u32 {
        self.free_leaves.pop().unwrap_or_else(|| {
            let id = u32::try_from(self.leaves.len()).expect("marker leaf capacity exhausted");
            assert!(id < (DEAD >> 6));
            self.leaves.push(Leaf::default());
            id
        })
    }
    fn write_leaf(&mut self, id: u32, entries: &[(u64, bool, u32)]) {
        let l = &mut self.leaves[id as usize];
        l.n = entries.len();
        l.after = 0;
        if entries.is_empty() {
            return;
        }
        l.base = entries[0].0;
        for (i, &(p, after, slot)) in entries.iter().enumerate() {
            l.rel[i] = u32::try_from(p - l.base).expect("marker relative span");
            l.slot[i] = slot;
            l.after |= (after as u64) << i;
            self.slots[slot as usize] = (id << 6) | i as u32;
        }
    }
    // Repack only the affected leaves. A wide coordinate span also splits a leaf.
    fn repack(&mut self, ranks: Range<usize>, entries: &[(u64, bool, u32)]) {
        let old: Vec<_> = self.order[ranks.clone()].to_vec();
        let mut ids = Vec::new();
        let mut start = 0;
        while start < entries.len() {
            let mut end = (start + CAP).min(entries.len());
            while entries[end - 1].0 - entries[start].0 > u64::from(u32::MAX) {
                end -= 1;
            }
            // Leave room for subsequent set-marker churn after a capacity split.
            if end - start == CAP && end < entries.len() {
                end = start + CAP / 2;
            }
            let id = old
                .get(ids.len())
                .copied()
                .unwrap_or_else(|| self.alloc_leaf());
            self.write_leaf(id, &entries[start..end]);
            ids.push(id);
            start = end;
        }
        for &id in &old[ids.len().min(old.len())..] {
            self.leaves[id as usize].n = 0;
            self.free_leaves.push(id);
        }
        self.order.splice(ranks, ids);
    }
    fn add_slot(&mut self, slot: u32, pos: u64, after: bool) {
        // Sequential allocation keeps completed leaves full instead of splitting
        // each into two half-full leaves. Wide spans still require a new base.
        if let Some(&id) = self.order.last() {
            let leaf = &mut self.leaves[id as usize];
            if leaf.key(leaf.n - 1) <= (pos, after) {
                if leaf.n < CAP && pos - leaf.base <= u64::from(u32::MAX) {
                    let i = leaf.n;
                    leaf.rel[i] = (pos - leaf.base) as u32;
                    leaf.slot[i] = slot;
                    leaf.after = (leaf.after & !(1 << i)) | ((after as u64) << i);
                    leaf.n += 1;
                    self.slots[slot as usize] = (id << 6) | i as u32;
                } else {
                    let id = self.alloc_leaf();
                    self.write_leaf(id, &[(pos, after, slot)]);
                    self.order.push(id);
                }
                return;
            }
        }
        let rank = self
            .lower_leaf((pos, after))
            .min(self.order.len().saturating_sub(1));
        if self.order.is_empty() {
            self.repack(0..0, &[(pos, after, slot)]);
        } else {
            let mut entries = self.leaf(rank).entries();
            let at = entries.partition_point(|&(p, a, _)| (p, a) <= (pos, after));
            entries.insert(at, (pos, after, slot));
            self.repack(rank..rank + 1, &entries);
        }
    }
    pub fn new_marker(&mut self, pos: u64, after: bool) -> MarkerSlot {
        let slot = self.free_slots.pop().unwrap_or_else(|| {
            let slot = u32::try_from(self.slots.len()).expect("marker slots exhausted");
            assert_ne!(slot, DEAD);
            self.slots.push(DEAD);
            slot
        });
        self.add_slot(slot, pos, after);
        self.len += 1;
        MarkerSlot(slot)
    }
    fn remove_slot(&mut self, slot: MarkerSlot) -> bool {
        let Some((id, i)) = self.location(slot) else {
            return false;
        };
        let pos = self.leaves[id].key(i).0;
        let mut rank = self.lower_leaf((pos, false));
        while self.order[rank] as usize != id {
            rank += 1;
        }
        let mut entries = self.leaves[id].entries();
        entries.remove(i);
        let mut end = rank + 1;
        if entries.len() < 16 && end < self.order.len() {
            let next = self.leaf(end);
            if entries.len() + next.n <= CAP
                && (entries.is_empty()
                    || next.key(next.n - 1).0 - entries[0].0 <= u64::from(u32::MAX))
            {
                entries.extend(next.entries());
                end += 1;
            }
        }
        if entries.len() < 16 && rank > 0 {
            let previous = self.leaf(rank - 1);
            if previous.n + entries.len() <= CAP
                && entries
                    .last()
                    .is_none_or(|e| e.0 - previous.base <= u64::from(u32::MAX))
            {
                let mut joined = previous.entries();
                joined.extend(entries);
                entries = joined;
                rank -= 1;
            }
        }
        self.repack(rank..end, &entries);
        self.slots[slot.0 as usize] = DEAD;
        true
    }
    pub fn free(&mut self, slot: MarkerSlot) -> bool {
        if !self.remove_slot(slot) {
            return false;
        }
        self.free_slots.push(slot.0);
        self.len -= 1;
        true
    }
    pub fn set(&mut self, slot: MarkerSlot, pos: u64) -> bool {
        let Some(after) = self.insertion_type(slot) else {
            return false;
        };
        self.remove_slot(slot);
        self.add_slot(slot.0, pos, after);
        true
    }
    pub fn set_insertion_type(&mut self, slot: MarkerSlot, after: bool) -> bool {
        let Some(pos) = self.position(slot) else {
            return false;
        };
        if self.insertion_type(slot) == Some(after) {
            return true;
        }
        self.remove_slot(slot);
        self.add_slot(slot.0, pos, after);
        true
    }
    /// Shift positions after `at`, including equal positions of after-type markers.
    /// `before_markers` advances every marker at the insertion boundary.
    pub fn insert(&mut self, at: u64, len: u64, before_markers: bool) {
        if len == 0 {
            return;
        }
        let key = (at, !before_markers);
        let rank = self.lower_leaf(key);
        if rank == self.order.len() {
            return;
        }
        assert!(
            self.leaf(self.order.len() - 1)
                .key(self.leaf(self.order.len() - 1).n - 1)
                .0
                .checked_add(len)
                .is_some(),
            "marker position overflow"
        );
        #[cfg(test)]
        {
            self.work = 0;
        }
        if self.leaf(rank).key(0) >= key {
            self.shift_suffix(rank, len, 0);
            return;
        }
        let id = self.order[rank] as usize;
        let leaf = &mut self.leaves[id];
        #[cfg(test)]
        {
            self.work += leaf.n;
        }
        if u64::from(leaf.rel[leaf.n - 1])
            .checked_add(len)
            .is_some_and(|r| r <= u64::from(u32::MAX))
        {
            let mut split = leaf.rel[..leaf.n].partition_point(|&r| leaf.base + u64::from(r) < at);
            while split < leaf.n && leaf.key(split) < key {
                split += 1;
            }
            for rel in &mut leaf.rel[split..leaf.n] {
                *rel += len as u32;
            }
            self.shift_suffix(rank + 1, len, 0);
        } else {
            let mut entries = leaf.entries();
            for (p, a, _) in &mut entries {
                if (*p, *a) >= key {
                    *p += len;
                }
            }
            self.shift_suffix(rank + 1, len, 0);
            self.repack(rank..rank + 1, &entries);
        }
    }
    fn shift_suffix(&mut self, rank: usize, add: u64, sub: u64) {
        for &id in &self.order[rank..] {
            self.leaves[id as usize].base = self.leaves[id as usize].base - sub + add;
            #[cfg(test)]
            {
                self.work += 1;
            }
        }
    }
    /// Collapse the closed boundary group `[from, to]` to `from`.
    pub fn delete(&mut self, from: u64, to: u64) {
        self.replace(from, to, 0);
    }
    /// Markers strictly inside the replaced range collapse to `from`, regardless
    /// of insertion type; markers at `to` follow the replacement's new end.
    /// For an empty old range this is a normal insertion.
    pub fn replace(&mut self, from: u64, to: u64, len: u64) {
        assert!(from <= to);
        if from == to {
            self.insert(from, len, false);
            return;
        }
        assert!(from.checked_add(len).is_some(), "marker position overflow");
        #[cfg(test)]
        {
            self.work = 0;
        }
        let start = self.lower_leaf((from, false));
        let end = self
            .order
            .partition_point(|&id| self.leaves[id as usize].key(0).0 <= to);
        if !self.order.is_empty() {
            let last = self.leaf(self.order.len() - 1);
            if last.key(last.n - 1).0 >= to {
                assert!(
                    last.key(last.n - 1)
                        .0
                        .checked_sub(to - from)
                        .and_then(|p| p.checked_add(len))
                        .is_some(),
                    "marker position overflow"
                );
            }
        }
        let mut entries = Vec::new();
        for rank in start..end {
            entries.extend(self.leaf(rank).entries());
        }
        #[cfg(test)]
        {
            self.work += entries.len();
        }
        for (p, _, _) in &mut entries {
            if *p >= to {
                *p = *p - (to - from) + len;
            } else if *p > from {
                *p = from;
            }
        }
        // Collapse joins previously separated before/after groups.
        entries.sort_by_key(|&(p, a, _)| (p, a));
        self.shift_suffix(end, len, to - from);
        self.repack(start..end, &entries);
    }
    /// Sorted `(slot, position, insertion_type)` entries in the half-open range.
    pub fn range(&self, range: Range<u64>) -> impl Iterator<Item = (MarkerSlot, u64, bool)> + '_ {
        let start = self.lower_leaf((range.start, false));
        self.order[start..]
            .iter()
            .flat_map(|&id| {
                let l = &self.leaves[id as usize];
                (0..l.n).map(move |i| {
                    let (p, a) = l.key(i);
                    (MarkerSlot(l.slot[i]), p, a)
                })
            })
            .skip_while(move |&(_, p, _)| p < range.start)
            .take_while(move |&(_, p, _)| p < range.end)
    }
}

#[cfg(test)]
mod tests;
