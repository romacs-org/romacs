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
romacs_snapshot::snapshot_struct!(Run { len, stamp, value });
impl Run {
    fn same(self, other: Self) -> bool {
        self.stamp == other.stamp && self.value == other.value
    }
    /// What a reader sees: its value when present, whatever its stamp.
    fn observed(self) -> Option<Value> {
        (self.stamp != 0).then_some(self.value)
    }
}
/// A leaf's runs, length and present length, and whether every run reads
/// the same (`uniform`, presence and value alike, stamps aside): a search
/// for the next change passes over a uniform subtree whole.
#[derive(Clone)]
enum Tree {
    Leaf(Vec<Run>, usize, usize, bool),
    Branch {
        left: Box<Tree>,
        right: Box<Tree>,
        len: usize,
        present: usize,
        height: u32,
        uniform: bool,
    },
}
romacs_snapshot::snapshot_enum!(Tree {
    Tree::Leaf(f0, f1, f2, f3) => (f0, f1, f2, f3),
    Tree::Branch { left, right, len, present, height, uniform } => (left, right, len, present, height, uniform),
});
impl Tree {
    fn leaf(runs: Vec<Run>) -> Self {
        let len = runs.iter().map(|r| r.len as usize).sum();
        let present = runs
            .iter()
            .filter(|r| r.stamp != 0)
            .map(|r| r.len as usize)
            .sum();
        let uniform = Self::runs_uniform(&runs);
        Self::Leaf(runs, len, present, uniform)
    }
    fn runs_uniform(runs: &[Run]) -> bool {
        runs.windows(2).all(|w| w[0].observed() == w[1].observed())
    }
    fn is_uniform(&self) -> bool {
        match self {
            Self::Leaf(.., uniform) => *uniform,
            Self::Branch { uniform, .. } => *uniform,
        }
    }
    /// Whether LEFT then RIGHT read the same throughout.
    fn joined_uniform(left: &Self, right: &Self) -> bool {
        left.is_uniform() && right.is_uniform() && left.edge(true).observed() == right.edge(false).observed()
    }
    fn len(&self) -> usize {
        match self {
            Self::Leaf(_, len, ..) => *len,
            Self::Branch { len, .. } => *len,
        }
    }
    fn present(&self) -> usize {
        match self {
            Self::Leaf(_, _, present, _) => *present,
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
            uniform: Self::joined_uniform(&left, &right),
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
            Self::Leaf(runs, len, present, uniform) => {
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
                *uniform = Self::runs_uniform(runs);
                true
            }
            Self::Branch {
                left,
                right,
                len,
                present,
                uniform,
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
                    *uniform = Self::joined_uniform(left, right);
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
    /// The first position after `at` whose value differs from `at`'s, or
    /// `limit` when that is nearer.
    fn change_after(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        let Some(after) = at.0.checked_add(1) else {
            return limit;
        };
        let before = self.at(at.0).filter(|r| r.stamp != 0).map(|r| r.value);
        match self.next_different(0, after, before, limit.0) {
            Some(p) => ByteOffset(p.min(limit.0)),
            None if before.is_some() && self.len() >= after => ByteOffset(self.len().min(limit.0)),
            None => limit,
        }
    }
    /// The first position from `at`, before `limit`, whose run does not
    /// read `value`. A uniform subtree that reads it is passed whole, so a
    /// search costs the tree's height, not the runs it passes.
    fn next_different(&self, base: usize, at: usize, value: Option<Value>, limit: usize) -> Option<usize> {
        if base + self.len() <= at || base >= limit {
            return None;
        }
        if self.is_uniform() && self.edge(false).observed() == value {
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
                .next_different(base, at, value, limit)
                .or_else(|| right.next_different(base + left.len(), at, value, limit)),
        }
    }
    /// The last position before `at` whose preceding value differs from
    /// the value just before `at`, or `limit` when that is nearer.
    fn change_before(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        if at.0 == 0 {
            return limit;
        }
        let before = self.at(at.0 - 1).filter(|r| r.stamp != 0).map(|r| r.value);
        ByteOffset(self.last_different(0, at.0, before, limit.0).unwrap_or(0).max(limit.0))
    }
    /// The end of the last run starting before `at` whose value is not
    /// `value`.
    fn last_different(&self, base: usize, at: usize, value: Option<Value>, limit: usize) -> Option<usize> {
        if base >= at || base + self.len() <= limit {
            return None;
        }
        if self.is_uniform() && self.edge(true).observed() == value {
            return None;
        }
        match self {
            Self::Leaf(runs, ..) => {
                let mut end = base + self.len();
                for r in runs.iter().rev() {
                    let start = end - r.len as usize;
                    if start < at && (r.stamp != 0).then_some(r.value) != value {
                        return Some(end.min(at));
                    }
                    end = start;
                }
                None
            }
            Self::Branch { left, right, .. } => right
                .last_different(base + left.len(), at, value, limit)
                .or_else(|| left.last_different(base, at, value, limit)),
        }
    }
    /// A copy of START..END of the runs, visiting only the subtrees
    /// inside it.
    fn copy_range(&self, start: usize, end: usize, work: &mut usize) -> Link {
        let end = end.min(self.len());
        if start >= end {
            return None;
        }
        *work += 1;
        if start == 0 && end == self.len() {
            return Some(Box::new(self.clone()));
        }
        match self {
            Self::Leaf(runs, ..) => {
                let mut p = 0;
                let mut out = Vec::new();
                for r in runs {
                    let (a, b) = (p.max(start), (p + r.len as usize).min(end));
                    if a < b {
                        out.push(Run { len: (b - a) as u32, ..*r });
                    }
                    p += r.len as usize;
                }
                Some(Box::new(Self::leaf(out)))
            }
            Self::Branch { left, right, .. } => {
                let n = left.len();
                let a = left.copy_range(start, end.min(n), work);
                let b = right.copy_range(start.saturating_sub(n), end.saturating_sub(n), work);
                Self::join(a, b, work)
            }
        }
    }
    /// The run holding `at` and its start.
    fn run_at(&self, at: usize) -> Option<(usize, Run)> {
        let mut base = 0;
        let mut node = self;
        loop {
            match node {
                Self::Leaf(v, ..) => {
                    for r in v {
                        if at < base + r.len as usize {
                            return Some((base, *r));
                        }
                        base += r.len as usize;
                    }
                    return None;
                }
                Self::Branch { left, right, .. } => {
                    if at < base + left.len() {
                        node = left;
                    } else {
                        base += left.len();
                        node = right;
                    }
                }
            }
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

/// One key's values for many reads: [`Properties::get_property`] without
/// the key's lookup, remembering the run it read last.
pub(crate) struct KeyReader<'a> {
    root: Option<&'a Tree>,
    /// The last run read: its start, end and value.
    run: std::cell::Cell<(usize, usize, Option<Value>)>,
}
impl KeyReader<'_> {
    pub fn get(&self, at: ByteOffset) -> Option<Value> {
        let (start, end, value) = self.run.get();
        if start <= at.0 && at.0 < end {
            return value;
        }
        let (start, run) = self.root?.run_at(at.0)?;
        let value = (run.stamp != 0).then_some(run.value);
        self.run.set((start, start + run.len as usize, value));
        value
    }
    /// The end of a stretch from AT over which the key has no value: AT
    /// itself when it has one there, `usize::MAX` when none follows the
    /// key's last run.  The stretch may end before the next value.
    pub fn unset_until(&self, at: ByteOffset) -> usize {
        if self.get(at).is_some() {
            return at.0;
        }
        let (start, end, _) = self.run.get();
        if start <= at.0 && at.0 < end { end } else { usize::MAX }
    }
}

/// The runs of a range seen through a few keys ([`Properties::key_runs`]):
/// each run's bytes and the values of those keys its characters have.
pub(crate) struct KeyRuns<'a> {
    layers: Vec<(&'a str, &'a Tree)>,
    at: ByteOffset,
    end: ByteOffset,
}
impl<'a> Iterator for KeyRuns<'a> {
    type Item = (Range<ByteOffset>, Vec<(&'a str, Value)>);
    fn next(&mut self) -> Option<Self::Item> {
        let at = self.at;
        if at >= self.end {
            return None;
        }
        let values = self.layers.iter().filter_map(|(key, root)| root.at(at.0).filter(|r| r.stamp != 0).map(|r| (*key, r.value))).collect();
        self.at = self.layers.iter().fold(self.end, |next, (_, root)| next.min(root.change_after(at, next)));
        Some((at..self.at, values))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Properties {
    layers: BTreeMap<String, Link>,
    stamp: u32,
    /// Where `syntax-table' changed since the text's parse-state summaries
    /// last took the changes (`crate::syntax_index`). Boxed, and made by the
    /// first such change, so that a string, which never has summaries,
    /// pays one word.
    syntax_changes: Option<Box<SyntaxChanges>>,
}
romacs_snapshot::snapshot_struct!(Properties { layers, stamp, syntax_changes });

/// Byte ranges where the `syntax-table' property changed, as the text's
/// bytes lie until its next edit: apart while they are few, then as their
/// hull (lowest start to highest end), one range that each later change
/// widens. A count never makes it `all', which is every property replaced.
#[derive(Clone, Debug, Default)]
pub(crate) struct SyntaxChanges {
    pub ranges: Vec<Range<usize>>,
    pub all: bool,
    /// Whether the ranges are one hull.
    pub hull: bool,
}
romacs_snapshot::snapshot_struct!(SyntaxChanges { ranges, all, hull });
impl SyntaxChanges {
    /// The ranges kept apart.
    const KEPT: usize = 64;
    /// Note RANGE in SLOT, making the log there if it has none. A range
    /// that starts where the last one ends continues it.
    fn note(slot: &mut Option<Box<Self>>, range: Range<usize>) {
        let Self { ranges, all: false, hull } = &mut **slot.get_or_insert_default() else { return };
        let join = |a: &Range<usize>, b: &Range<usize>| a.start.min(b.start)..a.end.max(b.end);
        match ranges.last_mut() {
            Some(last) if *hull => *last = join(last, &range),
            Some(last) if last.end == range.start => last.end = range.end,
            _ => ranges.push(range),
        }
        if ranges.len() > Self::KEPT {
            *ranges = ranges.iter().cloned().reduce(|a, b| join(&a, &b)).into_iter().collect();
            *hull = true;
        }
    }
}

impl Properties {
    /// Whether the log holds a change.
    pub(crate) fn has_syntax_changes(&self) -> bool {
        self.syntax_changes.is_some()
    }
    /// The log since the last take.
    pub(crate) fn take_syntax_changes(&mut self) -> SyntaxChanges {
        self.syntax_changes.take().map(|changes| *changes).unwrap_or_default()
    }
    /// No properties at all, as a new store, the change noted.
    pub(crate) fn reset(&mut self) {
        let changes = if self.layers.contains_key("syntax-table") {
            Some(Box::new(SyntaxChanges { all: true, ..SyntaxChanges::default() }))
        } else {
            self.syntax_changes.take()
        };
        *self = Self { syntax_changes: changes, ..Self::new() };
    }
    pub fn properties_keys(&self) -> Vec<String> {
        self.layers.keys().cloned().collect()
    }
    /// The runs of equal properties inside RANGE, in order, each with its
    /// properties: a snapshot that costs RANGE's runs, not the whole text.
    pub fn runs(&self, range: Range<ByteOffset>) -> Vec<(Range<ByteOffset>, Vec<(String, Value)>)> {
        let mut runs = Vec::new();
        let mut at = range.start;
        while at < range.end {
            let next = self.next_change(at, range.end);
            runs.push((at..next, self.properties_at(at).into_iter().map(|(k, v)| (k.to_owned(), v)).collect()));
            at = next;
        }
        runs
    }
    /// The runs inside RANGE seen through KEYS alone, each with its start:
    /// a run ends only where one of KEYS changes, and holds those of KEYS
    /// its characters have.  Costs the runs of KEYS, not of every property.
    pub fn runs_of(&self, range: Range<ByteOffset>, keys: &[String]) -> Vec<(ByteOffset, Vec<(String, Value)>)> {
        self.key_runs(range, keys.iter().map(String::as_str))
            .map(|(run, values)| (run.start, values.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()))
            .collect()
    }
    /// [`Self::runs_of`], one run at a time: a caller that stops early pays
    /// for the runs it read.
    pub fn key_runs<'a, 'k>(&'a self, range: Range<ByteOffset>, keys: impl IntoIterator<Item = &'k str>) -> KeyRuns<'a> {
        let layers = keys
            .into_iter()
            .filter_map(|key| self.layers.get_key_value(key))
            .filter_map(|(key, root)| Some((key.as_str(), root.as_deref()?)))
            .collect();
        KeyRuns { layers, at: range.start, end: range.end }
    }
    /// Whether any text has the property KEY.
    pub fn has_key(&self, key: &str) -> bool {
        self.layers.contains_key(key)
    }
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
    /// GNU's text has an interval tree once any property was ever put on
    /// it, even after every property is removed again.
    pub fn has_intervals(&self) -> bool {
        self.stamp != 0 || !self.layers.is_empty()
    }
    /// Note that the text has had properties (a unibyte buffer's map).
    pub fn mark_intervals(&mut self) {
        self.stamp = self.stamp.max(1);
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
    /// A reader of KEY's values.
    pub fn reader(&self, key: &str) -> KeyReader<'_> {
        KeyReader { root: self.layers.get(key).and_then(|root| root.as_deref()), run: std::cell::Cell::new((0, 0, None)) }
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
    /// A `syntax-table' put is a change where a run had another value (as
    /// `eq') or none.
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
        let logged = key == "syntax-table";
        b.unwrap().walk(range.start.0, &mut |at, r| {
            if logged && r.observed() != Some(value) {
                SyntaxChanges::note(&mut self.syntax_changes, at..at + r.len as usize);
            }
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
    /// A `syntax-table` remove is a change where a run held a value.
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
        let (b, c) = Tree::split(b, end - start, &mut work);
        if key == "syntax-table" && let Some(b) = &b {
            b.walk(start, &mut |at, r| {
                if r.stamp != 0 {
                    SyntaxChanges::note(&mut self.syntax_changes, at..at + r.len as usize);
                }
            });
        }
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
    /// Each key's search stops at the nearest change found so far.
    pub fn next_change(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        self.layers.values().flatten().fold(limit, |next, root| next.min(root.change_after(at, next)))
    }
    /// The last boundary before `at` where any property changes, or
    /// `limit` when that is nearer: [`Self::next_change`] backwards.
    pub fn previous_change(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        self.layers.values().flatten().fold(limit, |previous, root| previous.max(root.change_before(at, previous)))
    }
    /// [`Self::next_change`] of `key` alone.
    pub fn next_single_change(&self, at: ByteOffset, key: &str, limit: ByteOffset) -> ByteOffset {
        self.layers.get(key).and_then(Option::as_ref).map_or(limit, |root| root.change_after(at, limit))
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
            let part = root.as_ref().and_then(|n| n.copy_range(range.start.0, range.end.0, &mut work));
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
            .sum::<usize>()
            + self.syntax_changes.as_ref().map_or(0, |changes| {
                size_of::<SyntaxChanges>() + changes.ranges.capacity() * size_of::<Range<usize>>()
            })
    }
}

/// What a property primitive's plan asks of the properties it changes: a
/// string's, or a text body's, which records what is stored.
pub(crate) trait Store {
    fn put(&mut self, range: Range<ByteOffset>, key: &str, value: Value);
    fn remove(&mut self, range: Range<ByteOffset>, key: &str);
    fn reset(&mut self);
    fn keys(&self) -> Vec<String>;
}
impl Store for Properties {
    fn put(&mut self, range: Range<ByteOffset>, key: &str, value: Value) { Properties::put(self, range, key, value); }
    fn remove(&mut self, range: Range<ByteOffset>, key: &str) { Properties::remove(self, range, key); }
    fn reset(&mut self) { Properties::reset(self); }
    fn keys(&self) -> Vec<String> { self.properties_keys() }
}
impl Store for BodyProperties {
    fn put(&mut self, range: Range<ByteOffset>, key: &str, value: Value) { BodyProperties::put(self, range, key, value); }
    fn remove(&mut self, range: Range<ByteOffset>, key: &str) { BodyProperties::remove(self, range, key); }
    fn reset(&mut self) { self.store.reset(); }
    fn keys(&self) -> Vec<String> { self.store.properties_keys() }
}

/// How a collection reads a text body's properties (`BodyProperties`).
#[derive(Clone, Copy, Default)]
pub(crate) struct Reading {
    /// The epoch the VM's last collection ended at (`crate::epoch`).
    pub(crate) since: u64,
    /// Every property is read, as a full collection, a compaction's rewrite
    /// and verify's reference walk do; else only what was stored since.
    pub(crate) whole: bool,
}

/// Values kept before a record gives up and the body is read whole.
const WRITTEN_LIMIT: usize = 64 << 10;

/// A text body's properties, with a record of what was stored into them.
/// A body is shared by a buffer and its indirect buffers and belongs to no
/// table entry alone, so no entry's colour says it was stored into: the
/// record is here, in the storage, and a minor collection reads it in place
/// of every property of every buffer. It hands out the properties to read
/// only (`Deref`), and each method that can store a Value records it, so a
/// store that records nothing does not compile. A string's properties need
/// none of this: its entry is dirtied.
///
/// The record is kept by epoch (`crate::epoch`), which any VM's collection
/// in the process advances: a store cannot tell whether its own VM has
/// collected since the last one, so it never drops what is recorded. The
/// collector, which knows the epoch its VM's last collection ended at,
/// reads what was recorded since and drops what was recorded before.
#[derive(Clone)]
pub(crate) struct BodyProperties {
    store: Properties,
    /// The Values stored that can name a heap entry, oldest first.
    written: Vec<Value>,
    /// The epochs `written` was recorded in, ascending, and where each one's
    /// Values begin.
    epochs: Vec<u64>,
    starts: Vec<usize>,
    /// The epoch at which everything last counted as stored: when the body
    /// was made, its properties replaced whole, or its record given up.
    all_at: u64,
}
romacs_snapshot::snapshot_struct!(BodyProperties { store, written, epochs, starts, all_at });

impl std::ops::Deref for BodyProperties {
    type Target = Properties;
    #[inline]
    fn deref(&self) -> &Properties {
        &self.store
    }
}

impl BodyProperties {
    /// STORE as a body's properties: all of it counts as just stored.
    pub(crate) fn new(store: Properties) -> Self {
        Self { store, written: Vec::new(), epochs: Vec::new(), starts: Vec::new(), all_at: crate::epoch::born() }
    }
    /// VALUES are about to be stored.
    fn record(&mut self, values: impl IntoIterator<Item = Value>) {
        let now = crate::epoch::born();
        // Everything counts as stored in this epoch already.
        if self.all_at == now { return; }
        if self.epochs.last() != Some(&now) {
            self.epochs.push(now);
            self.starts.push(self.written.len());
        }
        self.written.extend(values.into_iter().filter(|v| matches!(v.tag(), 0 | 3..=5) && *v != Value::NIL));
        if self.written.len() > WRITTEN_LIMIT {
            (self.written, self.epochs, self.starts, self.all_at) = (Vec::new(), Vec::new(), Vec::new(), now);
        }
    }
    pub fn put(&mut self, range: Range<ByteOffset>, key: &str, value: Value) {
        self.record([value]);
        self.store.put(range, key, value);
    }
    pub fn inherit_inserted(&mut self, range: Range<ByteOffset>, inherited: &[(String, Value)]) {
        self.record(inherited.iter().map(|&(_, value)| value));
        self.store.inherit_inserted(range, inherited);
    }
    pub fn copy_into(&mut self, at: ByteOffset, source: &Properties) {
        self.record(source.to_map().into_values().flat_map(BTreeMap::into_values));
        self.store.copy_into(at, source);
    }
    pub fn remove(&mut self, range: Range<ByteOffset>, key: &str) {
        self.store.remove(range, key);
    }
    pub fn replace(&mut self, range: Range<ByteOffset>, inserted: usize) -> usize {
        self.store.replace(range, inserted)
    }
    pub fn resize_character(&mut self, range: Range<ByteOffset>, inserted: usize) {
        self.store.resize_character(range, inserted);
    }
    pub(crate) fn take_syntax_changes(&mut self) -> SyntaxChanges {
        self.store.take_syntax_changes()
    }
    /// The collector's visit, as READING says. What was recorded before the
    /// VM's last collection is dropped: that collection kept it. What was
    /// recorded since is visited, by a whole reading too: a compaction
    /// rewrites those Values, and one removed since it was stored is kept
    /// until the next collection. Nothing recorded since is dropped here:
    /// a mark a key press abandons has consumed nothing.
    pub(crate) fn visit_values(&mut self, reading: Reading, visit: &mut impl FnMut(&mut Value)) {
        let stale = self.epochs.partition_point(|&epoch| epoch < reading.since);
        if stale > 0 {
            let from = self.starts.get(stale).copied().unwrap_or(self.written.len());
            self.written.drain(..from);
            self.epochs.drain(..stale);
            self.starts.drain(..stale);
            self.starts.iter_mut().for_each(|start| *start -= from);
        }
        self.written.iter_mut().for_each(&mut *visit);
        if reading.whole || self.all_at >= reading.since {
            self.store.visit_values(visit);
        }
    }
}
