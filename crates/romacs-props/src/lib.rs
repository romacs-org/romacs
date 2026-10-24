//! Heap-global ordered plists and 0-based character property runs, unwired to Lisp.
//! Bounds require `from <= to <= len`. Equal IDs merge eagerly except where
//! combined lengths exceed u32::MAX. Equal sets with different IDs retain plist
//! order/history but property-change queries skip their boundaries. Hidden GNU
//! interval boundaries are not retained; see `perf/VS-GNU.md` for known gaps.
mod sets;
mod sticky;
mod tree;
use romacs_chars::CharPos;
pub use sets::PropSets;
use std::{cell::Cell, hash::Hash};
pub use sticky::{Sticky, StickyEnv, StickyKey};
use tree::Tree;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SetId(u32);
impl SetId {
    pub const EMPTY: Self = Self(0);
}
pub enum Op<'a, V> {
    Add(&'a [(V, V)]),
    Set(SetId),
    Remove(&'a [V]),
}
#[derive(Clone, Copy, Debug)]
struct Finger {
    leaf: u32,
    idx: usize,
    start: usize,
    leaf_end: usize,
    version: u64,
}
#[derive(Clone, Debug)]
pub struct Props {
    tree: Tree,
    len: usize,
    version: u64,
    flags_epoch: Option<u32>,
    finger: Cell<Option<Finger>>,
    #[cfg(test)]
    work: Cell<usize>,
}
impl Default for Props {
    fn default() -> Self {
        Self::new()
    }
}
impl Props {
    pub fn new() -> Self {
        Self {
            tree: Tree::default(),
            len: 0,
            version: 0,
            flags_epoch: None,
            finger: Cell::new(None),
            #[cfg(test)]
            work: Cell::new(0),
        }
    }
    pub fn filled(len: usize, s: SetId) -> Self {
        let mut p = Self::new();
        p.insert(CharPos(0), len, s);
        p
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn is_plain(&self) -> bool {
        self.runs(CharPos(0), CharPos(self.len))
            .all(|(_, _, s)| s == SetId::EMPTY)
    }
    /// Allocated tree storage, including spare arena capacity and free lists.
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.tree.bytes()
    }
    #[cfg(test)]
    fn reset_work(&self) {
        self.work.set(0)
    }
    #[cfg(test)]
    fn measured_work(&self) -> usize {
        self.work.get()
    }
    fn count(&self, n: usize) {
        #[cfg(test)]
        self.work.set(self.work.get() + n);
        #[cfg(not(test))]
        let _ = n;
    }
    fn invalidate(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.finger.set(None);
    }
    #[inline]
    fn find(&self, p: usize) -> (u32, usize, usize) {
        assert!(p < self.len);
        if let Some(f) = self.finger.get() {
            let l = &self.tree.leaves[f.leaf as usize];
            if p >= f.start && p < f.start + l.lens[f.idx] as usize {
                return (f.leaf, f.idx, f.start);
            }
        }
        self.find_miss(p)
    }
    #[cold]
    fn find_miss(&self, p: usize) -> (u32, usize, usize) {
        if let Some(f) = self
            .finger
            .get()
            .filter(|f| f.version == self.version && p >= f.start && p <= f.leaf_end)
        {
            let l = &self.tree.leaves[f.leaf as usize];
            let end = f.start + l.lens[f.idx] as usize;
            if p >= f.start && p < end {
                return (f.leaf, f.idx, f.start);
            }
            if p >= end {
                let (mut id, mut i, mut a) = (f.leaf, f.idx + 1, end);
                if i == l.n {
                    id = l.next;
                    i = 0;
                }
                if !Tree::is_nil(id) {
                    let l = &self.tree.leaves[id as usize];
                    let leaf_end = if id == f.leaf {
                        f.leaf_end
                    } else {
                        a + l.chars
                    };
                    if p < a + l.lens[i] as usize {
                        self.finger.set(Some(Finger {
                            leaf: id,
                            idx: i,
                            start: a,
                            leaf_end,
                            version: self.version,
                        }));
                        self.count(1);
                        return (id, i, a);
                    }
                    if p < leaf_end {
                        while p >= a + l.lens[i] as usize {
                            self.count(1);
                            a += l.lens[i] as usize;
                            i += 1;
                        }
                        self.finger.set(Some(Finger {
                            leaf: id,
                            idx: i,
                            start: a,
                            leaf_end,
                            version: self.version,
                        }));
                        self.count(1);
                        return (id, i, a);
                    }
                }
            }
        }
        self.find_cold(p)
    }
    #[cold]
    fn find_cold(&self, p: usize) -> (u32, usize, usize) {
        let (id, i, a, depth, leaf_end) = self.tree.locate(p);
        self.count(depth + 1);
        self.finger.set(Some(Finger {
            leaf: id,
            idx: i,
            start: a,
            leaf_end,
            version: self.version,
        }));
        (id, i, a)
    }
    #[inline]
    pub fn run_at(&self, p: CharPos) -> (CharPos, CharPos, SetId) {
        let (id, i, a) = self.find(p.0);
        let l = &self.tree.leaves[id as usize];
        (CharPos(a), CharPos(a + l.lens[i] as usize), l.sets[i])
    }
    pub fn runs(
        &self,
        from: CharPos,
        to: CharPos,
    ) -> impl Iterator<Item = (CharPos, CharPos, SetId)> + '_ {
        assert!(from <= to && to.0 <= self.len);
        let (mut id, mut i, mut pos) = if from < to {
            self.find(from.0)
        } else {
            (u32::MAX, 0, 0)
        };
        std::iter::from_fn(move || {
            if Tree::is_nil(id) || pos >= to.0 {
                return None;
            }
            let l = &self.tree.leaves[id as usize];
            let end = pos + l.lens[i] as usize;
            let out = (CharPos(pos.max(from.0)), CharPos(end.min(to.0)), l.sets[i]);
            pos = end;
            i += 1;
            if i == l.n {
                id = l.next;
                i = 0;
            }
            Some(out)
        })
    }
    fn boundary(&mut self, p: usize) -> (u32, usize) {
        if p == self.len {
            let id = self.tree.last;
            return (
                id,
                if Tree::is_nil(id) {
                    0
                } else {
                    self.tree.leaves[id as usize].n
                },
            );
        }
        let (id, i, a) = self.find(p);
        if a == p {
            return (id, i);
        }
        let l = &self.tree.leaves[id as usize];
        let (len, s) = (l.lens[i] as usize, l.sets[i]);
        self.tree.set_len(id, i, (p - a) as u32);
        self.tree.insert(id, i + 1, (a + len - p) as u32, s);
        self.finger.set(None);
        let (id, i, _) = self.find(p);
        (id, i)
    }
    fn append(&mut self, mut n: usize, s: SetId) {
        if Tree::is_nil(self.tree.last) {
            self.tree.initial();
        }
        while n > 0 {
            let add = n.min(u32::MAX as usize);
            assert!(self.tree.append(add as u32, s));
            self.len += add;
            n -= add;
        }
    }
    #[inline]
    pub fn insert(&mut self, at: CharPos, len: usize, fill: SetId) {
        assert!(at.0 <= self.len);
        if len == 0 {
            return;
        }
        self.len.checked_add(len).expect("property length overflow");
        self.invalidate();
        if at.0 == self.len {
            self.append(len, fill);
            return;
        }
        let (id, i, _) = self.find(at.0);
        let l = &self.tree.leaves[id as usize];
        if l.sets[i] == fill && len <= u32::MAX as usize - l.lens[i] as usize {
            self.tree.set_len(id, i, l.lens[i] + len as u32);
            self.len += len;
            self.invalidate();
            return;
        }
        self.boundary(at.0);
        let mut pos = at.0;
        let mut n = len;
        while n > 0 {
            let (id, i, _) = self.find(pos);
            let add = n.min(u32::MAX as usize);
            self.tree.insert(id, i, add as u32, fill);
            self.len += add;
            n -= add;
            pos += add;
            self.invalidate();
        }
        self.coalesce_at(at.0);
        self.coalesce_at(at.0 + len);
        self.invalidate();
    }
    fn coalesce_at(&mut self, mut p: usize) {
        while p > 0 && p < self.len {
            let (lid, li, _) = self.find(p - 1);
            let (rid, ri, a) = self.find(p);
            if a != p {
                break;
            }
            let l = &self.tree.leaves[lid as usize];
            let r = &self.tree.leaves[rid as usize];
            if l.sets[li] != r.sets[ri] {
                break;
            }
            let next = p + r.lens[ri] as usize;
            let n = l.lens[li] as u64 + r.lens[ri] as u64;
            if n <= u32::MAX as u64 {
                self.tree.set_len(lid, li, n as u32);
                self.tree.remove(rid, ri);
            } else {
                self.tree.set_len(lid, li, u32::MAX);
                self.tree.set_len(rid, ri, (n - u32::MAX as u64) as u32);
            }
            self.finger.set(None);
            p = next;
        }
    }
    pub fn delete(&mut self, from: CharPos, to: CharPos) {
        assert!(from <= to && to.0 <= self.len);
        if from == to {
            return;
        }
        self.invalidate();
        self.boundary(to.0);
        self.boundary(from.0);
        let mut n = to - from;
        while n > 0 {
            let (id, i, _) = self.find(from.0);
            let len = self.tree.leaves[id as usize].lens[i] as usize;
            self.tree.remove(id, i);
            self.len -= len;
            n -= len;
            self.invalidate();
            self.count(1);
        }
        self.coalesce_at(from.0);
        self.invalidate();
    }
    pub fn insert_props(&mut self, at: CharPos, src: &Props) {
        assert!(at.0 <= self.len);
        self.len
            .checked_add(src.len)
            .expect("property length overflow");
        let mut pos = at;
        for (a, b, s) in src.runs(CharPos(0), CharPos(src.len)) {
            self.insert(pos, b - a, s);
            pos = pos + (b - a);
        }
    }
    pub fn slice(&self, from: CharPos, to: CharPos) -> Self {
        let mut out = Self::new();
        for (a, b, s) in self.runs(from, to) {
            out.insert(CharPos(out.len), b - a, s);
        }
        out
    }
    pub fn would_change<V: Copy + Eq + Hash>(
        &self,
        sets: &PropSets<V>,
        from: CharPos,
        to: CharPos,
        op: &Op<'_, V>,
    ) -> bool {
        self.runs(from, to).any(|(_, _, s)| sets.changed(s, op))
    }
    pub fn apply<V: Copy + Eq + Hash>(
        &mut self,
        sets: &mut PropSets<V>,
        from: CharPos,
        to: CharPos,
        op: Op<'_, V>,
    ) -> bool {
        assert!(from <= to && to.0 <= self.len);
        if from == to {
            return false;
        }
        let (id, i, a) = self.find(from.0);
        let l = &self.tree.leaves[id as usize];
        let old = l.sets[i];
        let old_chars = l.chars;
        let leaf_end = self.finger.get().unwrap().leaf_end;
        let b = a + l.lens[i] as usize;
        if to.0 <= b {
            if !sets.changed(old, &op) {
                return false;
            }
            let set = sets.transform(old, &op);
            if set == old {
                return true;
            }
            let (leaf, idx) = self.tree.paint(
                id,
                i,
                (from.0 - a) as u32,
                (to - from) as u32,
                (b - to.0) as u32,
                set,
            );
            self.invalidate();
            self.count(1);
            self.finger.set(Some(Finger {
                leaf,
                idx,
                start: from.0,
                leaf_end: if leaf == id {
                    leaf_end - old_chars + self.tree.leaves[id as usize].chars
                } else {
                    leaf_end
                },
                version: self.version,
            }));
            if from.0 == a {
                self.coalesce_at(from.0);
            }
            if to.0 == b {
                self.coalesce_at(to.0);
            }
            return true;
        }
        if !self.would_change(sets, from, to, &op) {
            return false;
        }
        self.invalidate();
        self.boundary(to.0);
        self.boundary(from.0);
        let (mut id, mut i, _) = self.find(from.0);
        let mut pos = from.0;
        let mut boundaries = Vec::new();
        while pos < to.0 {
            let l = &mut self.tree.leaves[id as usize];
            let next = l.next;
            let mut visits = 0;
            while i < l.n && pos < to.0 {
                pos += l.lens[i] as usize;
                l.sets[i] = sets.transform(l.sets[i], &op);
                i += 1;
                visits += 1;
            }
            self.count(visits);
            self.tree.normalize(id);
            boundaries.push(pos);
            id = next;
            i = 0;
        }
        self.invalidate();
        for end in boundaries {
            self.coalesce_at(end);
        }
        self.coalesce_at(from.0);
        self.coalesce_at(to.0);
        self.invalidate();
        true
    }
    pub fn merge_under<V: Copy + Eq + Hash>(
        &mut self,
        sets: &mut PropSets<V>,
        at: CharPos,
        src: &Props,
    ) {
        let mut out = Self::new();
        for (a, b, s) in src.runs(CharPos(0), CharPos(src.len)) {
            for (x, y, t) in self.runs(at + a.0, at + b.0) {
                let id = sets.union_under(t, s);
                out.insert(CharPos(out.len), y - x, id);
            }
        }
        self.delete(at, at + src.len);
        self.insert_props(at, &out);
    }
    fn change(
        &self,
        p: CharPos,
        limit: CharPos,
        forward: bool,
        equal: impl Fn(SetId, SetId) -> bool,
    ) -> Option<CharPos> {
        if forward {
            if p.0 >= self.len {
                return None;
            }
            let (id, i, a) = self.find(p.0);
            let l = &self.tree.leaves[id as usize];
            let base = l.sets[i];
            let (mut id, mut i, mut pos) = (id, i, a);
            let mut leaf_end = self.finger.get().unwrap().leaf_end;
            loop {
                let l = &self.tree.leaves[id as usize];
                pos += l.lens[i] as usize;
                i += 1;
                if pos >= limit.0 || pos >= self.len {
                    return None;
                }
                if i == l.n {
                    id = l.next;
                    i = 0;
                    leaf_end = pos + self.tree.leaves[id as usize].chars;
                }
                self.count(1);
                let s = self.tree.leaves[id as usize].sets[i];
                if !equal(base, s) {
                    self.finger.set(Some(Finger {
                        leaf: id,
                        idx: i,
                        start: pos,
                        leaf_end,
                        version: self.version,
                    }));
                    return Some(CharPos(pos));
                }
            }
        } else {
            if p.0 == 0 {
                return None;
            }
            let mut q = p.0.min(self.len);
            let base = self.run_at(CharPos(q - 1)).2;
            while q > limit.0 {
                let (a, _, s) = self.run_at(CharPos(q - 1));
                if !equal(base, s) {
                    return Some(CharPos(q));
                }
                q = a.0;
            }
            None
        }
    }
    pub fn next_change<V: Copy + Eq + Hash>(
        &self,
        s: &PropSets<V>,
        p: CharPos,
        l: CharPos,
    ) -> Option<CharPos> {
        self.change(p, l, true, |a, b| s.set_eq(a, b))
    }
    pub fn prev_change<V: Copy + Eq + Hash>(
        &self,
        s: &PropSets<V>,
        p: CharPos,
        l: CharPos,
    ) -> Option<CharPos> {
        self.change(p, l, false, |a, b| s.set_eq(a, b))
    }
    /// Finds an effective-value boundary, including owner-resolved category fallback.
    /// Absence must resolve to Lisp nil; no boundary maps to the supplied limit.
    #[inline]
    pub fn next_single<V: Eq>(
        &self,
        p: CharPos,
        l: CharPos,
        value: impl Fn(SetId) -> V,
    ) -> Option<CharPos> {
        self.change(p, l, true, |a, b| value(a) == value(b))
    }
    pub fn prev_single<V: Eq>(
        &self,
        p: CharPos,
        l: CharPos,
        value: impl Fn(SetId) -> V,
    ) -> Option<CharPos> {
        self.change(p, l, false, |a, b| value(a) == value(b))
    }
    /// Finds a flagged run, clipped to the requested range. The first call after
    /// collection rebuilds summaries; ordinary edits refresh only dirty leaves.
    /// As with all run APIs, IDs must belong to the supplied heap-global interner.
    pub fn next_flagged<V: Copy + Eq + Hash>(
        &mut self,
        sets: &PropSets<V>,
        from: CharPos,
        to: CharPos,
        mask: u8,
    ) -> Option<(CharPos, CharPos, SetId)> {
        assert!(from <= to && to.0 <= self.len);
        if from == to {
            return None;
        }
        if self.flags_epoch != Some(sets.epoch()) {
            self.tree.refresh_flags(|s| sets.flags(s));
            self.flags_epoch = Some(sets.epoch());
        } else {
            self.tree.refresh_dirty_flags(|s| sets.flags(s));
        }
        self.tree
            .next_flagged(from.0, to.0, mask, |s| sets.flags(s))
            .map(|(a, b, s)| (CharPos(a), CharPos(b), s))
    }
    pub fn trace<V: Copy + Eq + Hash>(&self, sets: &PropSets<V>, mark: &mut impl FnMut(V)) {
        for (_, _, s) in self.runs(CharPos(0), CharPos(self.len)) {
            sets.trace_set(s, mark);
        }
    }
    #[doc(hidden)]
    pub fn check_invariants<V: Copy + Eq + Hash>(&self, sets: &PropSets<V>) {
        self.tree.check();
        let (mut total, mut last) = (0, None);
        for (a, b, s) in self.runs(CharPos(0), CharPos(self.len)) {
            assert_eq!(a.0, total);
            assert!(b > a);
            if let Some((old, len)) = last {
                assert!(old != s || len + (b - a) > u32::MAX as usize);
            }
            sets.pairs(s);
            last = Some((s, b - a));
            total = b.0;
        }
        assert_eq!(total, self.len);
    }
}
#[cfg(test)]
extern crate self as romacs_props;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod p1_tests;
#[cfg(test)]
mod scaling_tests;
