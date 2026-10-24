//! Bias-partitioned interval and endpoint treaps over registered anchors.
use crate::{
    Value,
    anchors::{Anchor, AnchorTree, Bias},
};
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Links {
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
}
#[derive(Clone)]
struct Node {
    value: Value,
    start: Anchor,
    end: Anchor,
    bounds: [Anchor; 4],
    group: usize,
    links: [Links; 2],
}
impl Node {
    fn endpoint(&self, order: usize) -> Anchor {
        if order == 0 { self.start } else { self.end }
    }
}
#[derive(Clone, Default)]
pub(crate) struct IntervalIndex {
    nodes: Vec<Node>,
    slots: HashMap<Anchor, usize>,
    free: Vec<usize>,
    roots: [[Option<usize>; 4]; 2],
}
fn priority(i: usize) -> u64 {
    let mut x = (i as u64).wrapping_add(1).wrapping_mul(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x ^ (x >> 27)
}
impl IntervalIndex {
    fn pull(&mut self, i: usize, anchors: &AnchorTree) {
        let node = &self.nodes[i];
        let mut bounds = [node.start, node.start, node.end, node.end];
        for child in [node.links[0].left, node.links[0].right]
            .into_iter()
            .flatten()
        {
            assert_eq!(self.nodes[child].group, node.group);
            for (j, &h) in self.nodes[child].bounds.iter().enumerate() {
                let pos = anchors.resolve(h).expect("live interval endpoint");
                let old = anchors.resolve(bounds[j]).expect("live interval bound");
                if if j % 2 == 0 { pos < old } else { pos > old } {
                    bounds[j] = h;
                }
            }
        }
        self.nodes[i].bounds = bounds;
    }
    fn repair(&mut self, mut i: Option<usize>, anchors: &AnchorTree) {
        while let Some(n) = i {
            self.pull(n, anchors);
            i = self.nodes[n].links[0].parent;
        }
    }
    fn replace_child(
        &mut self,
        order: usize,
        parent: Option<usize>,
        old: usize,
        new: Option<usize>,
    ) {
        if let Some(p) = parent {
            if self.nodes[p].links[order].left == Some(old) {
                self.nodes[p].links[order].left = new;
            } else {
                assert_eq!(self.nodes[p].links[order].right, Some(old));
                self.nodes[p].links[order].right = new;
            }
        } else {
            self.roots[order][self.nodes[old].group] = new;
        }
        if let Some(n) = new {
            self.nodes[n].links[order].parent = parent;
        }
    }
    fn rotate_up(&mut self, order: usize, i: usize, anchors: &AnchorTree) {
        let p = self.nodes[i].links[order].parent.unwrap();
        let grand = self.nodes[p].links[order].parent;
        if self.nodes[p].links[order].left == Some(i) {
            let middle = self.nodes[i].links[order].right;
            self.nodes[p].links[order].left = middle;
            self.nodes[i].links[order].right = Some(p);
            if let Some(m) = middle {
                self.nodes[m].links[order].parent = Some(p);
            }
        } else {
            let middle = self.nodes[i].links[order].left;
            self.nodes[p].links[order].right = middle;
            self.nodes[i].links[order].left = Some(p);
            if let Some(m) = middle {
                self.nodes[m].links[order].parent = Some(p);
            }
        }
        self.replace_child(order, grand, p, Some(i));
        self.nodes[p].links[order].parent = Some(i);
        if order == 0 {
            self.pull(p, anchors);
            self.pull(i, anchors);
        }
    }
    pub fn insert(&mut self, value: Value, start: Anchor, end: Anchor, anchors: &AnchorTree) {
        assert!(!self.slots.contains_key(&start));
        let group = usize::from(anchors.bias(start) == Some(Bias::Right)) * 2
            + usize::from(anchors.bias(end) == Some(Bias::Right));
        let node = Node {
            value,
            start,
            end,
            group,
            bounds: [start, start, end, end],
            links: [Links::default(); 2],
        };
        let i = if let Some(i) = self.free.pop() {
            self.nodes[i] = node;
            i
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        };
        if self.free.capacity() < self.nodes.len() {
            self.free.reserve(self.nodes.len() - self.free.len());
        }
        self.slots.insert(start, i);
        for order in 0..2 {
            let key = (anchors.resolve(self.nodes[i].endpoint(order)).unwrap(), i);
            if let Some(mut p) = self.roots[order][group] {
                loop {
                    let left = key < (anchors.resolve(self.nodes[p].endpoint(order)).unwrap(), p);
                    let child = if left {
                        self.nodes[p].links[order].left
                    } else {
                        self.nodes[p].links[order].right
                    };
                    if let Some(c) = child {
                        p = c;
                    } else {
                        if left {
                            self.nodes[p].links[order].left = Some(i);
                        } else {
                            self.nodes[p].links[order].right = Some(i);
                        }
                        self.nodes[i].links[order].parent = Some(p);
                        break;
                    }
                }
            } else {
                self.roots[order][group] = Some(i);
            }
            while self.nodes[i].links[order]
                .parent
                .is_some_and(|p| priority(i) > priority(p))
            {
                self.rotate_up(order, i, anchors);
            }
        }
        self.repair(Some(i), anchors);
    }
    pub fn remove(&mut self, start: Anchor, anchors: &AnchorTree) {
        let Some(i) = self.slots.remove(&start) else {
            return;
        };
        for order in 0..2 {
            while self.nodes[i].links[order].left.is_some()
                || self.nodes[i].links[order].right.is_some()
            {
                let child = match (
                    self.nodes[i].links[order].left,
                    self.nodes[i].links[order].right,
                ) {
                    (Some(l), Some(r)) => {
                        if priority(l) > priority(r) {
                            l
                        } else {
                            r
                        }
                    }
                    (Some(c), None) | (None, Some(c)) => c,
                    _ => unreachable!(),
                };
                self.rotate_up(order, child, anchors);
            }
            let parent = self.nodes[i].links[order].parent;
            self.replace_child(order, parent, i, None);
            if order == 0 {
                self.repair(parent, anchors);
            }
        }
        self.nodes[i].value = Value::NIL;
        self.free.push(i);
    }
    pub fn refresh(&mut self, start: Anchor, anchors: &AnchorTree) {
        if let Some(&i) = self.slots.get(&start) {
            let (value, end) = (self.nodes[i].value, self.nodes[i].end);
            self.remove(start, anchors);
            self.insert(value, start, end, anchors);
        }
    }
    fn extent(&self, i: usize, kind: usize, anchors: &AnchorTree) -> u32 {
        anchors.resolve(self.nodes[i].bounds[kind]).unwrap()
    }
    pub fn visit(
        &self,
        a: u32,
        b: u32,
        point: bool,
        anchors: &AnchorTree,
        mut emit: impl FnMut(Value, u32, u32),
    ) -> usize {
        self.visit_with_end(a, b, point, b == anchors.extent(), anchors, &mut emit)
    }
    fn visit_with_end(
        &self,
        a: u32,
        b: u32,
        point: bool,
        include_end: bool,
        anchors: &AnchorTree,
        mut emit: impl FnMut(Value, u32, u32),
    ) -> usize {
        let mut visited = 0;
        fn visit(
            t: &IntervalIndex,
            i: Option<usize>,
            range: (u32, u32, bool, bool),
            anchors: &AnchorTree,
            emit: &mut impl FnMut(Value, u32, u32),
            visited: &mut usize,
        ) {
            let Some(i) = i else {
                return;
            };
            let (a, b, point, include_end) = range;
            let min_start = t.extent(i, 0, anchors);
            let max_end = t.extent(i, 3, anchors);
            let overlaps = max_end > a && if point { min_start <= a } else { min_start < b };
            let empties = !point
                && max_end >= a
                && min_start <= b
                && (a == b || include_end || min_start < b)
                && t.extent(i, 1, anchors) >= a
                && t.extent(i, 2, anchors) <= b
                && (a == b || include_end || t.extent(i, 2, anchors) < b);
            if !overlaps && !empties {
                return;
            }
            *visited += 1;
            let n = &t.nodes[i];
            let s = anchors.resolve(n.start).unwrap();
            let e = anchors.resolve(n.end).unwrap();
            let hit = if point {
                s <= a && a < e
            } else {
                (s < b && e > a)
                    || (s == e && a <= s && (s < b || ((a == b || include_end) && s == b)))
            };
            if hit {
                emit(n.value, s, e);
            }
            visit(t, n.links[0].left, range, anchors, emit, visited);
            visit(t, n.links[0].right, range, anchors, emit, visited);
        }
        for root in self.roots[0] {
            visit(
                self,
                root,
                (a, b, point, include_end),
                anchors,
                &mut emit,
                &mut visited,
            );
        }
        visited
    }
    #[cfg(test)]
    pub fn query(&self, a: u32, b: u32, point: bool, anchors: &AnchorTree) -> (Vec<Value>, usize) {
        self.query_with_end(a, b, point, b == anchors.extent(), anchors)
    }
    pub fn query_with_end(
        &self,
        a: u32,
        b: u32,
        point: bool,
        include_end: bool,
        anchors: &AnchorTree,
    ) -> (Vec<Value>, usize) {
        let mut found = Vec::new();
        let visited =
            self.visit_with_end(a, b, point, include_end, anchors, |v, _, _| found.push(v));
        (found, visited)
    }
    pub fn next_counted(&self, a: u32, limit: u32, anchors: &AnchorTree) -> (u32, usize) {
        let mut best = limit;
        let mut visited = 0;
        for order in 0..2 {
            for root in self.roots[order] {
                let mut current = root;
                while let Some(i) = current {
                    visited += 1;
                    let node = &self.nodes[i];
                    let pos = anchors.resolve(node.endpoint(order)).unwrap();
                    if pos > a {
                        best = best.min(pos);
                        current = node.links[order].left;
                    } else {
                        current = node.links[order].right;
                    }
                }
            }
        }
        (best, visited)
    }
    pub fn next(&self, a: u32, limit: u32, anchors: &AnchorTree) -> u32 {
        self.next_counted(a, limit, anchors).0
    }
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        for &i in self.slots.values() {
            f(&mut self.nodes[i].value);
        }
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        (if capacity {
            self.nodes.capacity()
        } else {
            self.nodes.len()
        }) * size_of::<Node>()
            + (if capacity {
                self.free.capacity()
            } else {
                self.free.len()
            }) * size_of::<usize>()
            + (if capacity {
                self.slots.capacity() * 2
            } else {
                self.slots.len()
            }) * (size_of::<(Anchor, usize)>() + 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn assert_invariants(index: &IntervalIndex, anchors: &AnchorTree) {
        fn check(index: &IntervalIndex, i: usize, anchors: &AnchorTree) -> ([u32; 4], usize) {
            let n = &index.nodes[i];
            let s = anchors.resolve(n.start).unwrap();
            let e = anchors.resolve(n.end).unwrap();
            let mut bounds = [s, s, e, e];
            let mut count = 1;
            for (child, left) in [(n.links[0].left, true), (n.links[0].right, false)] {
                if let Some(c) = child {
                    assert_eq!(index.nodes[c].links[0].parent, Some(i));
                    assert_eq!(index.nodes[c].group, n.group);
                    assert!(priority(i) >= priority(c));
                    let (b, len) = check(index, c, anchors);
                    assert!(
                        if left { b[1] <= s } else { b[0] >= s },
                        "interval start order"
                    );
                    count += len;
                    for j in 0..4 {
                        bounds[j] = if j % 2 == 0 {
                            bounds[j].min(b[j])
                        } else {
                            bounds[j].max(b[j])
                        };
                    }
                }
            }
            for (j, &h) in n.bounds.iter().enumerate() {
                assert_eq!(
                    anchors.resolve(h).unwrap(),
                    bounds[j],
                    "subtree endpoint bound"
                );
            }
            (bounds, count)
        }
        let mut count = 0;
        for root in index.roots[0].into_iter().flatten() {
            assert_eq!(index.nodes[root].links[0].parent, None);
            count += check(index, root, anchors).1;
        }
        assert_eq!(count, index.slots.len());
        fn ends(index: &IntervalIndex, i: usize, anchors: &AnchorTree, out: &mut Vec<u32>) {
            let n = &index.nodes[i];
            if let Some(c) = n.links[1].left {
                assert_eq!(index.nodes[c].links[1].parent, Some(i));
                assert!(priority(i) >= priority(c));
                ends(index, c, anchors, out);
            }
            out.push(anchors.resolve(n.end).unwrap());
            if let Some(c) = n.links[1].right {
                assert_eq!(index.nodes[c].links[1].parent, Some(i));
                assert!(priority(i) >= priority(c));
                ends(index, c, anchors, out);
            }
        }
        let mut count = 0;
        for root in index.roots[1].into_iter().flatten() {
            assert_eq!(index.nodes[root].links[1].parent, None);
            let mut ordered = Vec::new();
            ends(index, root, anchors, &mut ordered);
            assert!(ordered.windows(2).all(|w| w[0] <= w[1]));
            count += ordered.len();
        }
        assert_eq!(count, index.slots.len());
    }
    #[test]
    fn interval_extrema_survive_collapses_biases_and_moves() {
        use crate::{annotations::AnnotatedBuffer, coordinates::ByteOffset, text::Text};
        let mut buffer: AnnotatedBuffer = Text::new(&vec![b'x'; 4096]).unwrap().into();
        let mut handles = Vec::new();
        let mut seed = 0x1833_7367_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u32
        };
        for i in 0..257 {
            let s = random() % 4096;
            let e = (s + random() % 101).min(4096);
            let front = if i & 1 == 0 { Bias::Left } else { Bias::Right };
            let rear = if i & 2 == 0 { Bias::Left } else { Bias::Right };
            let a = buffer.anchors.create(s, front).unwrap();
            let b = buffer.anchors.create(e, rear).unwrap();
            if front == Bias::Right && rear == Bias::Left {
                buffer.reversed_pairs.insert(a, b);
            }
            let value = Value::integer(i).unwrap();
            buffer.intervals.insert(value, a, b, &buffer.anchors);
            handles.push((value, a, b, front, rear));
        }
        for step in 0..2000 {
            let extent = buffer.anchors.extent();
            let s = random() % (extent + 1);
            let e = (s + random() % 80).min(extent);
            if step % 5 == 0 {
                let j = random() as usize % handles.len();
                let (v, a, b, front, rear) = handles[j];
                buffer.intervals.remove(a, &buffer.anchors);
                buffer.anchors.set(a, s, front);
                buffer.anchors.set(b, e, rear);
                buffer.intervals.insert(v, a, b, &buffer.anchors);
            } else {
                let len = random() as usize % 90;
                buffer
                    .replace_with_bias(
                        ByteOffset(s as usize)..ByteOffset(e as usize),
                        &vec![b'y'; len],
                        step % 7 == 0,
                    )
                    .unwrap();
            }
            assert_invariants(&buffer.intervals, &buffer.anchors);
            let extent = buffer.anchors.extent();
            for point in [true, false] {
                let a = random() % (extent + 1);
                let b = if point || step % 3 == 0 {
                    a
                } else {
                    (a + random() % 100).min(extent)
                };
                let mut expected = Vec::new();
                let mut next = extent;
                for &(v, start, end, _, _) in &handles {
                    let s = buffer.anchors.resolve(start).unwrap();
                    let e = buffer.anchors.resolve(end).unwrap();
                    assert!(s <= e, "step={step}: {s}..{e}");
                    if s > a {
                        next = next.min(s);
                    }
                    if e > a {
                        next = next.min(e);
                    }
                    let hit = if point {
                        s <= a && a < e
                    } else {
                        (s < b && e > a)
                            || (s == e && a <= s && (s < b || ((a == b || b == extent) && s == b)))
                    };
                    if hit {
                        expected.push(v.as_integer().unwrap());
                    }
                }
                let (actual, _) = buffer.intervals.query(a, b, point, &buffer.anchors);
                let mut actual: Vec<_> = actual
                    .into_iter()
                    .map(|v| v.as_integer().unwrap())
                    .collect();
                actual.sort_unstable();
                assert_eq!(actual, expected, "step={step} range={a}..{b} point={point}");
                assert_eq!(
                    buffer.intervals.next(a, extent, &buffer.anchors),
                    next,
                    "successor step={step}"
                );
            }
        }
    }
    #[test]
    fn shared_endpoints_prune_zero_hit_ranges() {
        for layout in 0..3 {
            let mut anchors = AnchorTree::new(100_000);
            let mut index = IntervalIndex::default();
            for i in 0..6673 {
                let (s, e) = match layout {
                    0 => (i, 20_000),
                    1 => (20_000, 30_000 + i),
                    _ => (500, 600),
                };
                let a = anchors
                    .create(s, if i & 1 == 0 { Bias::Left } else { Bias::Right })
                    .unwrap();
                let b = anchors
                    .create(e, if i & 2 == 0 { Bias::Left } else { Bias::Right })
                    .unwrap();
                index.insert(Value::integer(i as i64).unwrap(), a, b, &anchors);
            }
            let (a, b) = match layout {
                0 => (20_000, 20_010),
                1 => (19_990, 20_000),
                _ => (600, 610),
            };
            let (hits, visited) = index.query(a, b, false, &anchors);
            assert!(hits.is_empty());
            eprintln!(
                "shared-endpoint layout={layout}: hits={} candidates={visited}/6673",
                hits.len()
            );
            assert!(
                visited < 128,
                "shared-endpoint query visited {visited}/6673"
            );
            let empty = anchors.create(a, Bias::Left).unwrap();
            let end = anchors.create(a, Bias::Left).unwrap();
            index.insert(Value::TRUE, empty, end, &anchors);
            let (hits, visited) = index.query(a, b, false, &anchors);
            assert_eq!(hits, [Value::TRUE]);
            assert!(
                visited < 128,
                "one empty overlay hid a full scan: {visited}"
            );
            assert!(anchors.delete(0, 100_000));
            let (hits, _) = index.query(0, 0, false, &anchors);
            assert_eq!(
                hits.len(),
                6674,
                "collapsed empty intervals must remain visible"
            );
            assert_invariants(&index, &anchors);
        }
    }
    #[test]
    fn real_notmuch_scale_queries_prune() {
        let mut anchors = AnchorTree::new(100_000);
        let mut index = IntervalIndex::default();
        let mut order: Vec<u32> = (0..6673).collect();
        let mut seed = 0x6673_9276_u64;
        for j in (1..order.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            order.swap(j, seed as usize % (j + 1));
        }
        for &i in &order {
            let a = anchors
                .create(i * 10, if i & 1 == 0 { Bias::Left } else { Bias::Right })
                .unwrap();
            let b = anchors
                .create(
                    i * 10 + 5,
                    if i & 2 == 0 { Bias::Left } else { Bias::Right },
                )
                .unwrap();
            index.insert(Value::integer(i as i64).unwrap(), a, b, &anchors);
        }
        eprintln!(
            "interval arena node={} bytes (plus map/free/Vec slack)",
            size_of::<Node>()
        );
        let mut worst = 0;
        for i in 0..6673 {
            let (found, visited) = index.query(i * 10 + 2, i * 10 + 2, true, &anchors);
            assert_eq!(found, [Value::integer(i as i64).unwrap()]);
            worst = worst.max(visited);
            assert!(visited < 128, "query visited {visited} identities");
            let (next, successor_visits) =
                index.next_counted(i * 10 + 2, anchors.extent(), &anchors);
            assert_eq!(next, i * 10 + 5);
            assert!(
                successor_visits < 256,
                "successor scanned {successor_visits} identities"
            );
        }
        eprintln!("shuffled notmuch fixture: worst={worst}/6673 across all positions");
    }
}
