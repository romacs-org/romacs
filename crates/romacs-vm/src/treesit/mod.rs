//! GNU treesit.c's objects and edit bookkeeping (§2 of the treesit design).
//! `native` is the runtime; this module holds what treesit.c keeps per
//! parser and per buffer, with GNU's arithmetic: edits clipped to the
//! parser's visible region (`treesit_record_change_1`), the region synced
//! to the narrowing before a parse (`treesit_sync_visible_region`), and the
//! line/column caches of tracking buffers. Byte positions are GNU's
//! 1-based ones; `TextBody` offsets are 0-based.
pub(crate) mod native;

use crate::text::{FrozenText, TextBody};
use std::sync::OnceLock;
use crate::Value;
use native::{TSInputEdit, TSPoint, Tree};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A grammar linked into the executable (DIVERGENCES VM2-TREESIT-BUNDLED):
/// GNU's language symbol name, its `tree_sitter_LANG` function, and the
/// repository and commit of its sources.
#[derive(Clone, Copy, Debug)]
pub struct BundledGrammar {
    pub name: &'static str,
    pub language: unsafe extern "C" fn() -> *const (),
    pub url: &'static str,
    pub commit: &'static str,
}

static BUNDLED: OnceLock<Vec<BundledGrammar>> = OnceLock::new();

/// Make GRAMMARS the executable's bundled grammars, once per process,
/// before any VM loads a language. A later call is ignored.
///
/// # Safety
/// Each grammar's `language` must be a tree-sitter grammar function
/// (`tree_sitter_LANG`) linked into this process: callable at any time
/// from any thread, returning a pointer to a static `TSLanguage` of an
/// ABI the runtime accepts or rejects through `ts_parser_set_language`.
pub unsafe fn register_bundled_grammars(grammars: Vec<BundledGrammar>) {
    let _ = BUNDLED.set(grammars);
}

/// The bundled grammars, none unless the executable registered them.
pub(crate) fn bundled_grammars() -> &'static [BundledGrammar] {
    BUNDLED.get().map_or(&[], Vec::as_slice)
}

/// GNU's `struct ts_linecol`; `bytepos` 0 is the empty linecol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Linecol {
    pub line: i64,
    pub col: i64,
    pub bytepos: i64,
}
romacs_snapshot::snapshot_struct!(Linecol { line, col, bytepos });
pub(crate) const BOB_LINECOL: Linecol = Linecol { line: 1, col: 0, bytepos: 1 };
pub(crate) const EMPTY_LINECOL: Linecol = Linecol { line: 0, col: 0, bytepos: 0 };

/// What an edit of the text needs of a parser, shared by its Lisp object
/// and its text's registry, so every text mutation edits the tree at once
/// as GNU's insdel calls do.
pub(crate) struct Core {
    pub tree: Option<Rc<Tree>>,
    pub need_reparse: bool,
    /// Whether TREE came from no earlier tree. An incremental parse need
    /// not build what a parse from nothing would: error recovery differs.
    pub fresh: bool,
    /// `visible_beg/end`: the text the tree sees, 1-based bytes.
    pub visible: (i64, i64),
    /// `visi_beg/end_linecol`; None unless the parser tracks linecols.
    pub linecols: Option<(Linecol, Linecol)>,
}

/// One text change, GNU's `treesit_record_change` arguments: 1-based bytes
/// and, in a tracking buffer, the linecols of its three positions.
#[derive(Clone, Copy)]
pub(crate) struct Change {
    pub start: i64,
    pub old_end: i64,
    pub new_end: i64,
    pub linecols: Option<(Linecol, Linecol, Linecol)>,
}

/// The parsers of a text (GNU's base buffer `ts_parser_list`), newest
/// first, and the text's linecol caches.
#[derive(Default)]
pub(crate) struct Registry {
    pub parsers: Vec<(Value, Rc<RefCell<Core>>)>,
    /// A first parse started before a parser asks for it.
    pub early: Option<Early>,
}
/// A copy of a text (a snapshot or a cloned buffer) has no parsers.
impl Clone for Registry {
    fn clone(&self) -> Self {
        Self::default()
    }
}
// SAFETY: a world image holds no parser: live treesit objects refuse world
// capture (`Object::foreign_resource`) and `world_quiescent` an early
// parse, so a captured registry is empty.
unsafe impl romacs_snapshot::SnapshotSafe for Registry {}

