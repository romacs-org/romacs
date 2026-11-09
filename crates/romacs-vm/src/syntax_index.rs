//! The parse-state summaries of a text (`crate::syntax_summary`), kept in
//! a tree beside it: leaves of about a leaf's characters ([`LEAF`]) under
//! a B-tree, each node holding its span's summary for the keys it was
//! entered in. A parse over many leaves scans only its two partial ends and
//! applies O(log n) summaries between. An edit changes the leaves it
//! touches and the nodes above them, nothing else: every position a
//! summary holds is relative to its node, and a split or a removal stays
//! inside the edited path.
//!
//! Summaries read the text, the syntax table and `comment-end-can-be-escaped'
//! and nothing else (scan_sexps_forward's inputs), so the index is valid
//! for one [`Context`]; a parse in another drops every summary and keeps the
//! leaves. Where `syntax-table' properties are honoured, summaries read
//! them as the scanner does, and a change of the property drops the
//! summaries of the leaves it touches (`TextBody::sync_syntax_properties`).
//!
//! A leaf never ends between the two characters of a two-character comment
//! start, where a scan cannot stop and resume exactly
//! (`crate::syntax_scan::Continuation`): an edit, a property change or a new
//! context leaves its end unchecked, and the next parse moves such an end
//! past the pair, at most a leaf's length on; a leaf with no such place
//! keeps an unsafe end, and a parse across it is not served.
//!
//! It holds no Lisp value: a derived cache, neither a root nor visited by
//! the collector, and a copy of a text starts without one.
use crate::syntax_scan::{ParseState, R, Scanner, Text};
use crate::syntax_summary::{self as summary, Key, Span};

/// Characters in a leaf the index cuts; a leaf grown past twice this splits.
pub(crate) const LEAF: usize = 2048;
/// Children a node holds before it splits.
const FANOUT: usize = 16;
/// Keys a node keeps a summary for; the oldest goes first.
const KEYS: usize = 4;

/// What summaries depend on besides the text: the syntax table, by its
/// identity and the char-table writes so far (`crate::char_table::writes`:
/// any write to any table, its parents' included, renews it),
/// `comment-end-can-be-escaped', and whether `syntax-table' properties are
/// honoured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Context {
    pub table: usize,
    pub writes: u64,
    pub escaped_ends: bool,
    pub properties: bool,
}
romacs_snapshot::snapshot_struct!(Context { table, writes, escaped_ends, properties });

/// A leaf (no children) or an inner node of the tree.
#[derive(Clone, Debug, Default)]
struct Node {
    chars: usize,
    spans: Vec<(Key, Span)>,
    /// Leaves below (a leaf: itself) whose end is not yet known to lie
    /// outside every two-character comment start, in the context's syntax.
    unchecked: usize,
    /// A checked leaf whose end lies inside one, with no place outside one
    /// near enough to move it to: its span is Deep and no parse crosses it.
    unsafe_end: bool,
    children: Vec<Node>,
}
romacs_snapshot::snapshot_struct!(Node { chars, spans, unchecked, unsafe_end, children });

impl Node {
    fn leaf(chars: usize) -> Self {
        Self { chars, unchecked: 1, ..Self::default() }
    }
    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
    /// Recount an inner node's CHARS and UNCHECKED from its children: none
    /// left, it is empty, and its parent drops it.
    fn sum(&mut self) {
        self.chars = self.children.iter().map(|child| child.chars).sum();
        self.unchecked = self.children.iter().map(|child| child.unchecked).sum();
    }
    /// Nodes of at most FANOUT of NODES each, recounted.
    fn group(nodes: Vec<Node>) -> Vec<Node> {
        let mut nodes = nodes.into_iter().peekable();
        let mut groups = Vec::new();
        while nodes.peek().is_some() {
            let mut node = Node { children: nodes.by_ref().take(FANOUT).collect(), ..Node::default() };
            node.sum();
            groups.push(node);
        }
        groups
    }
}

