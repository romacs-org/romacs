//! The buffer list's order, GNU's `Vbuffer_alist`: creation order, with
//! recorded buffers moved to the front and buried ones to the back. It is a
//! list linked through an arena the VM owns, so a create, kill, record or
//! bury is O(1) and nothing is consed until `buffer-list` asks.
//!
//! Each live buffer holds its node's index (`BufferState::order_node`), plain
//! data. The nodes hold the buffers, which the collector marks and rewrites
//! as roots (`visit_values`), as it did the Lisp list this replaces. It walks
//! the links, never the arena: the arena keeps the session's peak number of
//! nodes, and a collection's work must be the live buffers', not that
//! history's. A name slot (`#:vm2-buffer:NAME`) is a symbol Lisp can set, so
//! it roots nothing the order may rely on.
use crate::Value;

#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct BufferOrder {
    nodes: Vec<Node>,
    free: Vec<u32>,
    head: Option<u32>,
    tail: Option<u32>,
}

#[derive(Clone, Copy, Debug, romacs_snapshot::Snapshot)]
struct Node {
    buffer: Value,
    prev: Option<u32>,
    next: Option<u32>,
}

/// The same walk serves buffer-list, marking and moving. A cycle reaches a
/// node from a second predecessor (or revisits the head) and fails before
/// another copy of its root reaches the collector's worklist.
struct Links {
    next: Option<u32>,
    prev: Option<u32>,
    remaining: usize,
}

impl Links {
    fn step(&mut self, order: &BufferOrder) -> Option<u32> {
        let Some(at) = self.next else {
            assert_eq!(self.prev, order.tail, "buffer order ends before its tail");
            assert_eq!(self.remaining, 0, "buffer order has unlinked nodes");
            return None;
        };
        let node = &order.nodes[at as usize];
        assert_eq!(node.prev, self.prev, "broken back link at buffer order node {at}");
        assert!(self.remaining > 0, "buffer order exceeds its live count");
        self.remaining -= 1;
        self.prev = Some(at);
        self.next = node.next;
        Some(at)
    }
}

