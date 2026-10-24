//! Per-key annotation layers. Relative lengths move with text; no anchors.
//! Stamp zero is ABSENT, not Lisp nil. Value equality is identity, never `equal`.
//! A key has no implicit or visual baseline.
use crate::{Value, coordinates::ByteOffset};
use std::{collections::BTreeMap, ops::Range};
type Values = BTreeMap<String, Value>;
type Link = Option<Box<Tree>>;
const LEAF: usize = 32;
#[cfg(test)]
#[path = "properties_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    len: u32,
    stamp: u32,
    value: Value,
}
impl Run {
    fn same(self, other: Self) -> bool {
        self.stamp == other.stamp && self.value == other.value
    }
}
#[derive(Clone)]
enum Tree {
    Leaf(Vec<Run>, usize, usize),
    Branch {
        left: Box<Tree>,
        right: Box<Tree>,
        len: usize,
        present: usize,
        height: u32,
    },
}
impl Tree {
    fn leaf(runs: Vec<Run>) -> Self {
        let len = runs.iter().map(|r| r.len as usize).sum();
        let present = runs
            .iter()
            .filter(|r| r.stamp != 0)
            .map(|r| r.len as usize)
            .sum();
        Self::Leaf(runs, len, present)
    }
    fn len(&self) -> usize {
        match self {
            Self::Leaf(_, len, _) => *len,
            Self::Branch { len, .. } => *len,
        }
    }
    fn present(&self) -> usize {
        match self {
            Self::Leaf(_, _, present) => *present,
            Self::Branch { present, .. } => *present,
        }
    }
    fn height(&self) -> u32 {
        match self {
            Self::Leaf(..) => 1,
            Self::Branch { height, .. } => *height,
        }
    }
    fn branch(left: Box<Self>, right: Box<Self>) -> Box<Self> {
        Box::new(Self::Branch {
            len: left.len() + right.len(),
            present: left.present() + right.present(),
            height: 1 + left.height().max(right.height()),
            left,
            right,
        })
    }
    fn balance(left: Box<Self>, right: Box<Self>) -> Box<Self> {
        if left.height() > right.height() + 1 {
            let Self::Branch {
                left: a, right: b, ..
            } = *left
            else {
                unreachable!()
            };
            if a.height() >= b.height() {
                Self::branch(a, Self::branch(b, right))
            } else {
                let Self::Branch {
                    left: b, right: c, ..
                } = *b
                else {
                    unreachable!()
                };
                Self::branch(Self::branch(a, b), Self::branch(c, right))
            }
        } else if right.height() > left.height() + 1 {
            let Self::Branch {
                left: b, right: c, ..
            } = *right
            else {
                unreachable!()
            };
            if c.height() >= b.height() {
                Self::branch(Self::branch(left, b), c)
            } else {
                let Self::Branch {
                    left: a, right: b, ..
                } = *b
                else {
                    unreachable!()
                };
                Self::branch(Self::branch(left, a), Self::branch(b, c))
            }
        } else {
            Self::branch(left, right)
        }
    }
    fn join(left: Link, right: Link, work: &mut usize) -> Link {
        match (left, right) {
            (None, b) => b,
            (a, None) => a,
            (Some(a), Some(b)) => {
                *work += 1;
                if let (Self::Leaf(x, ..), Self::Leaf(y, ..)) = (&*a, &*b)
                    && x.len() + y.len() <= LEAF
                {
                    let Self::Leaf(mut x, ..) = *a else {
                        unreachable!()
                    };
                    let Self::Leaf(y, ..) = *b else {
                        unreachable!()
                    };
                    x.extend(y);
                    return Some(Box::new(Self::leaf(x)));
                }
                if a.height() > b.height() + 1 {
                    let Self::Branch { left, right, .. } = *a else {
                        unreachable!()
                    };
                    Some(Self::balance(
                        left,
                        Self::join(Some(right), Some(b), work).unwrap(),
                    ))
                } else if b.height() > a.height() + 1 {
                    let Self::Branch { left, right, .. } = *b else {
                        unreachable!()
                    };
                    Some(Self::balance(
                        Self::join(Some(a), Some(left), work).unwrap(),
                        right,
                    ))
                } else {
                    Some(Self::branch(a, b))
                }
            }
        }
    }
    fn split(tree: Link, at: usize, work: &mut usize) -> (Link, Link) {
        let Some(tree) = tree else {
            return (None, None);
        };
        *work += 1;
        if at == 0 {
            return (None, Some(tree));
        }
        if at >= tree.len() {
            return (Some(tree), None);
        }
        match *tree {
            Self::Leaf(runs, ..) => {
                let (mut a, mut b, mut p) = (Vec::new(), Vec::new(), 0);
                for r in runs {
                    let end = p + r.len as usize;
                    if end <= at {
                        a.push(r)
                    } else if p >= at {
                        b.push(r)
                    } else {
                        a.push(Run {
                            len: (at - p) as u32,
                            ..r
                        });
                        b.push(Run {
                            len: (end - at) as u32,
                            ..r
                        });
                    }
                    p = end;
                }
                (Some(Box::new(Self::leaf(a))), Some(Box::new(Self::leaf(b))))
            }
            Self::Branch { left, right, .. } => {
                let n = left.len();
                if at < n {
                    let (a, b) = Self::split(Some(left), at, work);
                    (a, Self::join(b, Some(right), work))
                } else {
                    let (a, b) = Self::split(Some(right), at - n, work);
                    (Self::join(Some(left), a, work), b)
                }
            }
        }
    }
    /// Small edits entirely within one leaf reuse its allocation and its tree
    /// path. The generic split/concat path handles cross-leaf or growing edits.
    fn edit_local(&mut self, start: usize, end: usize, inserted: usize, work: &mut usize) -> bool {
        *work += 1;
        match self {
            Self::Leaf(runs, len, present) => {
                if inserted > u32::MAX as usize {
                    return false;
                }
                // Edits contained in one absent run change only its length.
                // Likewise deletion inside a present run retains its value.
                // This is a per-key semantic fast path, not an anchor/cache.
                let mut base = 0;
                for r in runs.iter_mut() {
                    let boundary = base + r.len as usize;
                    if start < boundary {
                        if end > boundary {
                            break;
                        }
                        let resized = r.len as usize - (end - start) + inserted;
                        if (r.stamp == 0 || inserted == 0)
                            && resized > 0
                            && resized <= u32::MAX as usize
                        {
                            *len = *len - (end - start) + inserted;
                            if r.stamp != 0 {
                                *present -= end - start;
                            }
                            r.len = resized as u32;
                            return true;
                        }
                        break;
                    }
                    base = boundary;
                }
                let mut output = [Run {
                    len: 0,
                    stamp: 0,
                    value: Value::NIL,
                }; LEAF + 3];
                let mut count = 0;
                let mut push = |run: Run| {
                    if run.len == 0 {
                        return;
                    }
                    if count > 0
                        && output[count - 1].same(run)
                        && output[count - 1].len.checked_add(run.len).is_some()
                    {
                        output[count - 1].len += run.len;
                    } else {
                        output[count] = run;
                        count += 1;
                    }
                };
                let mut at = 0;
                for r in runs.iter() {
                    let limit = at + r.len as usize;
                    if at < start {
                        push(Run {
                            len: (limit.min(start) - at) as u32,
                            ..*r
                        });
                    }
                    at = limit;
                }
                push(Run {
                    len: inserted as u32,
                    stamp: 0,
                    value: Value::NIL,
                });
                at = 0;
                for r in runs.iter() {
                    let limit = at + r.len as usize;
                    if limit > end {
                        push(Run {
                            len: (limit - at.max(end)) as u32,
                            ..*r
                        });
                    }
                    at = limit;
                }
                if count == 0 || count > LEAF {
                    return false;
                }
                runs.clear();
                runs.extend_from_slice(&output[..count]);
                *len = runs.iter().map(|r| r.len as usize).sum();
                *present = runs
                    .iter()
                    .filter(|r| r.stamp != 0)
                    .map(|r| r.len as usize)
                    .sum();
                true
            }
            Self::Branch {
                left,
                right,
                len,
                present,
                ..
            } => {
                let boundary = left.len();
                let changed = if end <= boundary {
                    left.edit_local(start, end, inserted, work)
                } else if start >= boundary {
                    right.edit_local(start - boundary, end - boundary, inserted, work)
                } else {
                    false
                };
                if changed {
                    *len = left.len() + right.len();
                    *present = left.present() + right.present();
                }
                changed
            }
        }
    }
    fn edge(&self, last: bool) -> Run {
        match self {
            Self::Leaf(v, ..) => {
                if last {
                    *v.last().unwrap()
                } else {
                    v[0]
                }
            }
            Self::Branch { left, right, .. } => {
                if last {
                    right.edge(true)
                } else {
                    left.edge(false)
                }
            }
        }
    }
    fn concat(a: Link, b: Link, work: &mut usize) -> Link {
        if let (Some(x), Some(y)) = (&a, &b) {
            let l = x.edge(true);
            let r = y.edge(false);
            if l.same(r) && l.len.checked_add(r.len).is_some() {
                let n = x.len();
                let (a, _) = Self::split(a, n - l.len as usize, work);
                let (_, b) = Self::split(b, r.len as usize, work);
                let mid = Some(Box::new(Self::leaf(vec![Run {
                    len: l.len + r.len,
                    ..l
                }])));
                let a = Self::join(a, mid, work);
                return Self::join(a, b, work);
            }
        }
        Self::join(a, b, work)
    }
    fn uniform(mut len: usize, stamp: u32, value: Value, work: &mut usize) -> Link {
        let mut tree = None;
        while len > 0 {
            let n = len.min(u32::MAX as usize);
            let run = Some(Box::new(Self::leaf(vec![Run {
                len: n as u32,
                stamp,
                value,
            }])));
            tree = Self::join(tree, run, work);
            len -= n;
        }
        tree
    }
    fn at(&self, mut at: usize) -> Option<Run> {
        match self {
            Self::Leaf(v, ..) => {
                for r in v {
                    if at < r.len as usize {
                        return Some(*r);
                    }
                    at -= r.len as usize;
                }
                None
            }
            Self::Branch { left, right, .. } => {
                if at < left.len() {
                    left.at(at)
                } else {
                    right.at(at - left.len())
                }
            }
        }
    }
    fn walk(&self, base: usize, f: &mut impl FnMut(usize, Run)) {
        match self {
            Self::Leaf(v, ..) => {
                let mut p = base;
                for r in v {
                    f(p, *r);
                    p += r.len as usize;
                }
            }
            Self::Branch { left, right, .. } => {
                left.walk(base, f);
                right.walk(base + left.len(), f);
            }
        }
    }
    fn renumber(&mut self, stamps: &BTreeMap<u32, u32>) {
        match self {
            Self::Leaf(runs, ..) => {
                for run in runs {
                    if run.stamp != 0 {
                        run.stamp = stamps[&run.stamp];
                    }
                }
            }
            Self::Branch { left, right, .. } => {
                left.renumber(stamps);
                right.renumber(stamps);
            }
        }
    }
    fn visit(&mut self, f: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Leaf(v, ..) => {
                for r in v {
                    if r.stamp != 0 {
                        f(&mut r.value)
                    }
                }
            }
            Self::Branch { left, right, .. } => {
                left.visit(f);
                right.visit(f);
            }
        }
    }
    fn next_different(&self, base: usize, at: usize, value: Option<Value>) -> Option<usize> {
        if base + self.len() <= at {
            return None;
        }
        match self {
            Self::Leaf(runs, ..) => {
                let mut p = base;
                for r in runs {
                    let end = p + r.len as usize;
                    if end > at && (r.stamp != 0).then_some(r.value) != value {
                        return Some(p.max(at));
                    }
                    p = end;
                }
                None
            }
            Self::Branch { left, right, .. } => left
                .next_different(base, at, value)
                .or_else(|| right.next_different(base + left.len(), at, value)),
        }
    }
    fn storage(&self) -> usize {
        size_of::<Self>()
            + match self {
                Self::Leaf(v, ..) => v.capacity() * size_of::<Run>(),
                Self::Branch { left, right, .. } => left.storage() + right.storage(),
            }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Properties {
    layers: BTreeMap<String, Link>,
    stamp: u32,
}
impl Properties {
    pub fn properties_keys(&self) -> Vec<String> {
        self.layers.keys().cloned().collect()
    }
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn same_run_state(&self, a: ByteOffset, b: ByteOffset) -> bool {
        self.layers.values().flatten().all(|root| {
            let state = |at| {
                root.at(at)
                    .filter(|r| r.stamp != 0)
                    .map(|r| (r.stamp, r.value))
            };
            state(a.0) == state(b.0)
        })
    }
    pub fn get_property(&self, at: ByteOffset, key: &str) -> Option<Value> {
        self.layers
            .get(key)?
            .as_ref()?
            .at(at.0)
            .filter(|r| r.stamp != 0)
            .map(|r| r.value)
    }
    /// Replacement preserves the insertion stamp at already-present positions.
    pub fn put(&mut self, range: Range<ByteOffset>, key: &str, value: Value) {
        if range.start >= range.end {
            return;
        }
        if self.stamp == u32::MAX {
            // Dense rank compression preserves all ordering and merge decisions;
            // dead stamps must not impose a lifetime limit on an editor buffer.
            let mut stamps = BTreeMap::new();
            for tree in self.layers.values().flatten() {
                tree.walk(0, &mut |_, r| {
                    if r.stamp != 0 {
                        stamps.insert(r.stamp, 0);
                    }
                });
            }
            self.stamp = u32::try_from(stamps.len()).expect("too many live annotation stamps");
            for (index, rank) in stamps.values_mut().enumerate() {
                *rank = index as u32 + 1;
            }
            for tree in self.layers.values_mut().flatten() {
                tree.renumber(&stamps);
            }
        }
        self.stamp = self
            .stamp
            .checked_add(1)
            .expect("too many live annotation stamps");
        let stamp = self.stamp;
        let root = self.layers.entry(key.into()).or_default();
        let mut work = 0;
        let len = root.as_ref().map_or(0, |n| n.len());
        if len < range.end.0 {
            *root = Tree::concat(
                root.take(),
                Tree::uniform(range.end.0 - len, 0, Value::NIL, &mut work),
                &mut work,
            );
        }
        let (a, b) = Tree::split(root.take(), range.start.0, &mut work);
        let (b, c) = Tree::split(b, range.end.0 - range.start.0, &mut work);
        let mut replacement = None;
        b.unwrap().walk(0, &mut |_, r| {
            replacement = Tree::concat(
                replacement.take(),
                Tree::uniform(
                    r.len as usize,
                    if r.stamp == 0 { stamp } else { r.stamp },
                    value,
                    &mut work,
                ),
                &mut work,
            );
        });
        *root = Tree::concat(Tree::concat(a, replacement, &mut work), c, &mut work);
    }
    pub fn remove(&mut self, range: Range<ByteOffset>, key: &str) {
        let Some(root) = self.layers.get_mut(key) else {
            return;
        };
        let mut work = 0;
        let len = root.as_ref().map_or(0, |n| n.len());
        let start = range.start.0.min(len);
        let end = range.end.0.min(len);
        if start >= end {
            return;
        }
        let (a, b) = Tree::split(root.take(), start, &mut work);
        let (_, c) = Tree::split(b, end - start, &mut work);
        *root = Tree::concat(
            Tree::concat(
                a,
                Tree::uniform(end - start, 0, Value::NIL, &mut work),
                &mut work,
            ),
            c,
            &mut work,
        );
        if root.as_ref().is_none_or(|n| n.present() == 0) {
            self.layers.remove(key);
        }
    }
    pub fn properties_at(&self, at: ByteOffset) -> Vec<(&str, Value)> {
        let mut values: Vec<_> = self
            .layers
            .iter()
            .filter_map(|(key, root)| {
                let r = root.as_ref()?.at(at.0)?;
                (r.stamp != 0).then_some((r.stamp, key.as_str(), r.value))
            })
            .collect();
        values.sort_unstable_by_key(|(stamp, _, _)| std::cmp::Reverse(*stamp));
        values.into_iter().map(|(_, k, v)| (k, v)).collect()
    }
    /// Observable boundaries compare presence and eq-values, never stamps.
    pub fn next_change(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        let Some(after) = at.0.checked_add(1) else {
            return limit;
        };
        let mut next = limit.0;
        for root in self.layers.values().flatten() {
            let before = root.at(at.0).filter(|r| r.stamp != 0).map(|r| r.value);
            if let Some(p) = root.next_different(0, after, before) {
                next = next.min(p);
            } else if before.is_some() && root.len() >= after {
                next = next.min(root.len());
            }
        }
        ByteOffset(next)
    }
    /// Plain insertion inherits nothing. Suffix boundaries move by sums.
    pub fn replace(&mut self, range: Range<ByteOffset>, inserted: usize) -> usize {
        let mut work = 0;
        self.layers.retain(|_, root| {
            let len = root.as_ref().map_or(0, |n| n.len());
            if range.start.0 < len {
                if range.end.0 <= len
                    && root.as_mut().unwrap().edit_local(
                        range.start.0,
                        range.end.0,
                        inserted,
                        &mut work,
                    )
                {
                    return root.as_ref().unwrap().present() != 0;
                }
                let (a, b) = Tree::split(root.take(), range.start.0, &mut work);
                let (_, c) = Tree::split(b, range.end.0.min(len) - range.start.0, &mut work);
                *root = Tree::concat(
                    Tree::concat(
                        a,
                        Tree::uniform(inserted, 0, Value::NIL, &mut work),
                        &mut work,
                    ),
                    c,
                    &mut work,
                );
            }
            root.as_ref().is_some_and(|n| n.present() != 0)
        });
        work
    }
    /// Legacy adapter, outside hot paths: expands runs into a byte map.
    pub fn to_map(&self) -> BTreeMap<ByteOffset, Values> {
        let mut out: BTreeMap<ByteOffset, Values> = BTreeMap::new();
        for (key, root) in &self.layers {
            if let Some(root) = root {
                root.walk(0, &mut |at, r| {
                    if r.stamp != 0 {
                        for p in at..at + r.len as usize {
                            out.entry(ByteOffset(p))
                                .or_default()
                                .insert(key.clone(), r.value);
                        }
                    }
                });
            }
        }
        out
    }
    /// Legacy one-byte fixture adapter. Production writes use whole ranges.
    pub fn insert(&mut self, at: ByteOffset, values: Values) -> Option<Values> {
        let old: Values = self
            .properties_at(at)
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect();
        for key in old.keys() {
            if !values.contains_key(key) {
                self.remove(at..ByteOffset(at.0 + 1), key);
            }
        }
        for (k, v) in values {
            self.put(at..ByteOffset(at.0 + 1), &k, v);
        }
        (!old.is_empty()).then_some(old)
    }
    pub fn from_map(map: &BTreeMap<ByteOffset, Values>) -> Self {
        let mut out = Self::new();
        for (at, values) in map {
            out.insert(*at, values.clone());
        }
        out
    }
    /// Character replacement changes UTF-8 width, not that character's properties.
    pub fn resize_character(&mut self, range: Range<ByteOffset>, inserted: usize) {
        let mut work = 0;
        for root in self.layers.values_mut() {
            let len = root.as_ref().map_or(0, |n| n.len());
            if range.start.0 >= len {
                continue;
            }
            let run = root.as_ref().unwrap().at(range.start.0).unwrap();
            let (a, b) = Tree::split(root.take(), range.start.0, &mut work);
            let (_, c) = Tree::split(b, range.end.0.min(len) - range.start.0, &mut work);
            *root = Tree::concat(
                Tree::concat(
                    a,
                    Tree::uniform(inserted, run.stamp, run.value, &mut work),
                    &mut work,
                ),
                c,
                &mut work,
            );
        }
    }
    /// GNU insert-and-inherit keeps inherited keys (including nil) and prepends
    /// missing string keys in plist traversal order.
    pub fn inherit_inserted(&mut self, range: Range<ByteOffset>, inherited: &[(String, Value)]) {
        if range.is_empty() || inherited.is_empty() {
            return;
        }
        let mut source = self.slice(range.clone());
        for (key, _) in inherited {
            source.remove(ByteOffset(0)..ByteOffset(range.end.0 - range.start.0), key);
        }
        let mut merged = Self::new();
        for (key, value) in inherited.iter().rev() {
            merged.put(
                ByteOffset(0)..ByteOffset(range.end.0 - range.start.0),
                key,
                *value,
            );
        }
        merged.copy_in_order(ByteOffset(0), &source, true);
        let keys: Vec<_> = self.layers.keys().cloned().collect();
        for key in keys {
            self.remove(range.clone(), &key);
        }
        self.copy_into(range.start, &merged);
    }
    /// Copy a relative subtree range without expanding runs into characters.
    pub fn slice(&self, range: Range<ByteOffset>) -> Self {
        let mut out = Self::new();
        out.stamp = self.stamp;
        for (key, root) in &self.layers {
            let len = root.as_ref().map_or(0, |n| n.len());
            if range.start.0 >= len {
                continue;
            }
            let mut work = 0;
            let (_, suffix) = Tree::split(root.clone(), range.start.0, &mut work);
            let (part, _) = Tree::split(suffix, range.end.0.min(len) - range.start.0, &mut work);
            if part.as_ref().is_some_and(|n| n.present() != 0) {
                out.layers.insert(key.clone(), part);
            }
        }
        out
    }
    /// Graft present runs into an already inserted span. Sorted source stamps
    /// preserve per-position key order; Values are copied by identity.
    pub fn copy_into(&mut self, at: ByteOffset, source: &Self) {
        self.copy_in_order(at, source, false);
    }
    fn copy_in_order(&mut self, at: ByteOffset, source: &Self, reverse: bool) {
        let mut runs = vec![];
        for (key, root) in &source.layers {
            if let Some(root) = root {
                root.walk(0, &mut |start, r| {
                    if r.stamp != 0 {
                        runs.push((r.stamp, key.as_str(), start, r.len as usize, r.value));
                    }
                });
            }
        }
        runs.sort_unstable_by_key(|(stamp, _, start, _, _)| (*stamp, *start));
        if reverse {
            runs.reverse();
        }
        for (_, key, start, len, value) in runs {
            self.put(
                ByteOffset(at.0 + start)..ByteOffset(at.0 + start + len),
                key,
                value,
            );
        }
    }
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for tree in self.layers.values_mut().flatten() {
            tree.visit(visit);
        }
    }
    pub fn storage_bytes(&self) -> usize {
        // Conservative BTree node reservation: up to one 11-slot node per key.
        self.layers
            .iter()
            .map(|(k, root)| {
                k.capacity()
                    + 11 * (size_of::<String>() + size_of::<Link>())
                    + 12 * size_of::<usize>()
                    + root.as_ref().map_or(0, |n| n.storage())
            })
            .sum()
    }
}