/// Counts of what the index did, for `romacs-syntax-summaries-stats'.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    /// Parses served, and parses the plain scanner took, by reason.
    pub served: u64,
    pub declined: [u64; Decline::COUNT],
    /// Leaf summaries scanned, leaves scanned into a state with no summary
    /// applied, summaries composed.
    pub leaf_scans: u64,
    pub plain_leaves: u64,
    pub composed: u64,
    /// Contexts changed.
    pub switches: u64,
}
romacs_snapshot::snapshot_struct!(Stats { served, declined, leaf_scans, plain_leaves, composed, switches });

/// Why a parse took the plain scanner.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Decline {
    /// `romacs-inhibit-syntax-summaries'.
    Inhibited,
    /// TARGETDEPTH, STOPBEFORE or COMMENTSTOP.
    Arguments,
    /// Too few leaves between FROM and TO to pay.
    Short,
    /// `syntax-table' properties honoured but not yet propertized past TO.
    Properties,
    /// A state no key resumes as the scan that made it would continue: an
    /// OLDSTATE both in a string and in a comment, or a nested comment an
    /// escaped end used up (forw_comment then never ends it).
    Unresumable,
}
impl Decline {
    pub(crate) const COUNT: usize = 5;
    pub(crate) const ALL: [Decline; Self::COUNT] = [Self::Inhibited, Self::Arguments, Self::Short, Self::Properties, Self::Unresumable];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Inhibited => "inhibited",
            Self::Arguments => "arguments",
            Self::Short => "short",
            Self::Properties => "properties",
            Self::Unresumable => "unresumable",
        }
    }
}

/// Whether a state at a leaf's end is one a key resumes: the scan that
/// made it reads on as one in its key's control state would. Not a
/// `spent' comment, nor a Lisp state both in a comment and in a string,
/// whose stale string GNU's resume keeps.
fn resumable(state: &ParseState) -> bool {
    let spent = state.continuation.is_some_and(|c| c.spent);
    !spent && (state.incomment == 0 || state.instring < 0)
}

/// The character position of character offset AT.
fn position(at: usize) -> i64 {
    i64::try_from(at).expect("a text's length fits a position") + 1
}
/// The character offset of position AT (at least 1).
fn offset(at: i64) -> usize {
    usize::try_from(at - 1).expect("a position in the text")
}

