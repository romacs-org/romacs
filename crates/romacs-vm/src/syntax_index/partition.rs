//! The index's partition of its text into leaves, made where scans reach.
//! A new tree is one raw leaf: no scan is served by it and no context
//! checks it. A scan that reaches a raw leaf cuts from it the leaf it is in
//! (`materialize`), and each context checks a leaf's end when a scan it
//! serves reaches that leaf (`ready`), so that what a scan costs the
//! partition, in work and in memory, stays within what it reads.
use super::*;

/// A leaf some context has not checked: where it starts, its characters,
/// the contexts that have not checked it, whether any found its end
/// unsafe, and whether it is raw.
pub(super) struct Pending {
    pub(super) start: usize,
    chars: usize,
    unchecked: u8,
    unsafe_any: bool,
    raw: bool,
}

impl SyntaxIndex {
    /// Make the leaves over offsets FROM..TO ready to serve context slot
    /// SLOT: a raw one cut into leaves, each end SLOT has not checked
    /// checked. The work is that of the leaves in range. Whether there was
    /// any.
    pub(super) fn ready<T: Text>(&mut self, scanner: &Scanner<'_, T>, slot: u8, (from, to): (usize, usize)) -> bool {
        let mut worked = false;
        while let Some(root) = &self.root
            && let Some(pending) = first_unchecked(root, 0, from, 1 << slot)
            && pending.start < to
        {
            if pending.raw {
                self.materialize(pending.start.max(from), to);
            } else {
                self.check_end(scanner, slot, &pending);
            }
            worked = true;
        }
        worked
    }

    /// The first leaf whose end context slot SLOT has not checked, ending
    /// after offset FROM.
    pub(super) fn pending(&self, slot: u8, from: usize) -> Option<Pending> {
        first_unchecked(self.root.as_ref()?, 0, from, 1 << slot)
    }

    /// Check the end of PENDING for context slot SLOT. One no context has
    /// checked moves past a possible split token, at most a leaf's length
    /// on, taking those characters from the next leaves; one with no safe
    /// cut in range, or that another context checked, is marked unsafe for
    /// SLOT where it is.
    fn check_end<T: Text>(&mut self, scanner: &Scanner<'_, T>, slot: u8, pending: &Pending) {
        let leaf_chars = self.leaf;
        let Some(root) = &mut self.root else { return };
        let text = scanner.text;
        let syntax = |at: usize| text.syntax(text.pos(position(at)));
        // Whether a scan can stop at offset AT and resume exactly.
        let total = root.chars;
        let safe = |at: usize| {
            if at == 0 || at >= total {
                return true;
            }
            let before = syntax(at - 1);
            !matches!(before & 0xff, scan::SESCAPE | scan::SCHARQUOTE | scan::SMATH)
                && !(scan::comstart_first(before) && scan::comstart_second(syntax(at)))
        };
        let (bit, start, end) = (1 << slot, pending.start, pending.start + pending.chars);
        if pending.unchecked != EVERY {
            // Unsafe after an edit beside it: what the path keeps, made
            // while it was safe, is not its span's.
            let unsafe_end = !safe(end);
            update(root, start, unsafe_end, &mut |leaf| {
                leaf.unchecked &= !bit;
                leaf.unsafe_ends |= if unsafe_end { bit } else { 0 };
            });
            return;
        }
        let moved = (end..=(end + leaf_chars).min(total)).find(|&at| safe(at));
        let mut unsafe_end = moved.is_none();
        if let Some(moved) = moved.filter(|&moved| moved > end) {
            let mut left = moved - end;
            while left > 0 {
                let mut took = 0;
                update(root, end, true, &mut |next| {
                    took = next.chars.min(left);
                    next.chars -= took;
                });
                left -= took;
            }
            update(root, start, true, &mut |leaf| leaf.chars += moved - end);
            unsafe_end = false;
        }
        // An end that is or was unsafe: what the path keeps is not its span's.
        update(root, start, unsafe_end || pending.unsafe_any, &mut |leaf| {
            leaf.unchecked = EVERY & !bit;
            leaf.unsafe_ends = if unsafe_end { bit } else { 0 };
        });
    }

