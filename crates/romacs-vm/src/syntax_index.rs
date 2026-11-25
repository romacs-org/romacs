//! The forward syntax summaries of a text (`crate::syntax_summary`), kept in
//! a tree beside it: leaves of about a leaf's characters ([`LEAF`]) under
//! a B-tree, each node holding its span's summary for the keys it was
//! entered in, and for the level counts where the span closes past the
//! bottom level or reaches GNU's cap. A parse over many leaves scans only
//! its two partial ends and applies O(log n) summaries between. An edit
//! changes the leaves it touches and the nodes above them, nothing else:
//! every position a summary holds is relative to its node, and a split or
//! a removal stays inside the edited path. A forward list scan folds the
//! same tree at leaf boundaries (`forward`).
//!
//! Summaries read the text, the syntax table and `comment-end-can-be-escaped'
//! and nothing else (scan_sexps_forward's inputs), so each is valid for one
//! [`Context`]. The index keeps up to [`CONTEXTS`] of them, the one least
//! recently used replaced first, and each entry names its context: a switch
//! to a known one forgets nothing. A context's char-table write count is
//! decided in one place, `prepare`; when it moved, that context's entries
//! go. Where `syntax-table' properties are honoured, summaries read them as
//! the scanner does, and a change of the property drops the summaries of
//! the leaves it touches (`TextBody::sync_syntax_properties`).
//!
//! One partition of leaves serves every context. A leaf prefers to end
//! where both scanners can stop and resume exactly, which each context
//! checks with its own table: a summary under a context needs that
//! context's check. An edit or a property change leaves an end unchecked
//! by all; the first check moves a cut after a possible split token at most
//! a leaf's length on, and only a cut no context has checked moves. A cut
//! with no such place stays unsafe for that context.
//!
//! It holds no Lisp value: a derived cache, neither a root nor visited by
//! the collector, and a copy of a text starts without one.
use crate::syntax_scan::{self as scan, ParseState, R, Scanner, Text};
use crate::syntax_summary::{self as summary, Key, Span, Summary};
use crate::syntax_summary::lists::{self as list_summary, Exact, Excursion, Mode};

/// Characters in a leaf the index cuts; a leaf grown past twice this splits.
pub(crate) const LEAF: usize = 1024;
/// Children a node holds before it splits.
const FANOUT: usize = 16;
/// Slots for the entries a node keeps, of every context and kind, the
/// least recently used given up first. A parse key whose span closes past
/// the bottom level can take two, general and exact.
const KEYS: usize = 8;
/// Contexts an index keeps entries for.
pub(crate) const CONTEXTS: usize = 4;
/// A bit for each context slot.
const EVERY: u8 = (1 << CONTEXTS) - 1;
/// What a node keeps a summary under: the key its span is entered in, and
/// the count of open levels the summary is exact for, or none for the
/// general one.
type Slot = (Key, Option<u32>);

/// What a node keeps, under the context slot it was made in: a parse
/// summary, or a forward list span's excursion or exact summary, none
/// where the span has none.
#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
enum Entry {
    Parse(u8, Slot, Span),
    Excursion(u8, Mode, Option<Excursion>),
    Exact(u8, list_summary::Key, Option<Exact>),
}
impl Entry {
    fn context(&self) -> u8 {
        match self {
            Self::Parse(context, ..) | Self::Excursion(context, ..) | Self::Exact(context, ..) => *context,
        }
    }
    /// Its kind, as `Stats::evictions' counts them.
    fn kind(&self) -> usize {
        match self {
            Self::Parse(..) => 0,
            Self::Exact(..) => 1,
            Self::Excursion(..) => 2,
        }
    }
}

/// What summaries depend on besides the text: the syntax table, by its
/// serial (`Vm::char_table_serial`) and the char-table writes so far (`crate::char_table::writes`:
/// any write to any table, its parents' included, renews it),
/// `comment-end-can-be-escaped', and whether `syntax-table' properties are
/// honoured. All but the writes name it; the writes say whether what was
/// kept under it still holds.
#[derive(Clone, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Context {
    pub table: u64,
    pub writes: u64,
    pub escaped_ends: bool,
    pub properties: bool,
}

/// A leaf (no children) or an inner node of the tree.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
struct Node {
    chars: usize,
    /// Its entries, the one used last first.
    spans: Vec<Entry>,
    /// The context slots that have not checked the end of a leaf below (a
    /// leaf: its own), a bit each.
    unchecked: u8,
    /// A leaf's: the context slots that found no safe cut close enough to
    /// move its end to.
    unsafe_ends: u8,
    children: Vec<Node>,
}