/// A text's summaries: a B-tree over its leaves, absent until a parse
/// needs it.
pub(crate) struct SyntaxIndex {
    /// Characters in a leaf it cuts.
    leaf: usize,
    root: Option<Node>,
    context: Option<Context>,
    pub(crate) stats: Stats,
}
romacs_snapshot::snapshot_struct!(SyntaxIndex { leaf, root, context, stats });
impl Default for SyntaxIndex {
    fn default() -> Self {
        Self { leaf: LEAF, root: None, context: None, stats: Stats::default() }
    }
}
impl Clone for SyntaxIndex {
    /// A copy of a text starts without summaries, as a cache does.
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl SyntaxIndex {
    /// Whether a parse of CHARS characters is long enough to serve.
    pub(crate) fn pays(chars: i64) -> bool {
        usize::try_from(chars).is_ok_and(|chars| chars >= 4 * LEAF)
    }

    /// Leaves of up to LEAF characters over CHARS.
    fn cut(leaf: usize, chars: usize) -> Vec<Node> {
        (0..chars).step_by(leaf).map(|at| Node::leaf(leaf.min(chars - at))).collect()
    }
    /// A tree over LEAVES.
    fn build(mut level: Vec<Node>) -> Node {
        if level.is_empty() {
            return Node::leaf(0);
        }
        while level.len() > 1 {
            level = Node::group(level);
        }
        level.pop().expect("a root")
    }

    /// The text's characters START..END (offsets) became INSERTED
    /// characters.
    pub(crate) fn edit(&mut self, start: usize, end: usize, inserted: usize) {
        let Some(root) = &mut self.root else { return };
        if end > root.chars || start > end {
            // A text the index does not describe: the next parse cuts it anew.
            self.root = None;
            return;
        }
        let total = root.chars;
        let leaf = self.leaf;
        let was_leaf = root.is_leaf();
        edit_node(root, 0, &Edit { start, end, inserted, at_end: start == total }, leaf);
        if root.children.len() > FANOUT {
            let children = std::mem::take(&mut root.children);
            *root = Self::build(Node::group(children));
        } else if !was_leaf && root.is_leaf() {
            // Every leaf went with the text.
            *root = Node::leaf(0);
        } else if root.is_leaf() && root.chars > 2 * leaf {
            *root = Self::build(Self::cut(leaf, root.chars));
        }
        self.uncheck_before(start);
    }

    /// The `syntax-table' property of the characters START..END changed:
    /// the leaves holding them lose their summaries.
    pub(crate) fn touch(&mut self, start: usize, end: usize) {
        let Some(root) = &mut self.root else { return };
        if start < end && end <= root.chars {
            touch_node(root, 0, start, end);
        }
        self.uncheck_before(start);
    }

    /// The leaf ending at START, whose end pairs its last character with
    /// the one at START, checks its end again.
    fn uncheck_before(&mut self, start: usize) {
        if let Some(root) = &mut self.root && start > 0 && start <= root.chars {
            update(root, start - 1, false, &mut |leaf| leaf.unchecked = 1);
        }
    }

    /// Every summary goes and every leaf end is checked again.
    pub(crate) fn forget(&mut self) {
        if let Some(root) = &mut self.root {
            forget(root);
        }
    }

    /// Check every unchecked leaf end: one inside a two-character comment
    /// start moves to the first place after it that is not, if one lies
    /// within a leaf's length, the characters between leaving the leaves
    /// after; else the end stays, unsafe.
    fn check_ends<T: Text>(&mut self, scanner: &Scanner<'_, T>) {
        let leaf_chars = self.leaf;
        let Some(root) = &mut self.root else { return };
        let text = scanner.text;
        let syntax = |at: usize| text.syntax(text.pos(position(at)));
        // Whether a scan can stop at offset AT and resume exactly.
        let total = root.chars;
        let safe = |at: usize| at == 0 || at >= total
            || !(crate::syntax_scan::comstart_first(syntax(at - 1)) && crate::syntax_scan::comstart_second(syntax(at)));
        while let Some((start, chars)) = first_unchecked(root, 0) {
            let end = start + chars;
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
            update(root, start, unsafe_end, &mut |leaf| {
                leaf.unchecked = 0;
                leaf.unsafe_end = unsafe_end;
            });
        }
    }

    /// NODE's summary entered in KEY; NODE starts at character offset
    /// START.
    fn span<T: Text>(counts: &mut Stats, scanner: &mut Scanner<'_, T>, node: &mut Node, start: usize, key: Key) -> Span {
        if let Some((_, span)) = node.spans.iter().find(|(k, _)| *k == key) {
            return span.clone();
        }
        let span = if node.is_leaf() && node.unsafe_end {
            Span::Deep
        } else if node.is_leaf() {
            counts.leaf_scans += 1;
            summary::summarize(scanner, scanner.text.pos(position(start)), position(start + node.chars), key)
        } else {
            let (first, rest) = node.children.split_first_mut().expect("an inner node has children");
            let mut span = Self::span(counts, scanner, first, start, key);
            let mut at = start + first.chars;
            for child in rest {
                let Span::Exact(so_far) = &span else { break };
                let next = Self::span(counts, scanner, child, at, so_far.exit());
                counts.composed += 1;
                span = u32::try_from(at - start).map_or(Span::Deep, |chars| summary::compose(&span, &next, chars));
                at += child.chars;
            }
            span
        };
        node.spans.insert(0, (key, span.clone()));
        node.spans.truncate(KEYS);
        span
    }

    /// Carry STATE across the characters FROM..TO (offsets, both leaf
    /// boundaries) under NODE, which starts at START. False where a state
    /// between leaves is not `resumable'.
    fn fold<T: Text>(counts: &mut Stats, scanner: &mut Scanner<'_, T>, state: &mut ParseState, node: &mut Node,
                     start: usize, (from, to): (usize, usize)) -> R<bool> {
        let end = start + node.chars;
        if end <= from || to <= start {
            return Ok(true);
        }
        if from <= start && end <= to {
            if let Span::Exact(span) = Self::span(counts, scanner, node, start, Key::of(state))
                && summary::apply(state, &span, position(start)) {
                return Ok(resumable(state));
            }
            if node.is_leaf() {
                if node.unsafe_end {
                    return Ok(false);
                }
                counts.plain_leaves += 1;
                summary::scan(scanner, state, position(start), position(end))?;
                return Ok(resumable(state));
            }
        }
        let mut at = start;
        for child in &mut node.children {
            if !Self::fold(counts, scanner, state, child, at, (from, to))? {
                return Ok(false);
            }
            at += child.chars;
        }
        Ok(true)
    }

    /// `parse-partial-sexp' from FROM to TO (character positions) in a
    /// text of CHARS characters, starting in STATE: what the scanner alone
    /// gives, reading only the two leaves FROM and TO are in. False, with
    /// STATE as it was, where the summaries do not serve it: too few leaves
    /// lie between, or a state on the way is not `resumable'.
    pub(crate) fn parse<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, chars: usize, context: Context,
                                 from: i64, to: i64, state: &mut ParseState) -> R<bool> {
        if self.context.as_ref() != Some(&context) {
            if self.context.is_some() {
                self.stats.switches += 1;
            }
            if let Some(root) = &mut self.root {
                forget(root);
            }
            self.context = Some(context);
        }
        if self.root.as_ref().is_none_or(|root| root.chars != chars) {
            self.root = Some(Self::build(Self::cut(self.leaf, chars)));
        }
        self.check_ends(scanner);
        let Self { root: Some(root), stats, .. } = self else { unreachable!("built above") };
        let (first_start, first_chars, first_unsafe) = locate(root, offset(from));
        let (last_start, ..) = locate(root, offset((to - 1).max(from)));
        let first_end = first_start + first_chars;
        if last_start <= first_end {
            stats.declined[Decline::Short as usize] += 1;
            return Ok(false);
        }
        if !resumable(state) || first_unsafe {
            stats.declined[Decline::Unresumable as usize] += 1;
            return Ok(false);
        }
        let mut carried = state.clone();
        // The caller's state resumes as GNU's does; every later step continues it.
        scanner.scan_sexps_forward(&mut carried, scanner.text.pos(from), position(first_end), i64::MIN, false, 0)?;
        if !resumable(&carried) || !Self::fold(stats, scanner, &mut carried, root, 0, (first_end, last_start))? {
            stats.declined[Decline::Unresumable as usize] += 1;
            return Ok(false);
        }
        summary::scan(scanner, &mut carried, position(last_start), to)?;
        *state = carried;
        stats.served += 1;
        Ok(true)
    }

