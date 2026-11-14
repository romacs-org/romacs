//! Per-key annotation layers. Relative lengths move with text; no anchors.
//! Stamp zero is ABSENT, not Lisp nil. Value equality is identity, never `equal`.
//! A key has no implicit or visual baseline.
use crate::{Value, coordinates::ByteOffset};
use std::{collections::BTreeMap, ops::Range};
type Link = Option<Box<Tree>>;
const LEAF: usize = 32;

/// A property's key: its symbol, compared by identity as GNU's `EQ`
/// compares plist keys, so an uninterned symbol is its own key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key(Value);
romacs_snapshot::snapshot_tuple!(Key(f0));
impl Ord for Key {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.0.cmp(&other.0.0)
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Key {
    /// SYMBOL's key, or None when it is no symbol.
    pub(crate) fn of(symbol: Value) -> Option<Self> {
        (symbol == Value::NIL || symbol == Value::TRUE || (symbol.tag() == 7 && symbol != Value::UNBOUND)).then_some(Self(symbol))
    }
    /// The symbol this key is.
    pub(crate) fn symbol(self) -> Value {
        self.0
    }
    const fn builtin(index: usize) -> Self {
        Self(Value::reference(index + 1, 7))
    }
    /// A key for NAME where no `Vm` interns it: its built-in key, else the
    /// key of NAME's place among the names tests have asked for, past the
    /// built-ins, so that two names are always two keys.
    #[cfg(test)]
    pub(crate) fn test(name: &str) -> Self {
        static NAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        if let Some(index) = Self::BUILTIN.iter().position(|builtin| *builtin == name) {
            return Self::builtin(index);
        }
        let mut names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = names.iter().position(|known| known == name).unwrap_or_else(|| {
            names.push(name.to_owned());
            names.len() - 1
        });
        Self::builtin(Self::BUILTIN.len() + index)
    }
}
/// The keys Rust names: `Vm::new` interns their symbols first and in this
/// order, so each is a constant, as GNU's `Qface` is.
macro_rules! builtin_keys {
    ($($constant:ident = $name:literal;)*) => {
        impl Key {
            /// The built-in keys' symbol names, each at its symbol's slot.
            pub(crate) const BUILTIN: &'static [&'static str] = &[$($name),*];
            builtin_keys!(@at 0; $($constant)*);
        }
    };
    (@at $index:expr; $constant:ident $($rest:ident)*) => {
        pub(crate) const $constant: Self = Self::builtin($index);
        builtin_keys!(@at $index + 1; $($rest)*);
    };
    (@at $index:expr;) => {};
}
builtin_keys! {
    FACE = "face";
    CATEGORY = "category";
    SYNTAX_TABLE = "syntax-table";
    READ_ONLY = "read-only";
    INHIBIT_READ_ONLY = "inhibit-read-only";
    MODIFICATION_HOOKS = "modification-hooks";
    INSERT_BEHIND_HOOKS = "insert-behind-hooks";
    INSERT_IN_FRONT_HOOKS = "insert-in-front-hooks";
    FRONT_STICKY = "front-sticky";
    REAR_NONSTICKY = "rear-nonsticky";
    MODE_LINE_ELT_NO = "mode-line-elt-no";
}
#[cfg(test)]
#[path = "properties_tests.rs"]
mod tests;
mod body;
mod values;
pub(crate) use body::{BodyProperties, Reading, Store};
pub(crate) use values::Values;

