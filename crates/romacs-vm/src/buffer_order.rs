//! The buffer list's order, GNU's `Vbuffer_alist`: creation order, with
//! recorded buffers moved to the front and buried ones to the back. It is a
//! list linked through an arena the VM owns, so a create, kill, record or
//! bury is O(1) and nothing is consed until `buffer-list` asks.
//!
//! Each live buffer holds its node's index (`BufferState::order_node`), plain
//! data. The nodes hold the buffers, which the collector visits as roots
//! (`visit_values`), as it did the Lisp list this replaces.
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
    /// The buffers, front to back.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        std::iter::successors(self.head, |&at| self.nodes[at as usize].next).map(|at| self.nodes[at as usize].buffer)
    }
    /// Every buffer the order holds, for the collector to mark or rewrite.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for node in &mut self.nodes { visit(&mut node.buffer); }
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
    }
}