/// The most first parses running at once, whatever a session visits.
const EARLY_LIMIT: usize = 4;
static EARLY_RUNNING: AtomicUsize = AtomicUsize::new(0);
/// One running first parse, counted until its thread ends.
pub(crate) struct EarlySlot;
impl EarlySlot {
    /// A slot, unless too many are running.
    pub(crate) fn take() -> Option<Self> {
        EARLY_RUNNING.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < EARLY_LIMIT).then_some(n + 1)).ok().map(|_| EarlySlot)
    }
}
/// Whether no thread holds a frozen text, which shares its rope's reference
/// counts with the live one.
pub(crate) fn early_idle() -> bool {
    EARLY_RUNNING.load(Ordering::Relaxed) == 0
}
impl Drop for EarlySlot {
    fn drop(&mut self) {
        EARLY_RUNNING.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A first parse on a thread of its own, overlapping what a visit does
/// before a parser asks for the tree (`set-auto-mode', the mode's body): it
/// reads a frozen copy of the text, so a parser may use it while the text
/// has not changed since (CHARS_MODIFF). Dropped, the thread ends alone.
pub(crate) struct Early {
    pub language: native::Language,
    pub visible: (i64, i64),
    pub chars_modiff: u64,
    job: std::thread::JoinHandle<Option<native::Loose>>,
}
impl Early {
    /// The parse of TEXT's VISIBLE region (1-based bytes) in SLOT.
    pub(crate) fn start(slot: EarlySlot, language: native::Language, text: FrozenText, visible: (i64, i64), chars_modiff: u64) -> Option<Self> {
        let job = std::thread::Builder::new().name("treesit-parse".into()).spawn(move || {
            // The slot outlives the text.
            let _slot = slot;
            let text = text;
            let mut parser = native::Parser::new();
            parser.set_language(language);
            let mut reader = Reader::new(Some(&text), ((visible.0 - 1) as usize, (visible.1 - 1) as usize));
            parser.parse(None, &mut reader).map(Tree::loose)
        }).ok()?;
        Some(Self { language, visible, chars_modiff, job })
    }
    /// The tree, once the thread ends.
    pub(crate) fn finish(self) -> Option<Tree> {
        self.job.join().ok().flatten().map(native::Loose::tree)
    }
}

impl Registry {
    pub(crate) fn is_empty(&self) -> bool {
        self.parsers.is_empty()
    }
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for (parser, _) in &mut self.parsers {
            visit(parser);
        }
    }
    /// treesit_record_change_1 for every parser of TEXT, edited already.
    pub(crate) fn record(&self, change: Change, text: &TextBody) {
        for (_, core) in &self.parsers {
            core.borrow_mut().record(change, text);
        }
    }
}

fn edit(start: i64, old_end: i64, new_end: i64, points: [TSPoint; 3]) -> TSInputEdit {
    TSInputEdit {
        start_byte: start as u32,
        old_end_byte: old_end as u32,
        new_end_byte: new_end as u32,
        start_point: points[0],
        old_end_point: points[1],
        new_end_point: points[2],
    }
}

impl Core {
    pub(crate) fn new(visible: (i64, i64), linecols: Option<(Linecol, Linecol)>) -> Self {
        Self { tree: None, need_reparse: true, fresh: false, visible, linecols }
    }
    /// treesit_record_change_1 for this parser: the change clipped to the
    /// visible region, the region moved, and the tree edited.
    fn record(&mut self, c: Change, text: &TextBody) {
        let Some(tree) = &self.tree else { return };
        let (visible_beg, visible_end) = self.visible;
        let start_offset = visible_end.min(visible_beg.max(c.start)) - visible_beg;
        let old_end_offset = visible_end.min(visible_beg.max(c.old_end)) - visible_beg;
        let new_end_offset = visible_beg.max(c.new_end) - visible_beg;
        let delta = if c.old_end > c.new_end {
            visible_beg.min(c.new_end) - visible_beg.min(c.old_end)
        } else if c.old_end < visible_beg {
            c.new_end - c.old_end
        } else {
            0
        };
        let new_visible = (visible_beg + delta, visible_end + delta + (new_end_offset - old_end_offset));
        let mut points = [native::POINT_1_0; 3];
        if let (Some((old_beg_lc, old_end_lc)), Some((start, old_end, new_end))) = (self.linecols, c.linecols) {
            let beg_lc = linecol_by_change(text, old_beg_lc, (start, old_end, new_end), new_visible.0);
            let end_lc = linecol_by_change(text, old_end_lc, (start, old_end, new_end), new_visible.1);
            self.linecols = Some((beg_lc, end_lc));
            points = [ts_point(beg_lc, start), ts_point(old_beg_lc, old_end), ts_point(beg_lc, new_end)];
        }
        self.visible = new_visible;
        tree.edit(&edit(start_offset, old_end_offset, new_end_offset, points));
        self.need_reparse = true;
    }
    /// treesit_sync_visible_region: the tree's view moved to the buffer's
    /// accessible region BEGV..ZV (1-based bytes), the parse forced when it
    /// moved. LINECOLS are the buffer's for BEGV and ZV, when tracking.
    pub(crate) fn sync_visible(&mut self, begv: i64, zv: i64, linecols: Option<(Linecol, Linecol)>) {
        let Some(tree) = &self.tree else {
            self.visible = (begv, zv);
            return;
        };
        let (mut beg, mut end) = self.visible;
        if beg != begv || end != zv {
            self.need_reparse = true;
        }
        let track = linecols.zip(self.linecols);
        let (mut beg_lc, mut end_lc) = self.linecols.unwrap_or((EMPTY_LINECOL, EMPTY_LINECOL));
        let (begv_lc, zv_lc) = linecols.unwrap_or((EMPTY_LINECOL, EMPTY_LINECOL));
        let point = |from: Linecol, to: Linecol| if track.is_some() { ts_point(from, to) } else { native::POINT_1_0 };
        let one = native::POINT_1_0;
        if beg > begv {
            tree.edit(&edit(0, 0, beg - begv, [one, one, point(begv_lc, beg_lc)]));
            beg = begv;
            beg_lc = begv_lc;
        }
        if end < zv {
            let start = point(beg_lc, end_lc);
            tree.edit(&edit(end - beg, end - beg, zv - beg, [start, start, point(beg_lc, zv_lc)]));
            end = zv;
            end_lc = zv_lc;
        } else if end > zv {
            let start = point(beg_lc, zv_lc);
            tree.edit(&edit(zv - beg, end - beg, zv - beg, [start, point(beg_lc, end_lc), start]));
            end = zv;
            end_lc = zv_lc;
        }
        if beg < begv {
            tree.edit(&edit(0, begv - beg, 0, [one, point(beg_lc, begv_lc), one]));
            beg = begv;
            beg_lc = begv_lc;
        }
        self.visible = (beg, end);
        if self.linecols.is_some() {
            self.linecols = Some((beg_lc, end_lc));
        }
    }
}

/// treesit_make_ts_point: POS relative to VISIBLE_BEG.
fn ts_point(visible_beg: Linecol, pos: Linecol) -> TSPoint {
    if visible_beg.line == pos.line {
        TSPoint { row: 0, column: (pos.col - visible_beg.col) as u32 }
    } else {
        TSPoint { row: (pos.line - visible_beg.line) as u32, column: pos.col as u32 }
    }
}

/// compute_new_linecol_by_change: the linecol of TARGET in TEXT after a
/// change (START, OLD_END, NEW_END), from POS's before it.
pub(crate) fn linecol_by_change(text: &TextBody, pos: Linecol, (start, old_end, new_end): (Linecol, Linecol, Linecol), target: i64) -> Linecol {
    let moved = if start.bytepos >= pos.bytepos {
        pos
    } else if old_end.bytepos <= pos.bytepos {
        let col = if old_end.line == pos.line { new_end.col + (pos.col - old_end.col) } else { pos.col };
        Linecol { line: pos.line + new_end.line - old_end.line, col, bytepos: pos.bytepos + new_end.bytepos - old_end.bytepos }
    } else if target - start.bytepos < (target - new_end.bytepos).abs() {
        linecol_of_pos(text, target, start)
    } else {
        linecol_of_pos(text, target, new_end)
    };
    if moved.bytepos != target { linecol_of_pos(text, target, moved) } else { moved }
}

/// treesit_count_lines: from START toward LIMIT (1-based bytes, whole
/// text), at most one newline in COUNT's direction; the newlines counted
/// and where it stopped: after the newline going forward, on it going
/// backward, else at LIMIT.
fn count_lines(text: &TextBody, start: i64, limit: i64, forward: bool) -> (i64, i64) {
    let (from, to) = ((start - 1) as usize, (limit - 1) as usize);
    let found = if forward { text.newline_after(from, to).map(|n| n as i64 + 2) } else { text.newline_before(from, to).map(|n| n as i64 + 1) };
    found.map_or((0, limit), |at| (1, at))
}

/// treesit_linecol_of_pos: the linecol of TARGET (1-based byte), scanned
/// from CACHE, which must be valid.
pub(crate) fn linecol_of_pos(text: &TextBody, target: i64, cache: Linecol) -> Linecol {
    if target == cache.bytepos {
        return cache;
    }
    let (mut at, mut previous, mut delta) = (cache.bytepos, 0, 0);
    if cache.bytepos < target {
        while at < target {
            let (counted, stop) = count_lines(text, at, target, true);
            at = stop;
            if counted > 0 {
                previous = at;
            }
            delta += counted;
        }
        let col = if delta > 0 { target - previous } else { target - cache.bytepos + cache.col };
        return Linecol { line: cache.line + delta, col, bytepos: target };
    }
    while at > target {
        let (counted, stop) = count_lines(text, at, target, false);
        at = stop;
        delta -= counted;
    }
    let col = if delta == 0 {
        cache.col - (cache.bytepos - target)
    } else {
        // GNU's expression, precedence included: `target_bytepos -
        // (byte_pos_2 + counted == 1 ? 1 : 0)`.
        let (counted, stop) = count_lines(text, at, 1, false);
        target - i64::from(stop + counted == 1)
    };
    Linecol { line: cache.line + delta, col, bytepos: target }
}

/// A text a parse can read.
pub(crate) trait Chunks {
    /// The bytes from AT to the end of their chunk; empty at the end.
    fn chunk_from(&self, at: usize) -> &[u8];
    /// The offsets of the raw-byte placeholders inside R.
    fn raw_in(&self, r: Range<usize>) -> &[u32];
}

/// The text a parse reads: TEXT's bytes in the visible region, chunk by
/// chunk, raw-byte placeholders as GNU's internal bytes (§2.3).
pub(crate) struct Reader<'a, T> {
    /// None for a killed buffer, which reads as empty.
    pub text: Option<&'a T>,
    /// 0-based bytes.
    pub visible: (usize, usize),
    raw: [u8; 2],
}
impl<'a, T> Reader<'a, T> {
    pub(crate) fn new(text: Option<&'a T>, visible: (usize, usize)) -> Self {
        Self { text, visible, raw: [0; 2] }
    }
}
impl<T: Chunks> native::Source for Reader<'_, T> {
    fn read(&mut self, offset: usize) -> &[u8] {
        let at = self.visible.0 + offset;
        let Some(text) = self.text.filter(|_| at < self.visible.1) else { return &[] };
        let chunk = text.chunk_from(at);
        let chunk = &chunk[..chunk.len().min(self.visible.1 - at)];
        match text.raw_in(at..at + chunk.len()).first() {
            Some(&p) if p as usize == at => {
                self.raw = [chunk[0] - crate::text::RAW_LEAD_SHIFT, chunk[1]];
                &self.raw
            }
            Some(&p) => &chunk[..p as usize - at],
            None => chunk,
        }
    }
}

