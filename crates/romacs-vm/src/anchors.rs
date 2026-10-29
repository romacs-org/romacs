//! Registered, byte-position anchors with GNU marker insertion semantics.
//!
//! Lazy offsets affect descendants, not the node itself. Handles carry a slot
//! generation; edits and relocation never allocate. No text/history is retained.
//! All positions are zero-based, bounded by the explicit extent (at most u32::MAX).

/// Insertion at an anchor stays before it only for Right bias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bias {
    Left,
    Right,
}
romacs_snapshot::snapshot_enum!(Bias {
    Bias::Left => (),
    Bias::Right => (),
});
/// Stable identity; stale handles cannot alias a reused slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Anchor {
    slot: u32,
    generation: u32,
}
romacs_snapshot::snapshot_struct!(Anchor { slot, generation });
const NIL: u32 = u32::MAX;
#[derive(Clone, Copy)]
struct Node {
    l: u32,
    r: u32,
    p: u32,
    pos: u32,
    lazy: u32, // pending wrapping add for descendants (not self)
    flags: u32,
}
romacs_snapshot::snapshot_struct!(Node { l, r, p, pos, lazy, flags });
#[derive(Clone)]
pub struct AnchorTree {
    n: Vec<Node>,
    root: u32,
    free: u32,
    len: u32,
    extent: u32,
    /// Each slot's creation sequence number, beside the nodes so that
    /// relocation never touches it: GNU's marker chain order.
    births: Vec<u64>,
    born: u64,
}
romacs_snapshot::snapshot_struct!(AnchorTree { n, root, free, len, extent, births, born });
#[inline]
fn prio(i: u32) -> u32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^ (x >> 15)
}
impl AnchorTree {
    #[inline]
    fn key_lt(&self, t: u32, k: (u32, bool)) -> bool {
        let n = &self.n[t as usize];
        (n.pos, n.flags & 1 != 0) < k
    }
    #[inline]
    fn apply(&mut self, t: u32, d: u32) {
        if t != NIL {
            let n = &mut self.n[t as usize];
            n.pos = n.pos.wrapping_add(d);
            n.lazy = n.lazy.wrapping_add(d);
        }
    }
    #[inline]
    fn push(&mut self, t: u32) {
        let z = self.n[t as usize].lazy;
        if z != 0 {
            let (l, r) = (self.n[t as usize].l, self.n[t as usize].r);
            self.apply(l, z);
            self.apply(r, z);
            self.n[t as usize].lazy = 0;
        }
    }
    fn split(&mut self, t: u32, k: (u32, bool)) -> (u32, u32) {
        if t == NIL {
            return (NIL, NIL);
        }
        self.push(t);
        if self.key_lt(t, k) {
            let (a, b) = self.split(self.n[t as usize].r, k);
            self.n[t as usize].r = a;
            if a != NIL {
                self.n[a as usize].p = t;
            }
            (t, b)
        } else {
            let (a, b) = self.split(self.n[t as usize].l, k);
            self.n[t as usize].l = b;
            if b != NIL {
                self.n[b as usize].p = t;
            }
            (a, t)
        }
    }
    fn merge(&mut self, a: u32, b: u32) -> u32 {
        if a == NIL {
            return b;
        }
        if b == NIL {
            return a;
        }
        if prio(a) > prio(b) {
            self.push(a);
            let m = self.merge(self.n[a as usize].r, b);
            self.n[a as usize].r = m;
            self.n[m as usize].p = a;
            a
        } else {
            self.push(b);
            let m = self.merge(a, self.n[b as usize].l);
            self.n[b as usize].l = m;
            self.n[m as usize].p = b;
            b
        }
    }
    fn link_root(&mut self, t: u32) {
        self.root = t;
        if t != NIL {
            self.n[t as usize].p = NIL;
        }
    }
    fn place(&mut self, i: u32) {
        let k = (self.n[i as usize].pos, (self.n[i as usize].flags & 1 != 0));
        let (a, b) = self.split(self.root, k);
        let m = self.merge(a, i);
        let r = self.merge(m, b);
        self.link_root(r);
    }
    fn detach(&mut self, i: u32) {
        self.push_ancestors(i);
        self.push(i);
        let (l, r, p) = (
            self.n[i as usize].l,
            self.n[i as usize].r,
            self.n[i as usize].p,
        );
        if l != NIL {
            self.n[l as usize].p = NIL;
        }
        if r != NIL {
            self.n[r as usize].p = NIL;
        }
        let sub = self.merge(l, r);
        if p == NIL {
            self.link_root(sub);
        } else {
            if self.n[p as usize].l == i {
                self.n[p as usize].l = sub;
            } else {
                self.n[p as usize].r = sub;
            }
            if sub != NIL {
                self.n[sub as usize].p = p;
            }
        }
        let n = &mut self.n[i as usize];
        n.l = NIL;
        n.r = NIL;
        n.p = NIL;
        n.lazy = 0;
    }
    fn push_ancestors(&mut self, i: u32) {
        let p = self.n[i as usize].p;
        if p != NIL {
            self.push_ancestors(p);
            self.push(p);
        }
    }
    // Partition the collapsed subtree by bias, retaining treap priorities.
    // No per-edit vector, heap allocation, or fixed-depth path buffer.
    fn collapse(&mut self, t: u32, pos: u32) -> (u32, u32) {
        if t == NIL {
            return (NIL, NIL);
        }
        let node = self.n[t as usize];
        let (ll, lr) = self.collapse(node.l, pos);
        let (rl, rr) = self.collapse(node.r, pos);
        let n = &mut self.n[t as usize];
        n.l = NIL;
        n.r = NIL;
        n.p = NIL;
        n.lazy = 0;
        n.pos = pos;
        if node.flags & 1 == 0 {
            let left = self.merge(ll, t);
            (self.merge(left, rl), self.merge(lr, rr))
        } else {
            let right = self.merge(lr, t);
            (self.merge(ll, rl), self.merge(right, rr))
        }
    }
}