impl Node {
    fn leaf(chars: usize) -> Self {
        Self { chars, unchecked: EVERY, ..Self::default() }
    }
    fn parse_span(&self, at: usize) -> &Span {
        let Entry::Parse(_, _, span) = &self.spans[at] else { unreachable!("a parse entry") };
        span
    }
    /// The entry at AT, used now: it moves first, where it is then.
    fn hit(&mut self, at: usize) -> usize {
        self.spans[..=at].rotate_right(1);
        0
    }
    /// Keep ENTRY first, giving up the least recently used where every
    /// slot is taken: the kind of the one given up.
    fn remember(&mut self, entry: Entry) -> Option<usize> {
        let evicted = (self.spans.len() >= KEYS).then(|| self.spans.pop().map(|old| old.kind())).flatten();
        self.spans.reserve_exact(1);
        self.spans.insert(0, entry);
        evicted
    }
    /// Drop the entries of context slot CONTEXT.
    fn drop_entries(&mut self, context: u8) {
        self.spans.retain(|entry| entry.context() != context);
    }

    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
    /// Recount an inner node's CHARS and UNCHECKED from its children: none
    /// left, it is empty, and its parent drops it.
    fn sum(&mut self) {
        self.chars = self.children.iter().map(|child| child.chars).sum();
        self.unchecked = self.children.iter().fold(0, |mask, child| mask | child.unchecked);
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
#[derive(Clone, Copy, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Stats {
    /// Parses served, and parses the plain scanner took, by reason.
    pub served: u64,
    pub declined: [u64; Decline::PARSE.len()],
    /// Forward list scan segments (a scan stopped for Lisp counts again when
    /// it goes on) that folded a span, and those that did not, by reason.
    pub list_served: u64,
    pub list_declined: [u64; Decline::COUNT],
    /// Leaf summaries scanned, leaves scanned into a state with no summary
    /// applied, summaries composed.
    pub leaf_scans: u64,
    pub plain_leaves: u64,
    pub composed: u64,
    /// Spans a forward list scan folded instead of reading them, by kind
    /// (positive excursion, exact entry), and the characters they held.
    pub folds: [u64; 2],
    pub skipped: u64,
    /// Entries given up for a slot, by kind: parse, exact, excursion.
    pub evictions: [u64; 3],
    /// Scans in another context than the last; contexts given up for a new
    /// one; contexts whose char-table write count moved, their entries gone.
    pub switches: u64,
    pub context_evictions: u64,
    pub context_invalidations: u64,
    /// Takes of the property change log that found it a hull.
    pub log_overflows: u64,
}

/// Why a scan took the plain scanner.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Decline {
    /// `romacs-inhibit-syntax-summaries'.
    Inhibited,
    /// TARGETDEPTH, STOPBEFORE or COMMENTSTOP.
    Arguments,
    /// Too few leaves between FROM and TO to pay.
    Short,
    /// `syntax-table' properties honoured but not yet propertized past TO;
    /// for a list scan, a leaf it reached that the frontier lies inside.
    Properties,
    /// A state no key resumes as the scan that made it would continue: an
    /// OLDSTATE both in a string and in a comment, or a nested comment an
    /// escaped end used up (forw_comment then never ends it).
    Unresumable,
    /// A quit poll stopped the list scan.
    Quit,
}
impl Decline {
    const COUNT: usize = 6;
    pub(crate) const PARSE: [Self; 5] = [Self::Inhibited, Self::Arguments, Self::Short, Self::Properties, Self::Unresumable];
    /// A list scan's: Short, it reached no leaf it could fold.
    pub(crate) const LISTS: [Self; 4] = [Self::Inhibited, Self::Short, Self::Properties, Self::Quit];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Inhibited => "inhibited",
            Self::Arguments => "arguments",
            Self::Short => "short",
            Self::Properties => "properties",
            Self::Unresumable => "unresumable",
            Self::Quit => "quit",
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
#[derive(romacs_snapshot::Snapshot)]
pub(crate) struct SyntaxIndex {
    /// Characters in a leaf it cuts.
    leaf: usize,
    root: Option<Node>,
    /// The contexts its entries name, by slot, each with when it was last
    /// used; the slot used last.
    contexts: [Option<(Context, u64)>; CONTEXTS],
    tick: u64,
    current: Option<u8>,
    pub(crate) stats: Stats,
}
impl Default for SyntaxIndex {
    fn default() -> Self {
        Self { leaf: LEAF, root: None, contexts: Default::default(), tick: 0, current: None, stats: Stats::default() }
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

    /// The slot of CONTEXT, the tree cut over CHARS characters and its
    /// ends checked for that slot: the one place that decides what kept
    /// entries hold. A context whose write count moved loses its entries; a
    /// new one takes a free slot or the least recently used one's.
    fn prepare<T: Text>(&mut self, scanner: &Scanner<'_, T>, chars: usize, context: Context) -> u8 {
        let same = |kept: &Context| kept.table == context.table && kept.escaped_ends == context.escaped_ends
            && kept.properties == context.properties;
        let found = self.contexts.iter().position(|slot| slot.as_ref().is_some_and(|(kept, _)| same(kept)));
        let slot = match found {
            Some(slot) if self.contexts[slot].as_ref().is_some_and(|(kept, _)| kept.writes == context.writes) => slot,
            Some(slot) => {
                self.stats.context_invalidations += 1;
                self.reset_context(slot);
                slot
            }
            None => {
                let slot = (0..CONTEXTS).min_by_key(|&slot| self.contexts[slot].as_ref().map_or(0, |(_, used)| *used + 1))
                    .expect("context slots");
                if self.contexts[slot].is_some() {
                    self.stats.context_evictions += 1;
                    self.reset_context(slot);
                }
                slot
            }
        };
        self.tick += 1;
        self.contexts[slot] = Some((context, self.tick));
        let slot = u8::try_from(slot).expect("few contexts");
        if self.current.is_some_and(|current| current != slot) {
            self.stats.switches += 1;
        }
        self.current = Some(slot);
        if self.root.as_ref().is_none_or(|root| root.chars != chars) {
            self.root = Some(Self::build(Self::cut(self.leaf, chars)));
        }
        self.check_ends(scanner, slot);
        slot
    }

    /// Context slot SLOT keeps nothing: its entries go, and every leaf end
    /// is unchecked for it.
    fn reset_context(&mut self, slot: usize) {
        fn reset(node: &mut Node, slot: u8) {
            node.drop_entries(slot);
            if node.is_leaf() {
                node.unchecked |= 1 << slot;
                node.unsafe_ends &= !(1 << slot);
            } else {
                node.children.iter_mut().for_each(|child| reset(child, slot));
                node.sum();
            }
        }
        if let Some(root) = &mut self.root {
            reset(root, u8::try_from(slot).expect("few contexts"));
        }
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
            update(root, start - 1, false, &mut |leaf| leaf.unchecked = EVERY);
        }
    }