impl BufferOrder {
    /// BUFFER joins at the back; its node.
    pub(crate) fn push_back(&mut self, buffer: Value) -> u32 {
        let node = Node { buffer, prev: self.tail, next: None };
        let at = match self.free.pop() {
            Some(at) => { self.nodes[at as usize] = node; at }
            None => { self.nodes.push(node); (self.nodes.len() - 1) as u32 }
        };
        self.link_after_tail(at);
        at
    }
    /// NODE leaves the order and its place is freed.
    pub(crate) fn release(&mut self, node: u32) {
        self.unlink(node);
        self.nodes[node as usize].buffer = Value::NIL;
        self.free.push(node);
    }
    /// NODE moves to the front (record_buffer).
    pub(crate) fn move_to_front(&mut self, node: u32) {
        if self.head == Some(node) { return; }
        self.unlink(node);
        let n = &mut self.nodes[node as usize];
        (n.prev, n.next) = (None, self.head);
        match self.head {
            Some(head) => self.nodes[head as usize].prev = Some(node),
            None => self.tail = Some(node),
        }
        self.head = Some(node);
    }
    /// NODE moves to the back (Fbury_buffer_internal).
    pub(crate) fn move_to_back(&mut self, node: u32) {
        if self.tail == Some(node) { return; }
        self.unlink(node);
        let n = &mut self.nodes[node as usize];
        n.prev = self.tail;
        self.link_after_tail(node);
    }
    fn links(&self) -> Links {
        assert!(self.free.len() <= self.nodes.len(), "too many free buffer order nodes");
        let remaining = self.nodes.len() - self.free.len();
        Links { next: self.head, prev: None, remaining }
    }
    /// The buffers, front to back.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        self.linked().map(|(_, buffer)| buffer)
    }
    /// Mark or rewrite only the live buffers, even after a much larger peak.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        let mut links = self.links();
        while let Some(at) = links.step(self) {
            visit(&mut self.nodes[at as usize].buffer);
        }
    }
    pub(crate) fn linked(&self) -> impl Iterator<Item = (u32, Value)> + '_ {
        let mut links = self.links();
        std::iter::from_fn(move || {
            let at = links.step(self)?;
            Some((at, self.nodes[at as usize].buffer))
        })
    }
    fn link_after_tail(&mut self, at: u32) {
        self.nodes[at as usize].next = None;
        match self.tail {
            Some(tail) => self.nodes[tail as usize].next = Some(at),
            None => self.head = Some(at),
        }
        self.tail = Some(at);
    }
    fn unlink(&mut self, at: u32) {
        let Node { prev, next, .. } = self.nodes[at as usize];
        match prev { Some(p) => self.nodes[p as usize].next = next, None => self.head = next }
        match next { Some(n) => self.nodes[n as usize].prev = prev, None => self.tail = prev }
        let n = &mut self.nodes[at as usize];
        (n.prev, n.next) = (None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(o: &BufferOrder) -> Vec<u64> { o.iter().map(|v| v.0).collect() }
    fn v(n: u64) -> Value { Value(n) }

    /// The exhaustive arena audit belongs at explicit test points. A collection
    /// needs the free count, but should never read released nodes or indices.
    fn check_arena(o: &BufferOrder) {
        let mut seen = vec![false; o.nodes.len()];
        for (at, _) in o.linked() { seen[at as usize] = true; }
        for &at in &o.free {
            let i = at as usize;
            assert!(!std::mem::replace(&mut seen[i], true), "node {at} is linked or free twice");
            assert_eq!(o.nodes[i].buffer, Value::NIL, "free node {at} holds a buffer");
        }
        assert!(seen.into_iter().all(|s| s), "unaccounted buffer order node");
    }

    #[test]
    fn steps_keep_the_order_and_reuse_freed_nodes() {
        let mut o = BufferOrder::default();
        let [a, b, c] = [1, 2, 3].map(|n| o.push_back(v(n)));
        assert_eq!(order(&o), [1, 2, 3]);
        o.move_to_front(b);
        assert_eq!(order(&o), [2, 1, 3]);
        o.move_to_back(b);
        assert_eq!(order(&o), [1, 3, 2]);
        o.move_to_back(b);
        o.move_to_front(a);
        assert_eq!(order(&o), [1, 3, 2]);
        o.release(c);
        assert_eq!(order(&o), [1, 2]);
        let d = o.push_back(v(4));
        assert_eq!(d, c, "a freed node is reused");
        assert_eq!(order(&o), [1, 2, 4]);
        for node in [a, b, d] { o.release(node); }
        assert_eq!(order(&o), Vec::<u64>::new());
        o.push_back(v(5));
        assert_eq!(order(&o), [5]);
        check_arena(&o);
    }

    #[test]
    fn collection_visits_only_the_linked_nodes() {
        let mut o = BufferOrder::default();
        let nodes: Vec<u32> = (1..=100).map(|n| o.push_back(v(n))).collect();
        for &node in &nodes[2..] { o.release(node); }
        let mut seen = Vec::new();
        o.visit_values(&mut |b| seen.push(b.0));
        assert_eq!(seen, [1, 2], "a freed node is no root");
        check_arena(&o);
    }

    #[test]
    #[should_panic(expected = "buffer order has unlinked nodes")]
    fn a_walk_detects_a_node_lost_from_the_arena() {
        let mut o = BufferOrder::default();
        let [_, b, _] = [1, 2, 3].map(|n| o.push_back(v(n)));
        o.unlink(b);
        o.visit_values(&mut |_| {});
    }

    #[test]
    #[should_panic(expected = "broken back link")]
    fn a_cyclic_root_walk_stops_before_visiting_a_buffer_twice() {
        let mut o = BufferOrder::default();
        let [a, b] = [1, 2].map(|n| o.push_back(v(n)));
        o.nodes[b as usize].next = Some(a);
        let mut count = 0;
        o.visit_values(&mut |_| {
            count += 1;
            assert!(count <= 2, "the collector revisited a root");
        });
    }

    #[test]
    fn a_verified_full_collection_does_not_read_the_free_nodes() {
        let mut vm = crate::Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.eval_source(r#"(progn (get-buffer-create "peak") (kill-buffer "peak"))"#).unwrap();
        // Dereferencing this deliberately invalid free index fails immediately.
        *vm.buffer_order.free.last_mut().expect("released node") = u32::MAX;
        vm.set_gc_verify(true);
        vm.collect_garbage();
    }
}