/// The Lisp treesit objects, one heap variant (`data::Object::Treesit`).
/// Each holds Lisp references, which the collector visits.
#[derive(Clone)]
pub(crate) enum Object {
    Parser(Parser),
    Node(Node),
    Query(Query),
    /// A primitive's state across its Lisp calls (`prim/leaf/treesit.rs`).
    Task(Task),
}

/// GNU's `Lisp_TS_Parser`. A copy shares the parser (no Lisp operation
/// copies one).
#[derive(Clone)]
pub(crate) struct Parser {
    pub buffer: Value,
    pub language: Value,
    pub tag: Value,
    pub embed_level: Value,
    pub notifiers: Value,
    pub last_set_ranges: Value,
    pub native: Rc<RefCell<native::Parser>>,
    pub core: Rc<RefCell<Core>>,
    pub timestamp: u64,
    pub deleted: bool,
    pub within_reparse: bool,
}

/// GNU's `Lisp_TS_Node`: a node of the tree generation TREE, made when its
/// parser's timestamp was TIMESTAMP.
#[derive(Clone)]
pub(crate) struct Node {
    pub parser: Value,
    pub node: native::Node,
    pub tree: Rc<Tree>,
    pub timestamp: u64,
}

/// GNU's `Lisp_TS_Query`: compiled on first use unless eager.
#[derive(Clone)]
pub(crate) struct Query {
    pub language: Value,
    pub source: Value,
    pub compiled: Option<Rc<Compiled>>,
}