impl AnchorTree {
    pub fn new(extent: u32) -> Self {
        Self {
            n: Vec::new(),
            root: NIL,
            free: NIL,
            len: 0,
            extent,
            births: Vec::new(),
            born: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn extent(&self) -> u32 {
        self.extent
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        (if capacity {
            self.n.capacity()
        } else {
            self.n.len()
        }) * std::mem::size_of::<Node>()
            + (if capacity { self.births.capacity() } else { self.births.len() }) * std::mem::size_of::<u64>()
    }
    pub const fn bytes_per_slot() -> usize {
        std::mem::size_of::<Node>()
    }
    fn valid(&self, h: Anchor) -> bool {
        self.n
            .get(h.slot as usize)
            .is_some_and(|n| n.flags & 2 != 0 && n.flags >> 2 == h.generation)
    }
    #[inline]
    pub fn resolve(&self, h: Anchor) -> Option<u32> {
        if !self.valid(h) {
            return None;
        }
        let mut pos = self.n[h.slot as usize].pos;
        let mut x = self.n[h.slot as usize].p;
        while x != NIL {
            pos = pos.wrapping_add(self.n[x as usize].lazy);
            x = self.n[x as usize].p;
        }
        Some(pos)
    }
    /// Find a matching anchor at a boundary without allocating a result list.
    pub fn find_at(
        &self,
        pos: u32,
        bias: Bias,
        predicate: impl Fn(Anchor, &Self) -> bool,
    ) -> Option<Anchor> {
        fn find(
            t: &AnchorTree,
            i: u32,
            offset: u32,
            key: (u32, bool),
            f: &impl Fn(Anchor, &AnchorTree) -> bool,
        ) -> Option<Anchor> {
            if i == NIL {
                return None;
            }
            let n = t.n[i as usize];
            let current = (n.pos.wrapping_add(offset), n.flags & 1 != 0);
            let child_offset = offset.wrapping_add(n.lazy);
            if current >= key
                && let Some(h) = find(t, n.l, child_offset, key, f)
            {
                return Some(h);
            }
            if current == key {
                let h = Anchor {
                    slot: i,
                    generation: n.flags >> 2,
                };
                if f(h, t) {
                    return Some(h);
                }
            }
            if current <= key {
                return find(t, n.r, child_offset, key, f);
            }
            None
        }
        find(self, self.root, 0, (pos, bias == Bias::Right), &predicate)
    }
    /// Whether any anchor lies at a position in A..=B.
    pub fn any_between(&self, a: u32, b: u32) -> bool {
        fn any(t: &AnchorTree, i: u32, offset: u32, a: u32, b: u32) -> bool {
            if i == NIL {
                return false;
            }
            let n = t.n[i as usize];
            let pos = n.pos.wrapping_add(offset);
            let child = offset.wrapping_add(n.lazy);
            (a..=b).contains(&pos)
                || (pos >= a && any(t, n.l, child, a, b))
                || (pos <= b && any(t, n.r, child, a, b))
        }
        any(self, self.root, 0, a, b)
    }
    /// When H was created relative to the tree's other anchors: a later
    /// anchor has a larger number.
    pub fn birth(&self, h: Anchor) -> Option<u64> {
        self.valid(h).then(|| self.births[h.slot as usize])
    }
    #[inline]
    pub fn bias(&self, h: Anchor) -> Option<Bias> {
        self.valid(h).then(|| {
            if self.n[h.slot as usize].flags & 1 == 0 {
                Bias::Left
            } else {
                Bias::Right
            }
        })
    }
    pub fn create(&mut self, pos: u32, bias: Bias) -> Option<Anchor> {
        if pos > self.extent {
            return None;
        }
        let i = if self.free != NIL {
            let i = self.free;
            self.free = self.n[i as usize].l;
            i
        } else {
            let i = u32::try_from(self.n.len()).ok().filter(|&i| i != NIL)?;
            self.n.push(Node {
                l: NIL,
                r: NIL,
                p: NIL,
                pos,
                lazy: 0,
                flags: 0,
            });
            self.births.push(0);
            i
        };
        self.births[i as usize] = self.born;
        self.born += 1;
        let generation = self.n[i as usize].flags >> 2;
        self.n[i as usize] = Node {
            l: NIL,
            r: NIL,
            p: NIL,
            pos,
            lazy: 0,
            flags: (generation << 2) | 2 | u32::from(bias == Bias::Right),
        };
        self.place(i);
        self.len += 1;
        Some(Anchor {
            slot: i,
            generation,
        })
    }
    pub fn remove(&mut self, h: Anchor) -> bool {
        if !self.valid(h) {
            return false;
        }
        self.detach(h.slot);
        self.len -= 1;
        let n = &mut self.n[h.slot as usize];
        // Retire exhausted generations instead of allowing stale-handle ABA.
        n.flags &= !3;
        if h.generation < (u32::MAX >> 2) {
            n.flags += 4;
            n.l = self.free;
            self.free = h.slot;
        }
        true
    }
    #[inline]
    pub fn set(&mut self, h: Anchor, pos: u32, bias: Bias) -> bool {
        if pos > self.extent || !self.valid(h) {
            return false;
        }
        if self.resolve(h) == Some(pos) && self.bias(h) == Some(bias) {
            return true;
        }
        self.detach(h.slot);
        let n = &mut self.n[h.slot as usize];
        n.pos = pos;
        n.flags = (n.flags & !1) | u32::from(bias == Bias::Right);
        self.place(h.slot);
        true
    }
    /// Text grows by LEN at the end and no anchor moves, whatever its bias:
    /// insdel.c insert_from_gap_1, which adjusts no marker.
    pub(crate) fn grow(&mut self, len: u32) -> bool {
        let Some(extent) = self.extent.checked_add(len) else { return false };
        self.extent = extent;
        true
    }
    pub(crate) fn extend_empty(&mut self, extent: u32) {
        debug_assert!(self.is_empty());
        debug_assert!(extent >= self.extent);
        self.extent = extent;
    }
    /// Atomic checked insertion. `before_markers` advances both biases at pos.
    #[inline]
    pub fn insert(&mut self, pos: u32, len: u32, before_markers: bool) -> bool {
        let Some(extent) = self.extent.checked_add(len) else {
            return false;
        };
        if pos > self.extent {
            return false;
        }
        if self.is_empty() {
            self.extent = extent;
            return true;
        }
        let (a, b) = self.split(self.root, (pos, !before_markers));
        self.apply(b, len);
        let root = self.merge(a, b);
        self.link_root(root);
        self.extent = extent;
        true
    }
    #[inline]
    pub fn delete(&mut self, a: u32, b: u32) -> bool {
        if a > b || b > self.extent {
            return false;
        }
        if a == b {
            return true;
        }
        if self.is_empty() {
            self.extent -= b - a;
            return true;
        }
        let (lo, rest) = if a == u32::MAX {
            (self.root, NIL)
        } else {
            self.split(self.root, (a + 1, false))
        };
        let (mid, hi) = if b == u32::MAX {
            (rest, NIL)
        } else {
            self.split(rest, (b + 1, false))
        };
        self.apply(hi, (b - a).wrapping_neg());
        self.extent -= b - a;
        if mid == NIL {
            let root = self.merge(lo, hi);
            self.link_root(root);
            return true;
        }
        let (left, right) = self.collapse(mid, a);
        let (lo1, lo2) = self.split(lo, (a, true));
        let t = self.merge(lo1, left);
        let t = self.merge(t, lo2);
        let t = self.merge(t, right);
        let root = self.merge(t, hi);
        self.link_root(root);
        true
    }
    /// Linear Cartesian-tree build; input must be sorted by (position, bias).
    /// Returned handles are in input order. Validation precedes construction.
    pub fn from_sorted(extent: u32, positions: &[(u32, Bias)]) -> Option<(Self, Vec<Anchor>)> {
        if positions.len() >= NIL as usize
            || positions.iter().any(|x| x.0 > extent)
            || positions.windows(2).any(|w| w[0] > w[1])
        {
            return None;
        }
        let mut tree = Self::new(extent);
        tree.n.reserve_exact(positions.len());
        let mut handles = Vec::with_capacity(positions.len());
        let mut last = NIL;
        for (i, &(pos, bias)) in positions.iter().enumerate() {
            let i = i as u32;
            let mut parent = last;
            let mut left = NIL;
            while parent != NIL && prio(parent) < prio(i) {
                left = parent;
                parent = tree.n[parent as usize].p;
            }
            tree.n.push(Node {
                l: left,
                r: NIL,
                p: parent,
                pos,
                lazy: 0,
                flags: 2 | u32::from(bias == Bias::Right),
            });
            if left != NIL {
                tree.n[left as usize].p = i;
            }
            if parent != NIL {
                tree.n[parent as usize].r = i;
            } else {
                tree.root = i;
            }
            last = i;
            handles.push(Anchor {
                slot: i,
                generation: 0,
            });
        }
        tree.len = positions.len() as u32;
        tree.births = (0..positions.len() as u64).collect();
        tree.born = positions.len() as u64;
        Some((tree, handles))
    }
}

#[cfg(test)]
#[path = "anchor_tests.rs"]
mod tests;