    /// Every summary goes and every leaf end is checked again.
    pub(crate) fn forget(&mut self) {
        if let Some(root) = &mut self.root {
            forget(root);
        }
    }

    /// Check for context slot SLOT each leaf end it has not checked. A cut
    /// no context has checked moves past a possible split token, at most a
    /// leaf's length on, taking those characters from the next leaves; one
    /// with no safe cut in range, or that another context checked, is
    /// marked unsafe for SLOT where it is.
    fn check_ends<T: Text>(&mut self, scanner: &Scanner<'_, T>, slot: u8) {
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
        let bit = 1 << slot;
        while let Some((start, chars, unchecked, was_unsafe)) = first_unchecked(root, 0, bit) {
            let end = start + chars;
            if unchecked != EVERY {
                // Unsafe after an edit beside it: what the path keeps for SLOT,
                // made while it was safe, is not its span's.
                let unsafe_end = !safe(end);
                update(root, start, unsafe_end, &mut |leaf| {
                    leaf.unchecked &= !bit;
                    leaf.unsafe_ends |= if unsafe_end { bit } else { 0 };
                });
                continue;
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
            update(root, start, unsafe_end || was_unsafe, &mut |leaf| {
                leaf.unchecked = EVERY & !bit;
                leaf.unsafe_ends = if unsafe_end { bit } else { 0 };
            });
        }
    }

    /// NODE's summary entered in KEY, for a state with LEVELS levels open:
    /// the general one where it fits that count, else the one exact for
    /// it; with no count, the general one. A kept one first; a general one
    /// is made before an exact one, since it outlives an edit before NODE
    /// that changes the count. NODE starts at character offset START.
    fn span<'a, T: Text>(counts: &mut Stats, scanner: &mut Scanner<'_, T>, (node, start, context): (&'a mut Node, usize, u8),
                         key: Key, levels: Option<u32>) -> &'a Span {
        let wanted = |span: &Span| levels.is_none_or(|open| matches!(span, Span::Exact(summary) if summary.fits(open as usize)));
        let kept = |node: &Node, slot: Slot| node.spans.iter()
            .position(|entry| matches!(entry, Entry::Parse(owner, kept, _) if *owner == context && *kept == slot));
        let general = kept(node, (key, None));
        if let Some(at) = general.filter(|&at| wanted(node.parse_span(at))) {
            let at = node.hit(at);
            return node.parse_span(at);
        }
        if let Some(at) = levels.and_then(|_| kept(node, (key, levels))) {
            let at = node.hit(at);
            return node.parse_span(at);
        }
        if general.is_none() && wanted(Self::make(counts, scanner, (node, start, context), (key, None))) {
            return node.parse_span(0);
        }
        Self::make(counts, scanner, (node, start, context), (key, levels))
    }