/// A compiled query and each pattern's predicates
/// (`treesit_predicates_for_pattern`).
pub(crate) struct Compiled {
    pub query: Rc<native::Query>,
    pub predicates: Vec<Vec<Predicate>>,
}

/// One predicate: its name, as the query spells it after `#`, and its
/// arguments.
pub(crate) struct Predicate {
    pub name: Vec<u8>,
    pub args: Vec<Arg>,
}
pub(crate) enum Arg {
    /// A capture, by its index in the query.
    Capture(u32),
    String(Vec<u8>),
}

impl Compiled {
    pub(crate) fn new(query: Rc<native::Query>) -> Self {
        let predicates = (0..query.pattern_count()).map(|p| {
            let mut out = Vec::new();
            let mut current: Vec<Arg> = Vec::new();
            let mut name: Option<Vec<u8>> = None;
            for step in query.predicates(p) {
                match step {
                    native::PredicateStep::Done => {
                        if let Some(name) = name.take() {
                            out.push(Predicate { name, args: std::mem::take(&mut current) });
                        }
                    }
                    native::PredicateStep::String(s) if name.is_none() => name = Some(s.to_vec()),
                    native::PredicateStep::String(s) => current.push(Arg::String(s.to_vec())),
                    native::PredicateStep::Capture(index) => {
                        name.get_or_insert_with(Vec::new);
                        current.push(Arg::Capture(index));
                    }
                }
            }
            out
        }).collect();
        Self { query, predicates }
    }
}