/// PAIRS of a character, each with its write's stamp, as a plist reads
/// them: the newest first.
fn newest_first(mut pairs: Vec<(u32, Key, Value)>) -> Vec<(Key, Value)> {
    pairs.sort_unstable_by_key(|(stamp, _, _)| std::cmp::Reverse(*stamp));
    pairs.into_iter().map(|(_, k, v)| (k, v)).collect()
}

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
    /// LEN bytes without the key.
    const fn absent(len: u32) -> Self {
        Self { len, stamp: 0, value: Value::NIL }
    }
    /// Add RUN to RUNS, lengthening the last run when RUN continues it.
    fn append(runs: &mut [Self], count: &mut usize, run: Self) {
        if run.len == 0 {
            return;
        }
        match count.checked_sub(1).map(|last| &mut runs[last]) {
            Some(last) if last.same(run) && last.len.checked_add(run.len).is_some() => last.len += run.len,
            _ => {
                runs[*count] = run;
                *count += 1;
            }
        }
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
    fn node(left: Box<Self>, right: Box<Self>) -> Self {
        Self::Branch {
            len: left.len() + right.len(),
            present: left.present() + right.present(),
            height: 1 + left.height().max(right.height()),
            uniform: Self::joined_uniform(&left, &right),
            left,
            right,
        }
    }
    fn branch(left: Box<Self>, right: Box<Self>) -> Box<Self> {
        Box::new(Self::node(left, right))
    }
    /// An overfull leaf's RUNS as two leaves, the first of AT runs, each
    /// holding no more than its runs.
    fn halves(runs: &[Run], at: usize) -> Self {
        let (left, right) = runs.split_at(at);
        Self::node(Box::new(Self::leaf(left.to_vec())), Box::new(Self::leaf(right.to_vec())))
    }
    /// A branch after an edit below it: whether its runs read the same
    /// throughout, and, where a child GREW taller, its height and balance.
    /// The edit's own descent has kept its length and present length.
    /// Whether the branch grew taller itself.
    fn settle(&mut self, grew: bool) -> bool {
        let Self::Branch { left, right, height, uniform, .. } = self else { return false };
        *uniform = Self::joined_uniform(left, right);
        if !grew {
            return false;
        }
        let before = *height;
        if left.height().abs_diff(right.height()) > 1 {
            let Self::Branch { left, right, .. } = std::mem::replace(self, Self::Leaf(Vec::new(), 0, 0, true)) else { unreachable!() };
            *self = *Self::balance(left, right);
        } else {
            *height = 1 + left.height().max(right.height());
        }
        self.height() != before
    }
    /// The runs of START..END through TO, where one leaf holds them all:
    /// the runs the range touches are rewritten in place, the leaf split in
    /// two when it overflows, and only its path is visited.  None when the
    /// range spans leaves; else how much more of the text has the key, and
    /// whether the tree grew taller here.
    fn rewrite_local(&mut self, start: usize, end: usize, to: &impl Fn(Run) -> Run) -> Option<(isize, bool)> {
        match self {
            Self::Leaf(runs, len, present, uniform) => {
                // The window: the runs the range touches and the one on
                // either side, which what is written may continue.
                let (mut first, mut at) = (0, 0);
                while at + runs[first].len as usize <= start {
                    at += runs[first].len as usize;
                    first += 1;
                }
                if first > 0 {
                    first -= 1;
                    at -= runs[first].len as usize;
                }
                let mut output = [Run::absent(0); LEAF + 2];
                let (mut count, mut last, mut was) = (0, first, 0);
                while last < runs.len() {
                    let r = runs[last];
                    let limit = at + r.len as usize;
                    if r.stamp != 0 {
                        was += r.len as isize;
                    }
                    // The run after the range ends the window.
                    let after = at >= end;
                    if limit <= start || after {
                        Run::append(&mut output, &mut count, r);
                    } else {
                        let (a, b) = (at.max(start), limit.min(end));
                        // Each piece is part of R, whose length is a u32.
                        let piece = |len: usize| u32::try_from(len).expect("part of a run");
                        Run::append(&mut output, &mut count, Run { len: piece(a - at), ..r });
                        Run::append(&mut output, &mut count, to(Run { len: piece(b - a), ..r }));
                        Run::append(&mut output, &mut count, Run { len: piece(limit - b), ..r });
                    }
                    (at, last) = (limit, last + 1);
                    if after {
                        break;
                    }
                }
                let now: isize = output[..count].iter().filter(|r| r.stamp != 0).map(|r| r.len as isize).sum();
                runs.splice(first..last, output[..count].iter().copied());
                *present = present.wrapping_add_signed(now - was);
                *uniform = Self::runs_uniform(runs);
                let grew = runs.len() > LEAF;
                if grew {
                    // A leaf written at its end is being filled in order:
                    // leave it full.
                    *self = Self::halves(runs, if end == *len { LEAF } else { runs.len() / 2 });
                }
                Some((now - was, grew))
            }
            Self::Branch { left, right, present, .. } => {
                let boundary = left.len();
                let (more, grew) = if end <= boundary {
                    left.rewrite_local(start, end, to)?
                } else if start >= boundary {
                    right.rewrite_local(start - boundary, end - boundary, to)?
                } else {
                    return None;
                };
                *present = present.wrapping_add_signed(more);
                Some((more, self.settle(grew)))
            }
        }
    }
    /// START..END's runs through TO, in one descent when a leaf holds the
    /// range, else by one split and join around it.
    fn rewrite(root: &mut Box<Self>, start: usize, end: usize, to: &impl Fn(Run) -> Run) {
        if root.rewrite_local(start, end, to).is_some() {
            return;
        }
        let mut work = 0;
        let whole = std::mem::replace(root, Box::new(Self::Leaf(Vec::new(), 0, 0, true)));
        let (a, b) = Self::split(Some(whole), start, &mut work);
        let (b, c) = Self::split(b, end - start, &mut work);
        let mut runs: Vec<Run> = Vec::new();
        b.expect("a range inside the tree").walk(0, &mut |_, r| {
            let r = to(r);
            match runs.last_mut() {
                Some(last) if last.same(r) && last.len.checked_add(r.len).is_some() => last.len += r.len,
                _ => runs.push(r),
            }
        });
        let middle = runs.chunks(LEAF).fold(None, |tree, chunk| Self::join(tree, Some(Box::new(Self::leaf(chunk.to_vec()))), &mut work));
        *root = Self::concat(Self::concat(a, middle, &mut work), c, &mut work).expect("a range inside the tree");
    }
    /// GAP without the key and then RUN at the tree's end, in its last leaf.
    /// Whether the tree grew taller.
    fn append_local(&mut self, gap: u32, run: Run) -> bool {
        let (longer, more) = (gap as usize + run.len as usize, if run.stamp == 0 { 0 } else { run.len as usize });
        match self {
            Self::Leaf(runs, len, present, uniform) => {
                for run in [Run::absent(gap), run] {
                    if run.len == 0 {
                        continue;
                    }
                    let longer = runs.last().filter(|last| last.same(run)).and_then(|last| last.len.checked_add(run.len));
                    match longer {
                        Some(len) => runs.last_mut().expect("a last run").len = len,
                        None => runs.push(run),
                    }
                }
                *len += longer;
                *present += more;
                *uniform = Self::runs_uniform(runs);
                let grew = runs.len() > LEAF;
                if grew {
                    *self = Self::halves(runs, LEAF);
                }
                grew
            }
            Self::Branch { right, len, present, .. } => {
                let grew = right.append_local(gap, run);
                *len += longer;
                *present += more;
                self.settle(grew)
            }
        }
    }
    /// GAP without the key and then LEN at STAMP and VALUE at the tree's
    /// end: one descent, unless a stretch is too long for one run.
    fn append(root: &mut Box<Self>, gap: usize, len: usize, stamp: u32, value: Value) {
        if let (Ok(gap), Ok(len)) = (u32::try_from(gap), u32::try_from(len)) {
            root.append_local(gap, Run { len, stamp, value });
            return;
        }
        let mut work = 0;
        let whole = std::mem::replace(root, Box::new(Self::Leaf(Vec::new(), 0, 0, true)));
        let grown = Self::concat(Some(whole), Self::uniform(gap, 0, Value::NIL, &mut work), &mut work);
        *root = Self::concat(grown, Self::uniform(len, stamp, value, &mut work), &mut work).expect("a tree");
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

/// One character's properties, read by key.
pub(crate) trait Plist {
    fn property(&self, key: Key) -> Option<Value>;
}
impl Plist for [(Key, Value)] {
    fn property(&self, key: Key) -> Option<Value> {
        self.iter().find(|(k, _)| *k == key).map(|(_, value)| *value)
    }
}
impl Plist for Vec<(Key, Value)> {
    fn property(&self, key: Key) -> Option<Value> {
        self.as_slice().property(key)
    }
}
/// The properties of the character at a byte of a text's store, each read
/// from its own key's runs: no list of them is made.
pub(crate) struct At<'a>(pub &'a Properties, pub ByteOffset);
impl Plist for At<'_> {
    fn property(&self, key: Key) -> Option<Value> {
        self.0.get_property(self.1, key)
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
    layers: Vec<(Key, &'a Tree)>,
    at: ByteOffset,
    end: ByteOffset,
}
impl Iterator for KeyRuns<'_> {
    type Item = (Range<ByteOffset>, Vec<(Key, Value)>);
    fn next(&mut self) -> Option<Self::Item> {
        let at = self.at;
        if at >= self.end {
            return None;
        }
        let values = self.layers.iter().filter_map(|(key, root)| root.at(at.0).and_then(Run::observed).map(|value| (*key, value))).collect();
        self.at = self.layers.iter().fold(self.end, |next, (_, root)| next.min(root.change_after(at, next)));
        Some((at..self.at, values))
    }
}

/// Every key of MAP, a symbol, through VISIT: a key keeps its symbol
/// alive.  The map is rebuilt in the rare case a key comes back another.
pub(crate) fn visit_keys<V>(map: &mut BTreeMap<Key, V>, visit: &mut impl FnMut(&mut Value)) {
    let mut moved = Vec::new();
    for &key in map.keys() {
        let mut symbol = key.0;
        visit(&mut symbol);
        if symbol != key.0 {
            moved.push((key, Key(symbol)));
        }
    }
    for (old, new) in moved {
        let value = map.remove(&old).expect("a key just read");
        map.insert(new, value);
    }
}

#[derive(Clone, Default)]
pub(crate) struct Properties {
    /// Each key's runs.  A key no text has has no layer.
    layers: BTreeMap<Key, Box<Tree>>,
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
    /// Note in SLOT the runs of ROOT's RANGE whose value CHANGED picks, a
    /// run past ROOT's end, or of no ROOT, having none.
    fn note_runs(slot: &mut Option<Box<Self>>, root: Option<&Tree>, range: Range<usize>, changed: impl Fn(Option<Value>) -> bool) {
        let mut at = range.start;
        while at < range.end {
            let (end, had) = match root.and_then(|root| root.run_at(at)) {
                Some((base, run)) => ((base + run.len as usize).min(range.end), run.observed()),
                None => (range.end, None),
            };
            if changed(had) {
                Self::note(slot, at..end);
            }
            at = end;
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
        let changes = if self.has_key(Key::SYNTAX_TABLE) {
            Some(Box::new(SyntaxChanges { all: true, ..SyntaxChanges::default() }))
        } else {
            self.syntax_changes.take()
        };
        *self = Self { syntax_changes: changes, ..Self::new() };
    }
    fn tree(&self, key: Key) -> Option<&Tree> {
        self.layers.get(&key).map(|root| &**root)
    }
    fn trees(&self) -> impl Iterator<Item = &Tree> {
        self.layers.values().map(|root| &**root)
    }
    /// The keys some text has.
    pub fn keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.layers.keys().copied()
    }
    /// The runs of equal properties inside RANGE, in order, each with its
    /// properties: a snapshot that costs RANGE's runs, not the whole text.
    pub fn runs(&self, range: Range<ByteOffset>) -> Vec<(Range<ByteOffset>, Vec<(Key, Value)>)> {
        let mut runs = Vec::new();
        let mut at = range.start;
        while at < range.end {
            let next = self.next_change(at, range.end);
            runs.push((at..next, self.properties_at(at)));
            at = next;
        }
        runs
    }
    /// The runs inside RANGE seen through KEYS alone, each with its start:
    /// a run ends only where one of KEYS changes, and holds those of KEYS
    /// its characters have.  Costs the runs of KEYS, not of every property.
    pub fn runs_of(&self, range: Range<ByteOffset>, keys: &[Key]) -> Vec<(ByteOffset, Vec<(Key, Value)>)> {
        self.key_runs(range, keys.iter().copied()).map(|(run, values)| (run.start, values)).collect()
    }
    /// [`Self::runs_of`], one run at a time: a caller that stops early pays
    /// for the runs it read.
    pub fn key_runs(&self, range: Range<ByteOffset>, keys: impl IntoIterator<Item = Key>) -> KeyRuns<'_> {
        let layers = keys.into_iter().filter_map(|key| Some((key, self.tree(key)?))).collect();
        KeyRuns { layers, at: range.start, end: range.end }
    }
    /// Whether any text has the property KEY.
    pub fn has_key(&self, key: Key) -> bool {
        self.layers.contains_key(&key)
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
        self.trees().all(|root| {
            let state = |at| {
                root.at(at)
                    .filter(|r| r.stamp != 0)
                    .map(|r| (r.stamp, r.value))
            };
            state(a.0) == state(b.0)
        })
    }
    /// A reader of KEY's values.
    pub fn reader(&self, key: Key) -> KeyReader<'_> {
        KeyReader { root: self.tree(key), run: std::cell::Cell::new((0, 0, None)) }
    }
    pub fn get_property(&self, at: ByteOffset, key: Key) -> Option<Value> {
        self.tree(key)?.run_at(at.0).and_then(|(_, run)| run.observed())
    }
    /// The first position in RANGE where KEY does not read VALUE (None:
    /// where the text has KEY at all), found without visiting the stretches
    /// that read VALUE throughout.
    pub fn first_unlike(&self, range: Range<ByteOffset>, key: Key, value: Option<Value>) -> Option<ByteOffset> {
        if range.start >= range.end {
            return None;
        }
        let Some(root) = self.tree(key).filter(|root| range.start.0 < root.len()) else { return value.is_some().then_some(range.start) };
        root.next_different(0, range.start.0, value, range.end.0)
            .filter(|at| *at < range.end.0)
            // Past the layer's end no text has the key.
            .or_else(|| (value.is_some() && root.len() < range.end.0).then(|| root.len().max(range.start.0)))
            .map(ByteOffset)
    }
    /// The stamp of a new write.
    fn next_stamp(&mut self) -> u32 {
        if self.stamp == u32::MAX {
            // Dense rank compression preserves all ordering and merge decisions;
            // dead stamps must not impose a lifetime limit on an editor buffer.
            let mut stamps = BTreeMap::new();
            for tree in self.trees() {
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
            for root in self.layers.values_mut() {
                root.renumber(&stamps);
            }
        }
        self.stamp = self
            .stamp
            .checked_add(1)
            .expect("too many live annotation stamps");
        self.stamp
    }
    /// Replacement preserves the insertion stamp at already-present positions.
    /// A `syntax-table' put is a change where a run had another value (as
    /// `eq') or none.
    pub fn put(&mut self, range: Range<ByteOffset>, key: Key, value: Value) {
        if range.start >= range.end {
            return;
        }
        if key == Key::SYNTAX_TABLE {
            let root = self.layers.get(&key).map(|root| &**root);
            SyntaxChanges::note_runs(&mut self.syntax_changes, root, range.start.0..range.end.0, |had| had != Some(value));
        }
        let stamp = self.next_stamp();
        let (start, end) = (range.start.0, range.end.0);
        if let Some(root) = self.layers.get_mut(&key) {
            let len = root.len();
            if len <= start {
                return Tree::append(root, start - len, end - start, stamp, value);
            }
            if len < end {
                Tree::append(root, end - len, 0, 0, Value::NIL);
            }
            Tree::rewrite(root, start, end, &|r| Run { stamp: if r.stamp == 0 { stamp } else { r.stamp }, value, ..r });
        } else {
            let mut work = 0;
            let before = Tree::uniform(start, 0, Value::NIL, &mut work);
            let root = Tree::concat(before, Tree::uniform(end - start, stamp, value, &mut work), &mut work);
            self.layers.insert(key, root.expect("a range that is not empty"));
        }
    }
    /// A `syntax-table' remove is a change where a run held a value.
    pub fn remove(&mut self, range: Range<ByteOffset>, key: Key) {
        let Some(root) = self.layers.get_mut(&key) else {
            return;
        };
        let len = root.len();
        let start = range.start.0.min(len);
        let end = range.end.0.min(len);
        if start >= end {
            return;
        }
        if key == Key::SYNTAX_TABLE {
            SyntaxChanges::note_runs(&mut self.syntax_changes, Some(root), start..end, |had| had.is_some());
        }
        Tree::rewrite(root, start, end, &|r| Run::absent(r.len));
        if root.present() == 0 {
            self.layers.remove(&key);
        }
    }
    /// Every key removed from RANGE.
    pub fn clear(&mut self, range: Range<ByteOffset>) {
        for key in self.keys().collect::<Vec<_>>() {
            self.remove(range.clone(), key);
        }
    }
    pub fn properties_at(&self, at: ByteOffset) -> Vec<(Key, Value)> {
        newest_first(self.layers.iter().filter_map(|(key, root)| {
            let r = root.at(at.0)?;
            (r.stamp != 0).then_some((r.stamp, *key, r.value))
        }).collect())
    }
    /// Observable boundaries compare presence and eq-values, never stamps.
    /// Each key's search stops at the nearest change found so far.
    pub fn next_change(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        self.trees().fold(limit, |next, root| next.min(root.change_after(at, next)))
    }
    /// The last boundary before `at` where any property changes, or
    /// `limit` when that is nearer: [`Self::next_change`] backwards.
    pub fn previous_change(&self, at: ByteOffset, limit: ByteOffset) -> ByteOffset {
        self.trees().fold(limit, |previous, root| previous.max(root.change_before(at, previous)))
    }
    /// [`Self::next_change`] of `key` alone.
    pub fn next_single_change(&self, at: ByteOffset, key: Key, limit: ByteOffset) -> ByteOffset {
        self.tree(key).map_or(limit, |root| root.change_after(at, limit))
    }
    /// Plain insertion inherits nothing. Suffix boundaries move by sums.
    pub fn replace(&mut self, range: Range<ByteOffset>, inserted: usize) -> usize {
        let mut work = 0;
        self.layers.retain(|_, root| {
            let len = root.len();
            if range.start.0 < len {
                if range.end.0 <= len && root.edit_local(range.start.0, range.end.0, inserted, &mut work) {
                    return root.present() != 0;
                }
                let whole = std::mem::replace(root, Box::new(Tree::Leaf(Vec::new(), 0, 0, true)));
                let (a, b) = Tree::split(Some(whole), range.start.0, &mut work);
                let (_, c) = Tree::split(b, range.end.0.min(len) - range.start.0, &mut work);
                let edited = Tree::concat(
                    Tree::concat(
                        a,
                        Tree::uniform(inserted, 0, Value::NIL, &mut work),
                        &mut work,
                    ),
                    c,
                    &mut work,
                );
                match edited {
                    Some(edited) => *root = edited,
                    None => return false,
                }
            }
            root.present() != 0
        });
        work
    }
    /// Legacy adapter, outside hot paths: expands runs into a byte map,
    /// each byte's properties in their plist order.
    pub fn to_map(&self) -> BTreeMap<ByteOffset, Values> {
        let mut stamped: BTreeMap<ByteOffset, Vec<(u32, Key, Value)>> = BTreeMap::new();
        for (key, root) in &self.layers {
            root.walk(0, &mut |at, r| {
                if r.stamp != 0 {
                    for p in at..at + r.len as usize {
                        stamped.entry(ByteOffset(p)).or_default().push((r.stamp, *key, r.value));
                    }
                }
            });
        }
        stamped.into_iter().map(|(at, pairs)| (at, newest_first(pairs).into_iter().collect())).collect()
    }
    /// Legacy one-byte fixture adapter. Production writes use whole ranges.
    /// The byte's properties become VALUES, in their order.
    pub fn insert(&mut self, at: ByteOffset, values: Values) -> Option<Values> {
        let old: Values = self.properties_at(at).into_iter().collect();
        for key in old.keys() {
            self.remove(at..ByteOffset(at.0 + 1), *key);
        }
        // The newest is written last.
        for (k, v) in values.into_iter().rev() {
            self.put(at..ByteOffset(at.0 + 1), k, v);
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
            let len = root.len();
            if range.start.0 >= len {
                continue;
            }
            let run = root.at(range.start.0).unwrap();
            let whole = std::mem::replace(root, Box::new(Tree::Leaf(Vec::new(), 0, 0, true)));
            let (a, b) = Tree::split(Some(whole), range.start.0, &mut work);
            let (_, c) = Tree::split(b, range.end.0.min(len) - range.start.0, &mut work);
            *root = Tree::concat(
                Tree::concat(
                    a,
                    Tree::uniform(inserted, run.stamp, run.value, &mut work),
                    &mut work,
                ),
                c,
                &mut work,
            )
            .expect("a character keeps a width");
        }
    }
    /// GNU insert-and-inherit keeps inherited keys (including nil) and prepends
    /// missing string keys in plist traversal order.
    pub fn inherit_inserted(&mut self, range: Range<ByteOffset>, inherited: &[(Key, Value)]) {
        if range.is_empty() || inherited.is_empty() {
            return;
        }
        let mut source = self.slice(range.clone());
        for (key, _) in inherited {
            source.remove(ByteOffset(0)..ByteOffset(range.end.0 - range.start.0), *key);
        }
        let mut merged = Self::new();
        for (key, value) in inherited.iter().rev() {
            merged.put(
                ByteOffset(0)..ByteOffset(range.end.0 - range.start.0),
                *key,
                *value,
            );
        }
        merged.copy_in_order(ByteOffset(0), &source, true);
        self.clear(range.clone());
        self.copy_into(range.start, &merged);
    }
    /// Copy a relative subtree range without expanding runs into characters.
    pub fn slice(&self, range: Range<ByteOffset>) -> Self {
        let mut out = Self::new();
        out.stamp = self.stamp;
        for (key, root) in &self.layers {
            let mut work = 0;
            let part = root.copy_range(range.start.0, range.end.0, &mut work);
            if let Some(part) = part.filter(|n| n.present() != 0) {
                out.layers.insert(*key, part);
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
            root.walk(0, &mut |start, r| {
                if r.stamp != 0 {
                    runs.push((r.stamp, *key, start, r.len as usize, r.value));
                }
            });
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
    /// Every value, and every key's symbol: a key keeps its symbol alive.
    pub fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit_keys(&mut self.layers, visit);
        for root in self.layers.values_mut() {
            root.visit(visit);
        }
    }
    pub fn storage_bytes(&self) -> usize {
        // Conservative BTree node reservation: up to one 11-slot node per key.
        self.layers.len() * 11 * (size_of::<Key>() + size_of::<Box<Tree>>())
            + self.trees().map(Tree::storage).sum::<usize>()
            + self.syntax_changes.as_ref().map_or(0, |changes| {
                size_of::<SyntaxChanges>() + changes.ranges.capacity() * size_of::<Range<usize>>()
            })
    }
}