    /// Cut from the raw leaf holding offset AT the leaves of LEAF characters
    /// from the one AT is in to the one before offset TO, counted from the
    /// raw leaf's start: what lies before and after them stays raw. The
    /// nodes above lose what they kept, which no raw leaf below them let
    /// them compose.
    fn materialize(&mut self, at: usize, to: usize) {
        let leaf = self.leaf;
        let Some(root) = &mut self.root else { return };
        if root.is_leaf() {
            if !root.raw {
                return;
            }
            let mut pieces = cut_raw(root.chars, (at, to), leaf);
            *root = if pieces.len() == 1 { pieces.remove(0) } else { Node { children: pieces, ..Node::default() } };
            root.sum_if_inner();
        } else {
            materialize_node(root, 0, (at, to), leaf);
        }
        if root.children.len() > FANOUT {
            let children = std::mem::take(&mut root.children);
            *root = Self::build(Node::group(children));
        }
    }
}

impl Node {
    /// `sum` where it has children.
    fn sum_if_inner(&mut self) {
        if !self.is_leaf() {
            self.sum();
        }
    }
}

/// The leaves a raw leaf of CHARS becomes when the leaves of LEAF
/// characters from the one holding offset AT (in it) to the one holding TO
/// are cut from it: raw before, those leaves, raw after. Its cuts fall
/// where a whole text's would, every LEAF characters from its start.
fn cut_raw(chars: usize, (at, to): (usize, usize), leaf: usize) -> Vec<Node> {
    let start = at.min(chars) / leaf * leaf;
    let end = (to.max(at + 1).min(chars).div_ceil(leaf) * leaf).clamp(start + leaf, chars.max(start + leaf)).min(chars);
    let mut pieces = Vec::new();
    if start > 0 {
        pieces.push(Node::raw(start));
    }
    pieces.extend((start..end).step_by(leaf).map(|at| Node::leaf(leaf.min(end - at))));
    if end < chars {
        pieces.push(Node::raw(chars - end));
    }
    pieces
}

/// `SyntaxIndex::materialize` under NODE, which starts at START.
fn materialize_node(node: &mut Node, start: usize, (at, to): (usize, usize), leaf: usize) {
    node.spans.clear();
    let (last, mut child_start, mut i) = (node.children.len() - 1, start, 0);
    while i < last && at >= child_start + node.children[i].chars {
        child_start += node.children[i].chars;
        i += 1;
    }
    let child = &mut node.children[i];
    if child.is_leaf() {
        if child.raw {
            let pieces = cut_raw(child.chars, (at - child_start, to.saturating_sub(child_start)), leaf);
            node.children.splice(i..=i, pieces);
        }
    } else {
        materialize_node(child, child_start, (at, to), leaf);
        if child.children.len() > FANOUT {
            let grandchildren = std::mem::take(&mut child.children);
            node.children.splice(i..=i, Node::group(grandchildren));
        }
    }
    node.sum();
}

/// The first leaf under NODE (at START) ending after offset FROM whose end
/// the context of BIT has not checked.
fn first_unchecked(node: &Node, start: usize, from: usize, bit: u8) -> Option<Pending> {
    if node.unchecked & bit == 0 || start + node.chars <= from {
        return None;
    }
    if node.is_leaf() {
        return Some(Pending { start, chars: node.chars, unchecked: node.unchecked, unsafe_any: node.unsafe_ends != 0, raw: node.raw });
    }
    let mut at = start;
    for child in &node.children {
        if let Some(found) = first_unchecked(child, at, from, bit) {
            return Some(found);
        }
        at += child.chars;
    }
    None
}