    /// Bytes the index holds, for memory accounting.
    pub(crate) fn storage_bytes(&self) -> usize {
        fn bytes(node: &Node) -> usize {
            size_of::<Node>()
                + node.spans.capacity() * size_of::<(Key, Span)>()
                + node.spans.iter().map(|(_, span)| if let Span::Exact(summary) = span { summary.opened_bytes() } else { 0 }).sum::<usize>()
                + node.children.iter().map(bytes).sum::<usize>()
                + (node.children.capacity() - node.children.len()) * size_of::<Node>()
        }
        self.root.as_ref().map_or(0, bytes)
    }
    /// How many leaves it has cut the text into.
    pub(crate) fn leaves(&self) -> usize {
        fn leaves(node: &Node) -> usize {
            if node.is_leaf() { 1 } else { node.children.iter().map(leaves).sum() }
        }
        self.root.as_ref().map_or(0, leaves)
    }
}

/// An edit of a text: START..END became INSERTED characters, inserted in
/// the leaf holding START, or the last leaf when AT_END.
#[derive(Clone, Copy)]
struct Edit {
    start: usize,
    end: usize,
    inserted: usize,
    at_end: bool,
}

/// EDIT applied under NODE, which starts at START: leaves it touches keep
/// what lies outside it, the one at its start gains what it inserted, and
/// each is unchecked and loses its summaries, as does every node above it.
/// Empty leaves go; leaves past twice LEAF and nodes past FANOUT split.
fn edit_node(node: &mut Node, start: usize, edit: &Edit, leaf: usize) {
    let end = start + node.chars;
    if node.is_leaf() {
        let before = edit.start.clamp(start, end) - start;
        let after = end - edit.end.clamp(start, end);
        let receives = (start <= edit.start && edit.start < end) || edit.at_end;
        node.chars = before + after + if receives { edit.inserted } else { 0 };
        node.spans.clear();
        node.unchecked = 1;
        node.unsafe_end = false;
        return;
    }
    let mut at = start;
    let count = node.children.len();
    let mut touched = false;
    for (i, child) in node.children.iter_mut().enumerate() {
        let child_end = at + child.chars;
        let holds_start = (at <= edit.start && edit.start < child_end) || (edit.at_end && i + 1 == count);
        if holds_start || (at < edit.end && edit.start < child_end) {
            let at_end = edit.at_end && i + 1 == count;
            edit_node(child, at, &Edit { at_end, ..*edit }, leaf);
            touched = true;
        }
        at = child_end;
    }
    if !touched {
        return;
    }
    let children = std::mem::take(&mut node.children);
    node.children = children.into_iter().filter(|child| child.chars > 0).flat_map(|child| {
        if child.is_leaf() && child.chars > 2 * leaf {
            SyntaxIndex::cut(leaf, child.chars)
        } else if child.children.len() > FANOUT {
            Node::group(child.children)
        } else {
            vec![child]
        }
    }).collect();
    node.spans.clear();
    node.sum();
}

