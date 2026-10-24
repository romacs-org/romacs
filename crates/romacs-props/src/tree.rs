use crate::SetId;
const CAP: usize = 64;
const FAN: usize = 16;
const NIL: u32 = u32::MAX;
const TAG: u32 = 1 << 31;
#[derive(Clone, Debug)]
pub(crate) struct Leaf {
    pub lens: [u32; CAP],
    pub sets: [SetId; CAP],
    pub n: usize,
    parent: u32,
    pub prev: u32,
    pub next: u32,
    pub chars: usize,
    flags: u8,
    dirty: bool,
}
impl Leaf {
    fn new(parent: u32) -> Self {
        Self {
            lens: [0; CAP],
            sets: [SetId::EMPTY; CAP],
            n: 0,
            parent,
            prev: NIL,
            next: NIL,
            chars: 0,
            flags: 0,
            dirty: false,
        }
    }
    pub fn insert(&mut self, i: usize, len: u32, set: SetId) {
        self.lens.copy_within(i..self.n, i + 1);
        self.sets.copy_within(i..self.n, i + 1);
        self.lens[i] = len;
        self.sets[i] = set;
        self.n += 1;
        self.chars += len as usize;
    }
    pub fn remove(&mut self, i: usize) {
        self.chars -= self.lens[i] as usize;
        self.lens.copy_within(i + 1..self.n, i);
        self.sets.copy_within(i + 1..self.n, i);
        self.n -= 1;
    }
}
#[derive(Clone, Debug)]
struct Branch {
    children: [u32; FAN],
    chars: [usize; FAN],
    flags: [u8; FAN],
    n: usize,
    parent: u32,
}
impl Branch {
    fn new(parent: u32) -> Self {
        Self {
            children: [0; FAN],
            chars: [0; FAN],
            flags: [0; FAN],
            n: 0,
            parent,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Tree {
    pub leaves: Vec<Leaf>,
    branches: Vec<Branch>,
    free_leaves: Vec<u32>,
    free_branches: Vec<u32>,
    root: u32,
    pub first: u32,
    pub last: u32,
    dirty: Vec<u32>,
    #[cfg(test)]
    pub work: std::cell::Cell<usize>,
}
impl Default for Tree {
    fn default() -> Self {
        Self {
            leaves: Vec::new(),
            branches: vec![Branch::new(NIL)],
            free_leaves: Vec::new(),
            free_branches: Vec::new(),
            root: 0,
            first: NIL,
            last: NIL,
            dirty: Vec::new(),
            #[cfg(test)]
            work: std::cell::Cell::new(0),
        }
    }
}
impl Tree {
    #[inline]
    fn count(&self, n: usize) {
        #[cfg(test)]
        self.work.set(self.work.get() + n);
        #[cfg(not(test))]
        let _ = n;
    }
    #[inline]
    fn mark_dirty(&mut self, id: u32) {
        let l = &mut self.leaves[id as usize];
        if !l.dirty {
            l.dirty = true;
            self.dirty.push(id);
        }
    }
    // Account at the inspected run, not once per API invocation.
    #[inline]
    fn append_tail(&self) -> Option<(SetId, u32)> {
        let l = &self.leaves[self.last as usize];
        if l.n == 0 {
            return None;
        }
        self.count(1);
        Some((l.sets[l.n - 1], l.lens[l.n - 1]))
    }
    pub fn bytes(&self) -> usize {
        self.leaves.capacity() * std::mem::size_of::<Leaf>()
            + self.branches.capacity() * std::mem::size_of::<Branch>()
            + (self.free_leaves.capacity() + self.free_branches.capacity() + self.dirty.capacity())
                * 4
    }
    fn alloc_leaf(&mut self, parent: u32) -> u32 {
        if let Some(i) = self.free_leaves.pop() {
            let dirty = self.leaves[i as usize].dirty;
            self.leaves[i as usize] = Leaf::new(parent);
            // A queued slot remains queued across free/reuse until refreshed.
            self.leaves[i as usize].dirty = dirty;
            i
        } else {
            let i = u32::try_from(self.leaves.len()).unwrap();
            assert!(i < TAG);
            self.leaves.push(Leaf::new(parent));
            i
        }
    }
    fn alloc_branch(&mut self, parent: u32) -> u32 {
        if let Some(i) = self.free_branches.pop() {
            self.branches[i as usize] = Branch::new(parent);
            i
        } else {
            let i = u32::try_from(self.branches.len()).unwrap();
            assert!(i < TAG);
            self.branches.push(Branch::new(parent));
            i
        }
    }
    fn parent(&self, child: u32) -> u32 {
        if child & TAG != 0 {
            self.leaves[(child ^ TAG) as usize].parent
        } else {
            self.branches[child as usize].parent
        }
    }
    fn set_parent(&mut self, child: u32, p: u32) {
        if child & TAG != 0 {
            self.leaves[(child ^ TAG) as usize].parent = p
        } else {
            self.branches[child as usize].parent = p
        }
    }
    fn summary(&self, child: u32) -> (usize, u8) {
        self.count(1);
        if child & TAG != 0 {
            (
                self.leaves[(child ^ TAG) as usize].chars,
                self.leaves[(child ^ TAG) as usize].flags,
            )
        } else {
            let b = &self.branches[child as usize];
            self.count(2 * b.n);
            (
                b.chars[..b.n].iter().sum(),
                b.flags[..b.n].iter().fold(0, |a, b| a | b),
            )
        }
    }
    fn refresh(&mut self, mut child: u32) {
        loop {
            let p = self.parent(child);
            if p == NIL {
                break;
            }
            self.count(1);
            let (chars, flags) = self.summary(child);
            let b = &mut self.branches[p as usize];
            let i = b.children[..b.n].iter().position(|&c| c == child).unwrap();
            b.chars[i] = chars;
            b.flags[i] = flags;
            child = p;
        }
    }
    pub fn set_len(&mut self, leaf: u32, i: usize, len: u32) {
        let l = &mut self.leaves[leaf as usize];
        l.chars = l.chars - l.lens[i] as usize + len as usize;
        l.lens[i] = len;
        self.refresh(leaf | TAG);
    }
    pub fn paint(
        &mut self,
        mut id: u32,
        mut i: usize,
        before: u32,
        middle: u32,
        after: u32,
        set: SetId,
    ) -> (u32, usize) {
        let pieces = usize::from(before > 0) + 1 + usize::from(after > 0);
        if self.leaves[id as usize].n + pieces - 1 > CAP {
            let right = self.split(id);
            let n = self.leaves[id as usize].n;
            if i >= n {
                id = right;
                i -= n;
            }
        }
        self.mark_dirty(id);
        let l = &mut self.leaves[id as usize];
        let old = l.sets[i];
        l.lens.copy_within(i + 1..l.n, i + pieces);
        l.sets.copy_within(i + 1..l.n, i + pieces);
        let mut j = i;
        if before > 0 {
            l.lens[j] = before;
            l.sets[j] = old;
            j += 1;
        }
        l.lens[j] = middle;
        l.sets[j] = set;
        j += 1;
        if after > 0 {
            l.lens[j] = after;
            l.sets[j] = old;
        }
        l.n += pieces - 1;
        (id, i + usize::from(before > 0))
    }
    pub fn append(&mut self, len: u32, set: SetId) -> bool {
        if self.last == NIL {
            return false;
        }
        let tail = self.append_tail();
        self.mark_dirty(self.last);
        let l = &mut self.leaves[self.last as usize];
        if tail.is_some_and(|(s, n)| s == set && n <= u32::MAX - len) {
            l.lens[l.n - 1] += len;
        } else if l.n < CAP {
            l.lens[l.n] = len;
            l.sets[l.n] = set;
            l.n += 1;
        } else {
            let old = self.last;
            let parent = l.parent;
            let next = self.alloc_leaf(parent);
            {
                let l = &mut self.leaves[next as usize];
                l.prev = old;
                l.insert(0, len, set);
            }
            self.mark_dirty(next);
            self.leaves[old as usize].next = next;
            let at = self.branches[parent as usize].n;
            self.insert_child(parent, at, next | TAG);
            self.last = next;
            return true;
        }
        l.chars += len as usize;
        let mut parent = l.parent;
        loop {
            self.count(1);
            let b = &mut self.branches[parent as usize];
            b.chars[b.n - 1] += len as usize;
            if b.parent == NIL {
                break;
            }
            parent = b.parent;
        }
        true
    }
    fn insert_child(&mut self, p: u32, at: usize, child: u32) {
        self.count(1);
        if self.branches[p as usize].n == FAN {
            let parent = self.branches[p as usize].parent;
            let right = self.alloc_branch(parent);
            let old = self.branches[p as usize].clone();
            self.branches[p as usize].n = FAN / 2;
            for j in FAN / 2..FAN {
                self.count(1);
                let c = old.children[j];
                let r = &mut self.branches[right as usize];
                let k = r.n;
                r.children[k] = c;
                r.chars[k] = old.chars[j];
                r.flags[k] = old.flags[j];
                r.n += 1;
                self.set_parent(c, right);
            }
            if parent == NIL {
                let root = self.alloc_branch(NIL);
                self.root = root;
                self.branches[p as usize].parent = root;
                self.branches[right as usize].parent = root;
                self.insert_child(root, 0, p);
                self.insert_child(root, 1, right);
            } else {
                self.refresh(p);
                let b = &self.branches[parent as usize];
                let idx = b.children[..b.n].iter().position(|&c| c == p).unwrap();
                self.insert_child(parent, idx + 1, right);
            }
            if at <= FAN / 2 {
                self.insert_child(p, at, child)
            } else {
                self.insert_child(right, at - FAN / 2, child)
            }
            return;
        }
        let (chars, flags) = self.summary(child);
        let b = &mut self.branches[p as usize];
        b.children.copy_within(at..b.n, at + 1);
        b.chars.copy_within(at..b.n, at + 1);
        b.flags.copy_within(at..b.n, at + 1);
        b.children[at] = child;
        b.chars[at] = chars;
        b.flags[at] = flags;
        b.n += 1;
        self.set_parent(child, p);
        self.refresh(p);
    }
    pub fn initial(&mut self) -> u32 {
        let id = self.alloc_leaf(self.root);
        self.first = id;
        self.last = id;
        self.insert_child(self.root, 0, id | TAG);
        id
    }
    pub fn split(&mut self, id: u32) -> u32 {
        let p = self.leaves[id as usize].parent;
        let right = self.alloc_leaf(p);
        self.mark_dirty(id);
        self.mark_dirty(right);
        let old = self.leaves[id as usize].clone();
        let mid = old.n / 2;
        for i in mid..old.n {
            self.count(1);
            self.leaves[right as usize].insert(i - mid, old.lens[i], old.sets[i]);
        }
        {
            let l = &mut self.leaves[id as usize];
            l.n = mid;
            l.chars = l.lens[..mid].iter().map(|&n| n as usize).sum();
            l.next = right;
        }
        {
            let l = &mut self.leaves[right as usize];
            l.prev = id;
            l.next = old.next;
        }
        if old.next != NIL {
            self.leaves[old.next as usize].prev = right;
        } else {
            self.last = right;
        }
        self.refresh(id | TAG);
        let b = &self.branches[p as usize];
        let at = b.children[..b.n]
            .iter()
            .position(|&c| c == (id | TAG))
            .unwrap();
        self.insert_child(p, at + 1, right | TAG);
        right
    }
    pub fn insert(&mut self, mut id: u32, mut i: usize, len: u32, set: SetId) -> (u32, usize) {
        if self.leaves[id as usize].n == CAP {
            let right = self.split(id);
            let n = self.leaves[id as usize].n;
            if i >= n {
                id = right;
                i -= n;
            }
        }
        self.mark_dirty(id);
        self.leaves[id as usize].insert(i, len, set);
        self.refresh(id | TAG);
        (id, i)
    }
    pub fn normalize(&mut self, id: u32) {
        self.mark_dirty(id);
        let l = &mut self.leaves[id as usize];
        let mut w = 0;
        for i in 0..l.n {
            let mut len = l.lens[i];
            let set = l.sets[i];
            if w > 0 && l.sets[w - 1] == set {
                let sum = l.lens[w - 1] as u64 + len as u64;
                l.lens[w - 1] = sum.min(u32::MAX as u64) as u32;
                len = sum.saturating_sub(u32::MAX as u64) as u32;
            }
            if len > 0 {
                l.lens[w] = len;
                l.sets[w] = set;
                w += 1;
            }
        }
        l.n = w;
    }
    pub fn remove(&mut self, id: u32, i: usize) {
        self.mark_dirty(id);
        self.leaves[id as usize].remove(i);
        if self.leaves[id as usize].n != 0 {
            self.refresh(id | TAG);
            if self.leaves[id as usize].n < 16 {
                let l = &self.leaves[id as usize];
                let (left, right) = if l.next != NIL {
                    (id, l.next)
                } else {
                    (l.prev, id)
                };
                if left != NIL
                    && self.leaves[left as usize].n + self.leaves[right as usize].n <= CAP
                {
                    self.mark_dirty(left);
                    self.mark_dirty(right);
                    let r = self.leaves[right as usize].clone();
                    let l = &mut self.leaves[left as usize];
                    for i in 0..r.n {
                        l.insert(l.n, r.lens[i], r.sets[i]);
                    }
                    self.leaves[right as usize].n = 0;
                    self.leaves[right as usize].chars = 0;
                    self.refresh(left | TAG);
                    self.remove_leaf(right);
                }
            }
            return;
        }
        self.remove_leaf(id);
    }
    fn remove_leaf(&mut self, id: u32) {
        let l = &self.leaves[id as usize];
        let (prev, next) = (l.prev, l.next);
        if prev == NIL {
            self.first = next
        } else {
            self.leaves[prev as usize].next = next;
        }
        if next == NIL {
            self.last = prev
        } else {
            self.leaves[next as usize].prev = prev;
        }
        let mut child = id | TAG;
        self.free_leaves.push(id);
        loop {
            let p = self.parent(child);
            let b = &mut self.branches[p as usize];
            let i = b.children[..b.n].iter().position(|&c| c == child).unwrap();
            b.children.copy_within(i + 1..b.n, i);
            b.chars.copy_within(i + 1..b.n, i);
            b.flags.copy_within(i + 1..b.n, i);
            b.n -= 1;
            if b.n != 0 || p == self.root {
                self.refresh(p);
                break;
            }
            child = p;
            self.free_branches.push(p);
        }
    }
    pub fn locate(&self, p: usize) -> (u32, usize, usize, usize, usize) {
        let (mut node, mut start, mut depth) = (self.root, 0, 0);
        loop {
            depth += 1;
            let b = &self.branches[node as usize];
            let mut found = None;
            for i in 0..b.n {
                if p < start + b.chars[i] {
                    found = Some(b.children[i]);
                    break;
                }
                start += b.chars[i];
            }
            let c = found.expect("property character out of bounds");
            if c & TAG != 0 {
                let leaf = c ^ TAG;
                let l = &self.leaves[leaf as usize];
                let leaf_end = start + l.chars;
                if p - start < (leaf_end - start) / 2 {
                    for i in 0..l.n {
                        let end = start + l.lens[i] as usize;
                        if p < end {
                            return (leaf, i, start, depth, leaf_end);
                        }
                        start = end;
                    }
                } else {
                    let mut end = leaf_end;
                    for i in (0..l.n).rev() {
                        start = end - l.lens[i] as usize;
                        if p >= start {
                            return (leaf, i, start, depth, leaf_end);
                        }
                        end = start;
                    }
                }
                unreachable!()
            }
            node = c;
        }
    }
    pub fn refresh_dirty_flags(&mut self, flags: impl Fn(SetId) -> u8) {
        while let Some(id) = self.dirty.pop() {
            let l = &mut self.leaves[id as usize];
            if !l.dirty {
                continue;
            }
            l.dirty = false;
            if l.n == 0 {
                continue;
            }
            let n = l.n;
            l.flags = l.sets[..n].iter().fold(0, |a, &s| a | flags(s));
            self.count(n);
            self.refresh(id | TAG);
        }
    }
    pub fn refresh_flags(&mut self, flags: impl Fn(SetId) -> u8) {
        for l in &mut self.leaves {
            l.dirty = false;
        }
        self.dirty.clear();
        fn walk(t: &mut Tree, id: u32, flags: &impl Fn(SetId) -> u8) -> u8 {
            if id & TAG != 0 {
                let n = t.leaves[(id ^ TAG) as usize].n;
                t.count(n);
                let l = &mut t.leaves[(id ^ TAG) as usize];
                l.flags = l.sets[..l.n].iter().fold(0, |a, &s| a | flags(s));
                l.flags
            } else {
                let n = t.branches[id as usize].n;
                let mut all = 0;
                for i in 0..n {
                    let c = t.branches[id as usize].children[i];
                    let f = walk(t, c, flags);
                    t.branches[id as usize].flags[i] = f;
                    all |= f;
                }
                all
            }
        }
        walk(self, self.root, &flags);
    }
    pub fn next_flagged(
        &self,
        from: usize,
        to: usize,
        mask: u8,
        flags: impl Fn(SetId) -> u8,
    ) -> Option<(usize, usize, SetId)> {
        fn walk(
            t: &Tree,
            id: u32,
            mut pos: usize,
            from: usize,
            to: usize,
            mask: u8,
            flags: &impl Fn(SetId) -> u8,
        ) -> Option<(usize, usize, SetId)> {
            if id & TAG != 0 {
                let l = &t.leaves[(id ^ TAG) as usize];
                for i in 0..l.n {
                    let end = pos + l.lens[i] as usize;
                    if end > from && pos < to && flags(l.sets[i]) & mask != 0 {
                        return Some((pos.max(from), end.min(to), l.sets[i]));
                    }
                    pos = end;
                    if pos >= to {
                        break;
                    }
                }
            } else {
                let b = &t.branches[id as usize];
                for i in 0..b.n {
                    let end = pos + b.chars[i];
                    if end > from
                        && pos < to
                        && b.flags[i] & mask != 0
                        && let Some(r) = walk(t, b.children[i], pos, from, to, mask, flags)
                    {
                        return Some(r);
                    }
                    pos = end;
                    if pos >= to {
                        break;
                    }
                }
            }
            None
        }
        walk(self, self.root, 0, from, to, mask, &flags)
    }
    pub fn is_nil(id: u32) -> bool {
        id == NIL
    }
    pub fn check(&self) {
        fn walk(t: &Tree, id: u32, parent: u32) -> usize {
            assert_eq!(t.parent(id), parent);
            if id & TAG != 0 {
                let l = &t.leaves[(id ^ TAG) as usize];
                assert!(l.n > 0 && l.n <= CAP);
                assert_eq!(l.chars, l.lens[..l.n].iter().map(|&x| x as usize).sum());
                l.chars
            } else {
                let b = &t.branches[id as usize];
                assert!(b.n <= FAN);
                let mut sum = 0;
                for i in 0..b.n {
                    let n = walk(t, b.children[i], id);
                    assert_eq!(b.chars[i], n);
                    sum += n;
                }
                sum
            }
        }
        walk(self, self.root, NIL);
    }
}
