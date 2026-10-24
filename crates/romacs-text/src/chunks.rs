//! An indexed AVL sequence. Structural edits touch only root-to-leaf paths;
//! byte, character and newline queries always see current subtree summaries.
use super::{Chunk, Sum};
use std::ops::Index;
#[cfg(test)]
mod tests;

type Link = Option<Box<Node>>;
#[derive(Clone)]
struct Node {
    chunk: Chunk,
    left: Link,
    right: Link,
    height: usize,
    len: usize,
    sum: Sum,
}
fn height(n: &Link) -> usize {
    n.as_ref().map_or(0, |n| n.height)
}
fn len(n: &Link) -> usize {
    n.as_ref().map_or(0, |n| n.len)
}
fn sum(n: &Link) -> Sum {
    n.as_ref().map_or(Sum::default(), |n| n.sum)
}
impl Node {
    fn refresh(&mut self, _work: &mut usize) {
        #[cfg(test)]
        {
            *_work += 1;
            super::TOPOLOGY_REFRESHES.with(|n| n.set(n.get() + 1));
        }
        self.height = 1 + height(&self.left).max(height(&self.right));
        self.len = 1 + len(&self.left) + len(&self.right);
        self.sum = sum(&self.left)
            .plus(self.chunk.wide())
            .plus(sum(&self.right));
    }
    fn rotate_left(mut self: Box<Self>, work: &mut usize) -> Box<Self> {
        let mut right = self.right.take().unwrap();
        self.right = right.left.take();
        self.refresh(work);
        right.left = Some(self);
        right.refresh(work);
        right
    }
    fn rotate_right(mut self: Box<Self>, work: &mut usize) -> Box<Self> {
        let mut left = self.left.take().unwrap();
        self.left = left.right.take();
        self.refresh(work);
        left.right = Some(self);
        left.refresh(work);
        left
    }
    fn balance(mut self: Box<Self>, work: &mut usize) -> Box<Self> {
        self.refresh(work);
        if height(&self.left) > height(&self.right) + 1 {
            let left = self.left.as_ref().unwrap();
            if height(&left.right) > height(&left.left) {
                self.left = Some(self.left.take().unwrap().rotate_left(work));
            }
            self.rotate_right(work)
        } else if height(&self.right) > height(&self.left) + 1 {
            let right = self.right.as_ref().unwrap();
            if height(&right.left) > height(&right.right) {
                self.right = Some(self.right.take().unwrap().rotate_right(work));
            }
            self.rotate_left(work)
        } else {
            self
        }
    }
    fn insert(root: Link, i: usize, chunk: Chunk, work: &mut usize) -> Box<Self> {
        let Some(mut n) = root else {
            assert_eq!(i, 0);
            #[cfg(test)]
            {
                *work += 1;
            }
            return Box::new(Self {
                chunk,
                left: None,
                right: None,
                height: 1,
                len: 1,
                sum: chunk.wide(),
            });
        };
        let left = len(&n.left);
        if i <= left {
            n.left = Some(Self::insert(n.left.take(), i, chunk, work));
        } else {
            n.right = Some(Self::insert(n.right.take(), i - left - 1, chunk, work));
        }
        n.balance(work)
    }
    fn remove(mut n: Box<Self>, i: usize, work: &mut usize) -> (Link, Chunk) {
        let left = len(&n.left);
        let removed;
        if i < left {
            let (child, c) = Self::remove(n.left.take().unwrap(), i, work);
            n.left = child;
            removed = c;
        } else if i > left {
            let (child, c) = Self::remove(n.right.take().unwrap(), i - left - 1, work);
            n.right = child;
            removed = c;
        } else {
            removed = n.chunk;
            if n.left.is_none() {
                return (n.right, removed);
            }
            if n.right.is_none() {
                return (n.left, removed);
            }
            let (right, successor) = Self::remove(n.right.take().unwrap(), 0, work);
            n.right = right;
            n.chunk = successor;
        }
        (Some(n.balance(work)), removed)
    }
    fn adjust(&mut self, mut i: usize, old: Sum, new: Sum, _work: &mut usize) {
        let mut node = self;
        loop {
            node.sum.bytes = node.sum.bytes - old.bytes + new.bytes;
            node.sum.chars = node.sum.chars - old.chars + new.chars;
            node.sum.nl = node.sum.nl - old.nl + new.nl;
            #[cfg(test)]
            {
                *_work += 1;
            }
            let left = len(&node.left);
            if i < left {
                node = node.left.as_deref_mut().unwrap();
            } else if i > left {
                i -= left + 1;
                node = node.right.as_deref_mut().unwrap();
            } else {
                node.chunk = new.into();
                return;
            }
        }
    }
    fn add(&mut self, mut i: usize, ins: Sum, _work: &mut usize) {
        let mut node = self;
        loop {
            #[cfg(test)]
            {
                *_work += 1;
            }
            node.sum = node.sum.plus(ins);
            let left = len(&node.left);
            if i < left {
                node = node.left.as_deref_mut().unwrap();
            } else if i > left {
                i -= left + 1;
                node = node.right.as_deref_mut().unwrap();
            } else {
                node.chunk = node.chunk.wide().plus(ins).into();
                break;
            }
        }
    }
    fn insert_summary(
        &mut self,
        key: usize,
        ins: Sum,
        mut prefix: Sum,
        mut rank: usize,
        _work: &mut usize,
    ) -> Option<(usize, Sum, Chunk)> {
        let root = self;
        let mut node = &mut *root;
        let result = loop {
            let before = prefix.plus(sum(&node.left));
            let left = len(&node.left);
            node.sum = node.sum.plus(ins);
            #[cfg(test)]
            {
                *_work += 1;
            }
            if key < before.bytes {
                node = node.left.as_deref_mut().unwrap();
            } else if key <= before.bytes + node.chunk.bytes as usize {
                let old = node.chunk;
                let rank = rank + left;
                if old.bytes as usize + ins.bytes > 2 * super::T {
                    break (rank, before, old, false);
                }
                node.chunk = old.wide().plus(ins).into();
                break (rank, before, old, true);
            } else {
                prefix = before.plus(node.chunk.wide());
                rank += left + 1;
                node = node.right.as_deref_mut().unwrap();
            }
        };
        let (mut i, before, old, fits) = result;
        if fits {
            return Some((i, before, old));
        }
        // Rare structural case: undo only the optimistic ancestor summaries.
        // The chunk itself is unchanged and the generic structural edit follows.
        let mut node = root;
        loop {
            node.sum.bytes -= ins.bytes;
            node.sum.chars -= ins.chars;
            node.sum.nl -= ins.nl;
            #[cfg(test)]
            {
                *_work += 1;
            }
            let left = len(&node.left);
            if i < left {
                node = node.left.as_deref_mut().unwrap();
            } else if i > left {
                i -= left + 1;
                node = node.right.as_deref_mut().unwrap();
            } else {
                break;
            }
        }
        None
    }
}
#[derive(Clone, Default)]
pub(super) struct Chunks {
    root: Link,
    // The last chunk has one owner and is excluded from every AVL summary.
    tail: Option<Chunk>,
    // Only counted in test builds; optimized builds discard this local counter.
    #[cfg(test)]
    pub(super) work: usize,
}
impl Chunks {
    pub(super) fn from_pieces(mut chunks: Vec<Chunk>) -> Self {
        fn build(chunks: &[Chunk]) -> Link {
            if chunks.is_empty() {
                return None;
            }
            let mid = chunks.len() / 2;
            let left = build(&chunks[..mid]);
            let right = build(&chunks[mid + 1..]);
            let node = Node {
                chunk: chunks[mid],
                height: 1 + height(&left).max(height(&right)),
                len: chunks.len(),
                sum: sum(&left).plus(chunks[mid].wide()).plus(sum(&right)),
                left,
                right,
            };
            Some(Box::new(node))
        }
        let tail = chunks.pop();
        Self {
            root: build(&chunks),
            tail,
            #[cfg(test)]
            work: chunks.len() + usize::from(tail.is_some()),
        }
    }
    pub(super) fn len(&self) -> usize {
        len(&self.root) + usize::from(self.tail.is_some())
    }
    fn record(&mut self, _work: usize) {
        #[cfg(test)]
        {
            self.work += _work;
        }
    }
    pub(super) fn insert(&mut self, i: usize, chunk: Chunk) {
        assert!(i <= self.len());
        let mut work = 0;
        if i == self.len() {
            if let Some(old) = self.tail.replace(chunk) {
                self.root = Some(Node::insert(self.root.take(), i - 1, old, &mut work));
            }
        } else {
            self.root = Some(Node::insert(self.root.take(), i, chunk, &mut work));
        }
        self.record(work);
    }
    pub(super) fn remove(&mut self, i: usize) -> Chunk {
        assert!(i < self.len());
        let mut work = 0;
        let root_len = len(&self.root);
        let chunk = if i == root_len {
            let chunk = self.tail.take().unwrap();
            if let Some(root) = self.root.take() {
                let (root, tail) = Node::remove(root, root_len - 1, &mut work);
                self.root = root;
                self.tail = Some(tail);
            }
            chunk
        } else {
            let (root, chunk) = Node::remove(self.root.take().unwrap(), i, &mut work);
            self.root = root;
            chunk
        };
        self.record(work);
        chunk
    }
    pub(super) fn set(&mut self, i: usize, chunk: Chunk) {
        assert!(i < self.len());
        if i == len(&self.root) {
            self.tail = Some(chunk);
            return;
        }
        let mut work = 0;
        let old = self[i].wide();
        self.root
            .as_mut()
            .unwrap()
            .adjust(i, old, chunk.wide(), &mut work);
        self.record(work);
    }
    pub(super) fn replace_summary(&mut self, i: usize, old: Sum, new: Sum) {
        if i == len(&self.root) {
            *self.tail.as_mut().unwrap() = new.into();
            return;
        }
        let mut work = 0;
        self.root
            .as_deref_mut()
            .unwrap()
            .adjust(i, old, new, &mut work);
        self.record(work);
    }
    pub(super) fn add(&mut self, i: usize, ins: Sum) {
        if i == len(&self.root) {
            let tail = self.tail.as_mut().unwrap();
            *tail = tail.wide().plus(ins).into();
            return;
        }
        let mut work = 0;
        self.root.as_deref_mut().unwrap().add(i, ins, &mut work);
        self.record(work);
    }
    pub(super) fn insert_summary(&mut self, key: usize, ins: Sum) -> Option<(usize, Sum, Chunk)> {
        let prefix = sum(&self.root);
        // At the root/tail boundary retain the old tree's inclusive selection.
        if self.root.is_none() || key > prefix.bytes {
            let tail = self.tail.as_mut()?;
            assert!(key <= prefix.bytes + tail.bytes as usize);
            let old = *tail;
            if old.bytes as usize + ins.bytes > 2 * super::T {
                return None;
            }
            *tail = old.wide().plus(ins).into();
            return Some((len(&self.root), prefix, old));
        }
        let mut work = 0;
        let found =
            self.root
                .as_deref_mut()?
                .insert_summary(key, ins, Sum::default(), 0, &mut work);
        self.record(work);
        found
    }
    pub(super) fn total(&self) -> Sum {
        sum(&self.root).plus(self.tail.map_or(Sum::default(), Chunk::wide))
    }
    #[cfg(test)]
    pub(super) fn prefix(&self, mut i: usize) -> Sum {
        assert!(i <= self.len());
        if i == self.len() {
            return self.total();
        }
        let mut out = Sum::default();
        let mut n = self.root.as_deref();
        while let Some(node) = n {
            let left = len(&node.left);
            if i <= left {
                n = node.left.as_deref();
            } else {
                out = out.plus(sum(&node.left)).plus(node.chunk.wide());
                i -= left + 1;
                n = node.right.as_deref();
            }
        }
        out
    }
    pub(super) fn locate(&self, key: usize, field: fn(Sum) -> usize) -> (usize, Sum) {
        let root_sum = sum(&self.root);
        if key >= field(root_sum) && key < field(self.total()) {
            return (len(&self.root), root_sum);
        }
        #[cfg(test)]
        super::TREE_LOOKUPS.with(|n| n.set(n.get() + 1));
        let mut prefix = Sum::default();
        let mut index = 0;
        let mut n = self.root.as_deref();
        while let Some(node) = n {
            let before = prefix.plus(sum(&node.left));
            if key < field(before) {
                n = node.left.as_deref();
            } else {
                let after = before.plus(node.chunk.wide());
                if key < field(after) {
                    return (index + len(&node.left), before);
                }
                index += len(&node.left) + 1;
                prefix = after;
                n = node.right.as_deref();
            }
        }
        panic!("summary key outside chunks");
    }
    pub(super) fn check(&self) {
        fn visit(n: &Link) -> (usize, usize, Sum) {
            let Some(n) = n else {
                return (0, 0, Sum::default());
            };
            let (lh, ll, ls) = visit(&n.left);
            let (rh, rl, rs) = visit(&n.right);
            assert!(lh.abs_diff(rh) <= 1);
            assert_eq!(n.height, 1 + lh.max(rh));
            assert_eq!(n.len, 1 + ll + rl);
            assert_eq!(n.sum, ls.plus(n.chunk.wide()).plus(rs));
            (n.height, n.len, n.sum)
        }
        visit(&self.root);
        assert!(self.tail.is_some() || self.root.is_none());
    }
}
impl Index<usize> for Chunks {
    type Output = Chunk;
    fn index(&self, mut i: usize) -> &Chunk {
        if i == len(&self.root) {
            return self.tail.as_ref().expect("chunk index outside chunks");
        }
        #[cfg(test)]
        super::TREE_LOOKUPS.with(|n| n.set(n.get() + 1));
        assert!(i < self.len());
        let mut n = self.root.as_deref().unwrap();
        loop {
            let left = len(&n.left);
            if i < left {
                n = n.left.as_deref().unwrap();
            } else if i > left {
                i -= left + 1;
                n = n.right.as_deref().unwrap();
            } else {
                return &n.chunk;
            }
        }
    }
}