/// Apply F to the leaf holding offset AT under NODE, then recount the
/// path, dropping its summaries when CLEAR, and remove the leaf if F
/// emptied it (and any node that loses its last child).
fn update(node: &mut Node, at: usize, clear: bool, f: &mut impl FnMut(&mut Node)) {
    if clear {
        node.spans.clear();
    }
    if node.is_leaf() {
        f(node);
        return;
    }
    let (last, mut start, mut i) = (node.children.len() - 1, 0, 0);
    while i < last && at >= start + node.children[i].chars {
        start += node.children[i].chars;
        i += 1;
    }
    update(&mut node.children[i], at - start, clear, f);
    if node.children[i].chars == 0 {
        node.children.remove(i);
    }
    node.sum();
}

/// The first leaf under NODE (at START) with an unchecked end: its start
/// and characters.
fn first_unchecked(node: &Node, start: usize) -> Option<(usize, usize)> {
    if node.unchecked == 0 {
        return None;
    }
    if node.is_leaf() {
        return Some((start, node.chars));
    }
    let mut at = start;
    for child in &node.children {
        if child.unchecked > 0 {
            return first_unchecked(child, at);
        }
        at += child.chars;
    }
    None
}

/// The leaves holding the characters START..END under NODE (at NODE_START)
/// lose their summaries and their checked ends, and so do their ancestors.
fn touch_node(node: &mut Node, node_start: usize, start: usize, end: usize) {
    node.spans.clear();
    if node.is_leaf() {
        node.unchecked = 1;
        node.unsafe_end = false;
        return;
    }
    let mut at = node_start;
    for child in &mut node.children {
        if at < end && start < at + child.chars {
            touch_node(child, at, start, end);
        }
        at += child.chars;
    }
    node.sum();
}

/// The leaf holding offset AT under ROOT (the last, past the end): its
/// start, characters and whether its end is unsafe.
fn locate(root: &Node, at: usize) -> (usize, usize, bool) {
    let (mut node, mut start) = (root, 0);
    while !node.is_leaf() {
        let last = node.children.len() - 1;
        let mut i = 0;
        while i < last && at >= start + node.children[i].chars {
            start += node.children[i].chars;
            i += 1;
        }
        node = &node.children[i];
    }
    (start, node.chars, node.unsafe_end)
}