    /// NODE's summary for SLOT, made and kept first of those it keeps: a
    /// leaf's by its scan, an inner node's from its children's.
    fn make<'a, T: Text>(counts: &mut Stats, scanner: &mut Scanner<'_, T>, (node, start, context): (&'a mut Node, usize, u8),
                         slot: Slot) -> &'a Span {
        let span = if node.is_leaf() && node.unsafe_ends & (1 << context) != 0 {
            Span::Deep
        } else if node.is_leaf() {
            counts.leaf_scans += 1;
            summary::summarize(scanner, scanner.text.pos(position(start)), position(start + node.chars), slot.0, slot.1)
        } else {
            Self::composed(counts, scanner, (node, start, context), slot).map_or(Span::Deep, Span::Exact)
        };
        if let Some(kind) = node.remember(Entry::Parse(context, slot, span)) {
            counts.evictions[kind] += 1;
        }
        node.parse_span(0)
    }

    /// The summaries of NODE's children as one, the first child entered as
    /// SLOT says and each next one as the one before leaves it: exact for
    /// SLOT's count, which every child's was chosen for.
    fn composed<T: Text>(counts: &mut Stats, scanner: &mut Scanner<'_, T>, (node, start, context): (&mut Node, usize, u8),
                         slot: Slot) -> Option<Summary> {
        let (mut at, mut entry, mut whole) = (start, slot, None::<Summary>);
        for child in &mut node.children {
            let chars = child.chars;
            let Span::Exact(next) = Self::span(counts, scanner, (child, at, context), entry.0, entry.1) else { return None };
            entry = (next.exit(), match entry.1 {
                Some(open) => Some(next.levels_after(open)?),
                None => None,
            });
            whole = Some(match whole {
                None => next.clone(),
                Some(so_far) => {
                    counts.composed += 1;
                    summary::compose(&so_far, next, u32::try_from(at - start).ok()?)?
                }
            });
            at += chars;
        }
        Some(match slot.1 {
            Some(levels) => whole?.exact_for(levels),
            None => whole?,
        })
    }

    /// Visit nodes in order, descending where the caller cannot cross
    /// under that node. Stop at the first leaf it cannot cross.
    fn fold(node: &mut Node, start: usize, (from, to): (usize, usize),
            visit: &mut impl FnMut(&mut Node, usize) -> bool) -> usize {
        let end = start + node.chars;
        if from <= start && end <= to {
            if visit(node, start) {
                return end;
            }
            if node.is_leaf() {
                return start;
            }
        }
        let mut at = start;
        for child in &mut node.children {
            let child_end = at + child.chars;
            if from < child_end && at < to {
                let reached = Self::fold(child, at, (from, to), visit);
                if reached < child_end.min(to) {
                    return reached;
                }
            }
            at = child_end;
        }
        end.min(to)
    }