/// A primitive suspended in a Lisp call; its values are traced.
#[derive(Clone)]
pub(crate) struct Task {
    pub values: Vec<Value>,
    pub state: Rc<RefCell<TaskState>>,
}

/// The native part of a suspended primitive (`prim/leaf/treesit.rs`).
pub(crate) enum TaskState {
    /// treesit-query-capture between `#pred?` calls.
    Capture(crate::treesit::Capture),
    /// A tree walk between predicate calls.
    Walk(crate::treesit::Walk),
}

/// treesit-query-capture's matches and progress. `values` of the task
/// holds, from index 0: the parser, then per match its capture names and
/// nodes in pairs, as MATCHES's ranges index them.
pub(crate) struct Capture {
    pub compiled: Rc<Compiled>,
    /// (pattern, start in `values`, capture count) per match.
    pub matches: Vec<(u32, usize, usize)>,
    pub at: usize,
    pub predicate: usize,
    /// Indices into MATCHES of the matches that passed.
    pub passed: Vec<usize>,
    pub timestamp: u64,
}

/// A walk of a tree (search, sparse tree, match) between predicate calls:
/// the cursor, the predicate results of the current node so far, and the
/// walk's own state.
pub(crate) struct Walk {
    pub cursor: native::Cursor,
    pub tree: Rc<Tree>,
    pub results: Vec<bool>,
    /// The pending call: whether it is guarded, and the parser's timestamp
    /// when it was made.
    pub pending: Option<(bool, u64)>,
    pub kind: WalkKind,
}

pub(crate) enum WalkKind {
    /// treesit-node-match-p.
    Match,
    /// treesit-search-subtree: depth-first, GNU's `treesit_search_dfs`.
    Subtree { forward: bool, named: bool, stack: Vec<Dfs> },
    /// treesit-search-forward.
    Forward { forward: bool, named: bool, phase: ForwardPhase },
    /// treesit-induce-sparse-tree, its process-fn pending when PROCESSING.
    Sparse { stack: Vec<SparseFrame>, processing: bool },
}