/// Drop every summary under NODE and uncheck every leaf end.
fn forget(node: &mut Node) {
    node.spans.clear();
    if node.is_leaf() {
        node.unchecked = 1;
        node.unsafe_end = false;
    } else {
        node.children.iter_mut().for_each(forget);
        node.sum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax_scan::tests::{random_table, random_text, Plain, Rng, ALPHABET};
    use crate::syntax_scan::{Options, ParseState, Pos, Scanner};
    use std::collections::BTreeMap;

    /// PLAIN with a syntax per position where PROPERTIES has one, as
    /// `syntax-table' properties honoured.
    struct Propertied<'a, F: Fn(u8) -> i32> {
        plain: &'a Plain<F>,
        properties: &'a [Option<i32>],
    }
    impl<F: Fn(u8) -> i32> crate::syntax_scan::Text for Propertied<'_, F> {
        fn char_at(&self, at: Pos) -> u32 { self.plain.char_at(at) }
        fn inc(&self, at: Pos) -> Pos { self.plain.inc(at) }
        fn dec(&self, at: Pos) -> Pos { self.plain.dec(at) }
        fn pos(&self, charpos: i64) -> Pos { self.plain.pos(charpos) }
        fn syntax(&self, at: Pos) -> i32 {
            self.properties.get(at.byte).copied().flatten().unwrap_or_else(|| self.plain.syntax(at))
        }
        fn table_syntax(&self, c: u32) -> i32 { self.plain.table_syntax(c) }
    }

    /// What Lisp's list form keeps of a scanner's state: no continuation.
    fn listed(state: &ParseState) -> ParseState {
        ParseState { continuation: None, maxdepth: 0, mindepth: 0, ..state.clone() }
    }

    /// Served parses, after random edits, in random tables, from the start
    /// or from a state Lisp gives, are the plain scanner's, field for field.
    #[test]
    fn summaries_serve_what_the_scanner_gives() {
        let mut rng = Rng(0x005e_ed0f_5ca1_ab1e);
        let mut stats = Stats::default();
        for case in 0..300 {
            let mut table = random_table(&mut rng);
            let len = rng.below(400);
            let mut text = random_text(&mut rng, len);
            let mut properties: Vec<Option<i32>> = vec![None; len];
            let options = Options { comment_end_can_be_escaped: rng.chance(50), ..Options::default() };
            let mut index = SyntaxIndex { leaf: 1 + rng.below(12), ..SyntaxIndex::default() };
            let mut stamp = u64::try_from(case).expect("a case") * 1000;
            for round in 0..12 {
                if round > 0 && rng.chance(80) {
                    let start = rng.below(text.len() + 1);
                    let end = (start + rng.below(6)).min(text.len());
                    let len = rng.below(5);
                    let inserted = random_text(&mut rng, len);
                    index.edit(start, end, inserted.len());
                    properties.splice(start..end, vec![None; inserted.len()]);
                    text.splice(start..end, inserted);
                }
                if rng.chance(40) && !text.is_empty() {
                    // A `syntax-table' property put or removed over a range.
                    let start = rng.below(text.len());
                    let end = (start + 1 + rng.below(8)).min(text.len());
                    let syntax = rng.chance(70).then(|| table[usize::from(ALPHABET[rng.below(ALPHABET.len())])]);
                    properties[start..end].fill(syntax);
                    index.touch(start, end);
                }
                if rng.chance(10) {
                    table = random_table(&mut rng);
                    stamp += 1;
                }
                let plain = Plain { text: text.clone(), table: |c: u8| table[usize::from(c)] };
                let plain = Propertied { plain: &plain, properties: &properties };
                let ppss = BTreeMap::new();
                let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
                let context = Context { table: 1, writes: stamp, escaped_ends: options.comment_end_can_be_escaped, properties: true };
                for _ in 0..6 {
                    let from = if rng.chance(50) { 1 } else { position(rng.below(text.len() + 1)) };
                    let to = position(offset(from) + rng.below(text.len() - offset(from) + 1));
                    let mut start = ParseState::default();
                    if from > 1 && rng.chance(50) {
                        scanner.scan_sexps_forward(&mut start, plain.pos(1), from, i64::MIN, false, 0).unwrap();
                        start = listed(&start);
                    }
                    let mut expected = start.clone();
                    scanner.scan_sexps_forward(&mut expected, plain.pos(from), to, i64::MIN, false, 0).unwrap();
                    let mut served = start.clone();
                    if !index.parse(&mut scanner, text.len(), context.clone(), from, to, &mut served).unwrap() {
                        assert_eq!(served, start, "a parse not served leaves its state");
                        scanner.scan_sexps_forward(&mut served, plain.pos(from), to, i64::MIN, false, 0).unwrap();
                    }
                    assert_eq!(served, expected, "case {case} round {round}: {:?} {from}..{to} from {start:?}, leaf {}, properties {properties:?}",
                               String::from_utf8_lossy(&text), index.leaf);
                }
            }
            let s = index.stats;
            (stats.served, stats.leaf_scans, stats.composed, stats.plain_leaves) =
                (stats.served + s.served, stats.leaf_scans + s.leaf_scans, stats.composed + s.composed, stats.plain_leaves + s.plain_leaves);
        }
        // The summaries did the work, not the scans at the ends.
        assert!(stats.served > 1000 && stats.composed > 1000 && stats.leaf_scans > 1000, "{stats:?}");
        let _ = ALPHABET;
    }

    /// An edit drops the summaries of the leaf it touched and its
    /// ancestors, and no others.
    #[test]
    fn an_edit_rescans_one_leaf() {
        let table = [crate::syntax_scan::SWORD; 128];
        let text: Vec<u8> = b"abc (def) ghi\n".iter().copied().cycle().take(4000).collect();
        let plain = Plain { text: text.clone(), table: |c: u8| match c { b'(' => crate::syntax_scan::SOPEN, b')' => crate::syntax_scan::SCLOSE, b' ' | b'\n' => 0, _ => table[usize::from(c)] } };
        let ppss = BTreeMap::new();
        let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
        let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
        let mut index = SyntaxIndex { leaf: 100, ..SyntaxIndex::default() };
        index.parse(&mut scanner, text.len(), context.clone(), 1, 4001, &mut ParseState::default()).unwrap();
        let cold = index.stats.leaf_scans;
        assert!(cold >= 38, "{cold}");
        // Replace one character near the start with another of the same class.
        index.edit(1, 2, 1);
        index.parse(&mut scanner, text.len(), context, 1, 4001, &mut ParseState::default()).unwrap();
        assert_eq!(index.stats.leaf_scans - cold, 0, "the edited leaf is the first, scanned plain at the start");
    }

    /// A text erased and filled again is cut into leaves again, and served.
    #[test]
    fn a_refilled_text_is_served_again() {
        let text: Vec<u8> = b"(a b) c\n".iter().copied().cycle().take(600).collect();
        let plain = Plain { text: text.clone(), table: |c: u8| match c { b'(' => crate::syntax_scan::SOPEN, b')' => crate::syntax_scan::SCLOSE, b' ' | b'\n' => 0, _ => crate::syntax_scan::SWORD } };
        let ppss = BTreeMap::new();
        let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
        let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
        let mut index = SyntaxIndex { leaf: 20, ..SyntaxIndex::default() };
        assert!(index.parse(&mut scanner, 600, context.clone(), 1, 601, &mut ParseState::default()).unwrap());
        index.edit(0, 600, 0);
        index.edit(0, 0, 600);
        assert!(index.leaves() >= 30, "{} leaves", index.leaves());
        assert!(index.parse(&mut scanner, 600, context, 1, 601, &mut ParseState::default()).unwrap());
    }

    /// A run of a two-character comment starter longer than a leaf leaves
    /// an unsafe end, and a parse across it is not served.
    #[test]
    fn a_long_comment_starter_run_is_not_crossed() {
        // `-' is both characters of "--", a comment starter.
        let table = |c: u8| match c { b'-' => 1 | 1 << 16 | 1 << 17, b'\n' => crate::syntax_scan::SENDCOMMENT, _ => crate::syntax_scan::SWORD };
        let text: Vec<u8> = [b"ab\n".repeat(20), vec![b'-'; 300], b"cd\n".repeat(20)].concat();
        let plain = Plain { text: text.clone(), table };
        let ppss = BTreeMap::new();
        let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
        let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
        let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
        let end = position(text.len());
        let mut served = ParseState::default();
        assert!(!index.parse(&mut scanner, text.len(), context, 1, end, &mut served).unwrap());
        assert_eq!(index.stats.declined[Decline::Unresumable as usize], 1);
    }
}