    /// `parse-partial-sexp` from FROM to TO (character positions) in a
    /// text of CHARS characters, starting in STATE: what the scanner alone
    /// gives, reading the leaves FROM and TO are in, and those of a run of
    /// unsafe ends between. From a leaf whose scan leaves a state that is
    /// not `resumable` it scans to TO. False, with STATE as it was, where
    /// the summaries do not serve it: no leaf with a safe start and end
    /// lies between, or STATE or the state after FROM's leaf is not
    /// `resumable`.
    pub(crate) fn parse<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, chars: usize, context: Context,
                                 from: i64, to: i64, state: &mut ParseState) -> R<bool> {
        let context = self.prepare(scanner, chars, context);
        let bit = 1 << context;
        let Self { root: Some(root), stats, .. } = self else { unreachable!("built above") };
        let (first_end, _) = safe_end(root, offset(from), bit);
        let (last_start, ..) = locate(root, offset((to - 1).max(from)), bit);
        if last_start <= first_end {
            stats.declined[Decline::Short as usize] += 1;
            return Ok(false);
        }
        let mut carried = state.clone();
        // The caller's state resumes as GNU's does; every later step continues it.
        let resumed = resumable(state) && {
            scanner.scan_sexps_forward(&mut carried, scanner.text.pos(from), position(first_end), i64::MIN, false, 0)?;
            resumable(&carried)
        };
        if !resumed {
            stats.declined[Decline::Unresumable as usize] += 1;
            return Ok(false);
        }
        let mut at = first_end;
        loop {
            at = Self::fold(root, 0, (at, last_start), &mut |node, start| {
                let levels = u32::try_from(carried.levelstarts.len()).ok();
                if let Span::Exact(span) = Self::span(stats, scanner, (node, start, context), Key::of(&carried), levels) {
                    summary::apply(&mut carried, span, position(start))
                } else {
                    false
                }
            });
            if at == last_start {
                break;
            }
            // The fold stopped before a leaf with an unsafe end, or one whose
            // scan leaves a state no key resumes: the first is scanned with
            // the leaves after it to a safe end, the second on to TO.
            let (next, leaves) = safe_end(root, at, bit);
            if next > last_start {
                break;
            }
            let mut across = carried.clone();
            summary::scan(scanner, &mut across, position(at), position(next))?;
            if !resumable(&across) {
                break;
            }
            stats.plain_leaves += leaves;
            (carried, at) = (across, next);
        }
        summary::scan(scanner, &mut carried, position(at), to)?;
        *state = carried;
        stats.served += 1;
        Ok(true)
    }

    /// Bytes the index holds, for memory accounting.
    pub(crate) fn storage_bytes(&self) -> usize {
        fn bytes(node: &Node) -> usize {
            size_of::<Node>()
                + node.spans.capacity() * size_of::<Entry>()
                + node.spans.iter().map(|entry| if let Entry::Parse(_, _, Span::Exact(summary)) = entry { summary.opened_bytes() } else { 0 }).sum::<usize>()
                + node.children.iter().map(bytes).sum::<usize>()
                + (node.children.capacity() - node.children.len()) * size_of::<Node>()
        }
        self.root.as_ref().map_or(0, bytes)
    }
    /// For each context slot, the leaves whose end it found unsafe; none
    /// for a slot no context holds.
    pub(crate) fn unsafe_cuts(&self) -> [Option<u64>; CONTEXTS] {
        fn count(node: &Node, bit: u8) -> u64 {
            if node.is_leaf() { u64::from(node.unsafe_ends & bit != 0) } else { node.children.iter().map(|child| count(child, bit)).sum() }
        }
        std::array::from_fn(|slot| self.contexts[slot].as_ref()
            .map(|_| self.root.as_ref().map_or(0, |root| count(root, 1 << slot))))
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
        (node.unchecked, node.unsafe_ends) = (EVERY, 0);
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

/// The first leaf under NODE (at START) whose end the context of BIT has
/// not checked: its start, its characters, the contexts that have not
/// checked it, and whether any found it unsafe when last checked.
fn first_unchecked(node: &Node, start: usize, bit: u8) -> Option<(usize, usize, u8, bool)> {
    if node.unchecked & bit == 0 {
        return None;
    }
    if node.is_leaf() {
        return Some((start, node.chars, node.unchecked, node.unsafe_ends != 0));
    }
    let mut at = start;
    for child in &node.children {
        if child.unchecked & bit != 0 {
            return first_unchecked(child, at, bit);
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
        (node.unchecked, node.unsafe_ends) = (EVERY, 0);
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

/// The end of the first leaf at or after the one holding offset AT under
/// ROOT whose end is safe for the context of BIT, and how many leaves that
/// is.
fn safe_end(root: &Node, at: usize, bit: u8) -> (usize, u64) {
    let (mut end, mut leaves) = (at, 0);
    loop {
        let (start, chars, unsafe_end) = locate(root, end, bit);
        (end, leaves) = (start + chars, leaves + 1);
        if !unsafe_end || end >= root.chars {
            return (end, leaves);
        }
    }
}

/// The leaf holding offset AT under ROOT (the last, past the end): its
/// start, characters and whether its end is unsafe for the context of BIT.
fn locate(root: &Node, at: usize, bit: u8) -> (usize, usize, bool) {
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
    (start, node.chars, node.unsafe_ends & bit != 0)
}

/// Drop every summary under NODE and uncheck every leaf end.
fn forget(node: &mut Node) {
    node.spans.clear();
    if node.is_leaf() {
        (node.unchecked, node.unsafe_ends) = (EVERY, 0);
    } else {
        node.children.iter_mut().for_each(forget);
        node.sum();
    }
}

mod forward;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod forward_tests;
#[cfg(test)]
mod list_tests;