/// A level of the depth-first search: matching its node, or iterating its
/// children.
#[derive(Clone, Copy)]
pub(crate) enum Dfs {
    /// Matching the node itself, LIMIT levels left below it.
    Enter { limit: i64 },
    /// Done with a child; LIMIT is the parent's.
    Children { limit: i64 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardPhase {
    /// Match the node under the cursor, the start excluded.
    Initial,
    /// Match the node reached by going to the next leaf.
    Leaf,
    /// Match the parent reached when no sibling was left.
    Parent,
}

/// A level of the sparse tree under construction: its limit, whether its
/// node matched, and whether it is iterating its children. The task's
/// values hold each level's parent cons (`prim/leaf/treesit.rs`).
#[derive(Clone, Copy)]
pub(crate) struct SparseFrame {
    pub limit: i64,
    pub matched: bool,
    pub children: bool,
}

impl Object {
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Object::Parser(p) => {
                for v in [&mut p.buffer, &mut p.language, &mut p.tag, &mut p.embed_level, &mut p.notifiers, &mut p.last_set_ranges] {
                    visit(v);
                }
            }
            Object::Node(n) => visit(&mut n.parser),
            Object::Query(q) => {
                visit(&mut q.language);
                visit(&mut q.source);
            }
            Object::Task(t) => t.values.iter_mut().for_each(visit),
        }
    }
    /// Live native resources: no Lisp world image may hold one.
    pub(crate) fn foreign_resource(&self) -> Option<&'static str> {
        Some(match self {
            Object::Parser(_) => "a tree-sitter parser",
            Object::Node(_) => "a tree-sitter node",
            Object::Query(q) if q.compiled.is_none() => return None,
            Object::Query(_) => "a compiled tree-sitter query",
            Object::Task(_) => "a tree-sitter task",
        })
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + match self {
            Object::Task(t) => t.values.capacity() * std::mem::size_of::<Value>(),
            _ => 0,
        }
    }
}

/// print.c's text of the treesit object OBJECT.
pub(crate) fn describe(vm: &crate::Vm, object: &Object) -> String {
    let symbol = |v: Value| v.index_if(7).and_then(|i| i.checked_sub(1)).and_then(|i| vm.symbol_names.get(i)).cloned().unwrap_or_else(|| "nil".into());
    let parser = |v: Value| match vm.object_if(v) {
        Some(crate::data::Object::Treesit(t)) => match &**t { Object::Parser(p) => Some(p), _ => None },
        _ => None,
    };
    match object {
        Object::Parser(p) => format!("#<treesit-parser for {}>", symbol(p.language)),
        Object::Query(_) => "#<treesit-compiled-query>".into(),
        Object::Task(_) => "#<treesit-task>".into(),
        Object::Node(n) => {
            let Some(p) = parser(n.parser) else { return "#<treesit-node>".into() };
            if p.timestamp != n.timestamp {
                return "#<treesit-node-outdated>".into();
            }
            let text = match vm.object_if(p.buffer) {
                Some(crate::data::Object::Buffer(b)) if b.state.live => b,
                _ => return "#<treesit-node-in-killed-buffer>".into(),
            };
            let beg = p.core.borrow().visible.0;
            let pos = |offset: u32| {
                let byte = ((beg - 1 + i64::from(offset)).max(0) as usize).min(text.len_bytes());
                text.char_of_byte(crate::coordinates::ByteOffset(byte)).map_or(1, |c| c.0 + 1)
            };
            let kind = String::from_utf8_lossy(n.node.kind().unwrap_or(b"")).into_owned();
            let kind = if n.node.is_named() { kind } else { format!("\"{kind}\"") };
            format!("#<treesit-node {kind} in {}-{}>", pos(n.node.start()), pos(n.node.end()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EARLY_LIMIT, EarlySlot, early_idle};

    #[test]
    fn early_slots_stop_at_the_limit_and_all_come_back() {
        let slots: Vec<_> = std::iter::from_fn(EarlySlot::take).collect();
        assert_eq!(slots.len(), EARLY_LIMIT);
        assert!(EarlySlot::take().is_none() && EarlySlot::take().is_none());
        drop(slots);
        assert!(early_idle());
        assert!(EarlySlot::take().is_some());
        assert!(early_idle());
    }
}
