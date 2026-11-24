//! treesit-fuzz GRAMMAR FILE EDITS SEED [DUMP]: FILE parsed with the grammar
//! Romacs ships under the name GRAMMAR, then EDITS seeded typing-like edits,
//! each followed by a reparse from the edited tree and a fresh parse of the
//! same text. The first line names GRAMMAR; then one line per state: the
//! edit's number, whether the reparsed tree and the fresh tree have an error
//! (0 or 1), five digests (the reparsed tree's records and parse states, the
//! ranges that changed from the previous tree, the fresh tree's records and
//! parse states), the reparse's microseconds, and for each field of the
//! records `NAME=REPARSED/FRESH`, the digests of that field's values over all
//! records of each tree. With DUMP, the state of that number also prints
//! every record behind the digests, so two runs' first difference can be
//! shown. A record's fields are `NAME=VALUE`, each ended by the unit
//! separator (0x1f), as values hold spaces.
//!
//! A tree's records are one per node, holding what each of GETTERS gives for
//! it, and one per looked-up position, holding what each of LOOKUPS gives
//! (named `lookup:NAME`). `test_getters.py` here holds them to every node and
//! cursor method `crates/romacs-vm/src/treesit/native.rs` wraps. A getter
//! that takes an index is asked at every index of a node of at most 256
//! children, else at a seeded window of 64, the first and the last, and names
//! the indices it asked; a range lookup is asked at the node's extent and at
//! a seeded interior range of it. The positions looked up in each state are
//! the edit's two ends and 8 seeded offsets. Parse states are apart because a
//! leaf's is the state it was lexed in, which depends on the parse's history
//! and not only on the text. Nodes the grammar hides are not in the API, so
//! their shape is outside the comparison except through what it decides.
//!
//! TREESIT_FUZZ_PERTURB plants a fault, for the checker's self-test: `node`
//! marks the root's kind, `child-swap` answers `child(1)` with child 0,
//! `kind-and-child` marks the root's kind and answers `child(1)` with child 0
//! in nodes after the first byte, `interior-range` answers the interior range
//! queries with the node's extent's, `ranges` adds a mark to the changed
//! ranges, `states` marks the root's parse states, `fresh-clean` and
//! `fresh-errors` put a newline before the fresh parse's text where that
//! parse has no error or has one, `drop-row` leaves out the last state's line,
//! `bad-flags` prints error flags outside 0 and 1, `wrong-grammar` names
//! another grammar, `exit` ends the worker with status 77 after its output,
//! and `field-name` blanks every name `field_name_for_child` gives.
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use tree_sitter::{InputEdit, Node, Parser, Point, Tree, TreeCursor};

fn language(name: &str) -> tree_sitter::Language {
    let grammar = romacs_grammars::BUNDLED.iter().find(|g| g.name == name);
    grammar.unwrap_or_else(|| panic!("no shipped grammar named {name}")).language.into()
}

fn point(text: &[u8], byte: usize) -> Point {
    let before = &text[..byte];
    let row = before.iter().filter(|&&b| b == b'\n').count();
    let column = byte - before.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
    Point { row, column }
}

/// The line starts of a text, so a byte's point costs a search, not a scan.
struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    fn new(text: &[u8]) -> Self {
        let starts = std::iter::once(0).chain(text.iter().enumerate().filter(|&(_, &b)| b == b'\n').map(|(i, _)| i + 1));
        Lines { starts: starts.collect(), len: text.len() }
    }
    /// BYTE's point; a byte past the end, as a planted fresh parse's shifted
    /// nodes give, counts as the end.
    fn point(&self, byte: usize) -> Point {
        let byte = byte.min(self.len);
        let row = self.starts.partition_point(|&start| start <= byte) - 1;
        Point { row, column: byte - self.starts[row] }
    }
}

/// xorshift64*, seeded; its state is never zero.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// An edit: replace START..OLD_END with NEW.
struct Edit {
    start: usize,
    old_end: usize,
    new: Vec<u8>,
}

/// A whole line of TEXT, picked at random, with its newline.
fn some_line(rng: &mut Rng, text: &[u8]) -> Vec<u8> {
    let at = rng.below(text.len() as u64 + 1) as usize;
    let start = text[..at].iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
    let end = text[at..].iter().position(|&b| b == b'\n').map_or(text.len(), |p| at + p + 1);
    text[start..end].to_vec()
}

/// The next edit, mostly where the last one ended, sometimes a jump: code
/// characters, newlines and indentation, deletions, a pasted line, a line
/// copied from elsewhere, and, half the time, an undo, which pops UNDO, the
/// edits that revert the ones not yet undone. Undo walks the text back to the
/// file's own, so error-free text keeps coming back. A new edit pushes its
/// revert.
fn next_edit(rng: &mut Rng, text: &[u8], at: &mut usize, undo: &mut Vec<Edit>) -> Edit {
    if rng.below(2) == 0
        && let Some(edit) = undo.pop()
    {
        return edit;
    }
    if rng.below(20) == 0 || *at > text.len() {
        *at = rng.below(text.len() as u64 + 1) as usize;
    }
    let pos = (*at).min(text.len());
    let pieces: [&[u8]; 24] = [
        b"x", b"x", b"a", b"_", b"1", b" ", b";", b"{", b"}", b"(", b")", b"\n", b"\"", b"/*", b":", b",", b"=", b"'",
        b"#", b"<", b">", b"-", b"\n    ", b"*/",
    ];
    let edit = match rng.below(20) {
        0..=10 => Edit { start: pos, old_end: pos, new: pieces[rng.below(pieces.len() as u64) as usize].to_vec() },
        11..=13 if pos > 0 => Edit { start: pos - 1, old_end: pos, new: vec![] },
        14 => Edit { start: pos, old_end: pos + (rng.below(40) as usize).min(text.len() - pos), new: vec![] },
        15..=17 => Edit { start: pos, old_end: pos, new: some_line(rng, text) },
        _ => Edit { start: pos, old_end: pos, new: b"int z = f(a, b);\n".to_vec() },
    };
    *at = edit.start + edit.new.len();
    let removed = text[edit.start..edit.old_end].to_vec();
    undo.push(Edit { start: edit.start, old_end: edit.start + edit.new.len(), new: removed });
    edit
}

/// A node in the walk: the node, its depth, the cursor's field, the text,
/// the planted fault, and the run's seed, which picks a wide node's window of
/// indices and its interior ranges.
struct Visit<'t> {
    node: Node<'t>,
    depth: usize,
    field: (Option<u16>, Option<&'t str>),
    lines: &'t Lines,
    plant: &'t str,
    seed: u64,
}

/// A node another getter leads to, as its kind id and byte range.
fn target(node: Option<Node>) -> String {
    node.map_or_else(|| "-".into(), |n| format!("{}@{:?}", n.kind_id(), n.byte_range()))
}

fn list(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(",")
}

/// A generator SEED and KEY pick: the run's choices for one node.
fn picker(seed: u64, key: usize) -> Rng {
    Rng((seed ^ (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1)
}

/// The indices of COUNT children a getter that takes an index is asked at:
/// all of them up to 256, else a window of 64 that SEED and KEY place, and
/// the first and the last.
fn indices(count: u32, seed: u64, key: usize) -> Vec<u32> {
    if count <= 256 {
        return (0..count).collect();
    }
    let start = picker(seed, key).below(u64::from(count - 63)) as u32;
    let mut picked: Vec<u32> = [0, count - 1].into_iter().chain(start..start + 64).collect();
    picked.sort_unstable();
    picked.dedup();
    picked
}

/// "index:what GET gives" at each of the node's indices for COUNT children.
fn indexed(v: &Visit, count: u32, get: impl Fn(u32) -> String) -> String {
    list(indices(count, v.seed, v.node.start_byte()).into_iter().map(|i| format!("{i}:{}", get(i))))
}

/// The child index `child(I)` is answered at, where PLANT moves index 1 to 0.
fn swapped(v: &Visit, i: u32) -> u32 {
    let swap = v.plant == "child-swap" || (v.plant == "kind-and-child" && v.node.start_byte() > 0);
    if swap && i == 1 { 0 } else { i }
}

/// The field names `field_name_for_child` gives NODE's children, each once.
fn child_fields<'t>(node: Node<'t>, blank: bool) -> Vec<&'t str> {
    let mut names: Vec<_> = (0..node.child_count()).filter_map(|i| node.field_name_for_child(i)).collect();
    names.sort_unstable();
    names.dedup();
    if blank { vec![] } else { names }
}

/// The byte ranges a range lookup on the node is asked at: its extent, and a
/// seeded interior range start..end, start < end, when it spans 2 bytes or more.
fn spans(v: &Visit) -> Vec<(usize, usize, bool)> {
    let (start, end) = (v.node.start_byte(), v.node.end_byte());
    let mut spans = vec![(start, end, false)];
    if end - start >= 2 {
        let mut rng = picker(v.seed, start ^ end.rotate_left(17));
        let a = start + rng.below((end - start - 1) as u64) as usize;
        spans.push((a, a + 1 + rng.below((end - a - 1) as u64) as usize, true));
    }
    spans
}

/// "start..end:what GET gives" at each of the node's spans; where PLANTED,
/// the interior-range plant answers an interior span with the extent's.
fn spanned(v: &Visit, planted: bool, get: impl Fn(usize, usize) -> String) -> String {
    let (start, end) = (v.node.start_byte(), v.node.end_byte());
    list(spans(v).into_iter().map(|(a, b, interior)| {
        let (qa, qb) = if interior && planted && v.plant == "interior-range" { (start, end) } else { (a, b) };
        format!("{a}..{b}:{}", get(qa, qb))
    }))
}

/// Where a cursor started at NODE goes: after each step's move, the node it
/// is on, until a move fails or STEPS ends. FIRST is the first move.
fn cursor_walk(node: Node, first: impl FnOnce(&mut TreeCursor) -> bool,
               mut next: impl FnMut(&mut TreeCursor) -> bool, steps: usize) -> String {
    let mut cursor = node.walk();
    let mut seen = Vec::new();
    if first(&mut cursor) {
        seen.push(target(Some(cursor.node())));
        while seen.len() < steps && next(&mut cursor) {
            seen.push(target(Some(cursor.node())));
        }
    }
    list(seen.into_iter())
}

type Getter = fn(&Visit) -> String;

/// Each node getter a record holds, named as the runtime's API names it. A
/// getter that leads to other nodes records each one's kind id and range, a
/// cursor getter the nodes a cursor started at the node moves to.
const GETTERS: &[(&str, Getter)] = &[
    ("depth", |v| v.depth.to_string()),
    ("kind", |v| format!("{:?}", v.node.kind())),
    ("kind_id", |v| v.node.kind_id().to_string()),
    ("grammar_name", |v| format!("{:?}", v.node.grammar_name())),
    ("grammar_id", |v| v.node.grammar_id().to_string()),
    ("is_named", |v| v.node.is_named().to_string()),
    ("is_extra", |v| v.node.is_extra().to_string()),
    ("is_error", |v| v.node.is_error().to_string()),
    ("is_missing", |v| v.node.is_missing().to_string()),
    ("has_error", |v| v.node.has_error().to_string()),
    ("has_changes", |v| v.node.has_changes().to_string()),
    ("start_byte", |v| v.node.start_byte().to_string()),
    ("end_byte", |v| v.node.end_byte().to_string()),
    ("start_position", |v| format!("{:?}", v.node.start_position())),
    ("end_position", |v| format!("{:?}", v.node.end_position())),
    ("child_count", |v| v.node.child_count().to_string()),
    ("named_child_count", |v| v.node.named_child_count().to_string()),
    ("descendant_count", |v| v.node.descendant_count().to_string()),
    ("cursor_field", |v| format!("{:?}", v.field)),
    ("field_name_for_child", |v| {
        let blank = v.plant == "field-name";
        indexed(v, v.node.child_count(), |i| format!("{:?}", v.node.field_name_for_child(i).filter(|_| !blank)))
    }),
    ("child", |v| indexed(v, v.node.child_count(), |i| target(v.node.child(swapped(v, i))))),
    ("named_child", |v| indexed(v, v.node.named_child_count() as u32, |i| target(v.node.named_child(i)))),
    ("child_by_field_name", |v| {
        let fields = child_fields(v.node, v.plant == "field-name");
        list(fields.iter().map(|name| format!("{name}={}", target(v.node.child_by_field_name(name)))))
    }),
    ("next_sibling", |v| target(v.node.next_sibling())),
    ("prev_sibling", |v| target(v.node.prev_sibling())),
    ("next_named_sibling", |v| target(v.node.next_named_sibling())),
    ("prev_named_sibling", |v| target(v.node.prev_named_sibling())),
    ("descendant_for_byte_range", |v| spanned(v, true, |a, b| target(v.node.descendant_for_byte_range(a, b)))),
    ("named_descendant_for_byte_range", |v| spanned(v, false, |a, b| target(v.node.named_descendant_for_byte_range(a, b)))),
    ("descendant_for_point_range", |v| {
        spanned(v, false, |a, b| target(v.node.descendant_for_point_range(v.lines.point(a), v.lines.point(b))))
    }),
    ("named_descendant_for_point_range", |v| {
        spanned(v, false, |a, b| target(v.node.named_descendant_for_point_range(v.lines.point(a), v.lines.point(b))))
    }),
    ("to_sexp", |v| if v.depth == 0 { v.node.to_sexp() } else { String::new() }),
    ("walk", |v| target(Some(v.node.walk().node()))),
    ("node", |v| cursor_walk(v.node, |_| true, |_| false, 1)),
    ("goto_first_child", |v| cursor_walk(v.node, |c| c.goto_first_child(), |_| false, 1)),
    ("goto_next_sibling", |v| cursor_walk(v.node, |c| c.goto_first_child(), |c| c.goto_next_sibling(), usize::MAX)),
    ("goto_previous_sibling", |v| {
        let last = |c: &mut TreeCursor| c.goto_first_child() && { while c.goto_next_sibling() {} true };
        cursor_walk(v.node, last, |c| c.goto_previous_sibling(), 64)
    }),
    ("goto_parent", |v| cursor_walk(v.node, |c| c.goto_first_child() && c.goto_parent(), |_| false, 1)),
    ("goto_first_child_for_byte", |v| {
        indexed(v, v.node.child_count(), |i| {
            let byte = v.node.child(i).map_or(v.node.start_byte(), |child| child.start_byte());
            let mut cursor = v.node.walk();
            format!("{:?}", cursor.goto_first_child_for_byte(byte))
        })
    }),
];

/// A position looked up in a tree: its root, and the position as a byte and a point.
struct Lookup<'t> {
    root: Node<'t>,
    byte: usize,
    point: Point,
}

/// The children a cursor started at LOOKUP's root goes down through with STEP.
fn descend(lookup: &Lookup, step: impl Fn(&mut TreeCursor) -> Option<usize>) -> String {
    let mut cursor = lookup.root.walk();
    let mut path = Vec::new();
    while let Some(index) = step(&mut cursor) {
        path.push(format!("{index}:{}", target(Some(cursor.node()))));
    }
    list(path.into_iter())
}

type Lookuper = fn(&Lookup) -> String;

/// What each position lookup gives at a position, named as the runtime's API names it.
const LOOKUPS: &[(&str, Lookuper)] = &[
    ("descendant_for_byte_range", |l| target(l.root.descendant_for_byte_range(l.byte, l.byte))),
    ("named_descendant_for_byte_range", |l| target(l.root.named_descendant_for_byte_range(l.byte, l.byte))),
    ("descendant_for_point_range", |l| target(l.root.descendant_for_point_range(l.point, l.point))),
    ("named_descendant_for_point_range", |l| target(l.root.named_descendant_for_point_range(l.point, l.point))),
    ("goto_first_child_for_byte", |l| descend(l, |c| c.goto_first_child_for_byte(l.byte))),
    ("goto_first_child_for_point", |l| descend(l, |c| c.goto_first_child_for_point(l.point))),
];

/// The node's record: what each getter gives, in GETTERS' order.
fn record(visit: &Visit) -> String {
    let mut line = String::new();
    for (name, getter) in GETTERS {
        let _ = write!(line, "{name}={}\x1f", getter(visit));
    }
    line
}

/// The record of a position looked up: what each lookup gives, in LOOKUPS' order.
fn lookup_record(lookup: &Lookup) -> String {
    let mut line = format!("lookup:at={}\x1f", lookup.byte);
    for (name, get) in LOOKUPS {
        let _ = write!(line, "lookup:{name}={}\x1f", get(lookup));
    }
    line
}

/// A tree's records, with LINES its text's, one per node in document order and one per
/// position of POSITIONS, and its nodes' parse states; PLANT and SEED as `Visit`'s.
fn records(tree: &Tree, lines: &Lines, positions: &[(usize, Point)], plant: &str, seed: u64) -> (Vec<String>, Vec<String>) {
    let (mut records, mut states) = (Vec::new(), Vec::new());
    let mut cursor: TreeCursor = tree.walk();
    let mut depth = 0;
    loop {
        let node = cursor.node();
        let field = (cursor.field_id().map(std::num::NonZero::get), cursor.field_name());
        records.push(record(&Visit { node, depth, field, lines, plant, seed }));
        states.push(format!("{} {}", node.parse_state(), node.next_parse_state()));
        if cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                if plant == "node" || plant == "kind-and-child" {
                    records[0] = records[0].replacen("kind=", "kind=planted", 1);
                }
                let root = tree.root_node();
                records.extend(positions.iter().map(|&(byte, point)| lookup_record(&Lookup { root, byte, point })));
                return (records, states);
            }
            depth -= 1;
        }
    }
}

/// The ranges that changed from OLD to NEW, one line each.
fn changed(old: &Tree, new: &Tree) -> Vec<String> {
    old.changed_ranges(new)
        .map(|r| format!("{}..{} {:?}-{:?}", r.start_byte, r.end_byte, r.start_point, r.end_point))
        .collect()
}

fn digest(lines: impl IntoIterator<Item = impl Hash>) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    lines.into_iter().for_each(|line| line.hash(&mut hash));
    hash.finish()
}

/// Each record field's name, with the digest of its values over LINES in order.
fn field_digests(lines: &[String]) -> std::collections::BTreeMap<&str, u64> {
    let mut values: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for line in lines {
        for field in line.split('\x1f').filter(|f| !f.is_empty()) {
            let (name, value) = field.split_once('=').unwrap_or((field, ""));
            values.entry(name).or_default().push(value);
        }
    }
    values.into_iter().map(|(name, values)| (name, digest(values))).collect()
}

/// One state: the reparsed tree, the ranges changed to it, a fresh parse of
/// the same text, the text's lines, the positions looked up in both, and the reparse's time.
struct State<'a> {
    tree: &'a Tree,
    ranges: Vec<String>,
    fresh: Tree,
    lines: &'a Lines,
    positions: Vec<(usize, Point)>,
    micros: u128,
}

/// Print state I: its flags, digests, microseconds and field digests, and with
/// DUMP its records; PLANT and SEED as `Visit`'s.
fn report(i: usize, state: State, dump: Option<usize>, plant: &str, seed: u64) {
    let (nodes, mut states) = records(state.tree, state.lines, &state.positions, plant, seed);
    let (fresh_nodes, fresh_states) = records(&state.fresh, state.lines, &state.positions, plant, seed);
    let mut ranges = state.ranges;
    if plant == "ranges" {
        ranges.push("planted".into());
    }
    if plant == "states" {
        states[0].push_str(" planted");
    }
    let flags = [state.tree, &state.fresh].map(|tree| u8::from(tree.root_node().has_error()).to_string());
    let flags = if plant == "bad-flags" { ["bogus".to_string(), "bogus".to_string()] } else { flags };
    let digests = [&nodes, &states, &ranges, &fresh_nodes, &fresh_states].map(|lines| format!("{:016x}", digest(lines)));
    let (reparsed, fresh) = (field_digests(&nodes), field_digests(&fresh_nodes));
    let fields: Vec<String> = reparsed.keys().chain(fresh.keys()).collect::<std::collections::BTreeSet<_>>().into_iter()
        .map(|name| format!("{name}={:016x}/{:016x}", reparsed.get(name).unwrap_or(&0), fresh.get(name).unwrap_or(&0)))
        .collect();
    println!("{i} {} {} {} {}", flags.join(" "), digests.join(" "), state.micros, fields.join(" "));
    if dump == Some(i) {
        let kinds = [("node", &nodes), ("state", &states), ("range", &ranges), ("fresh", &fresh_nodes), ("freshstate", &fresh_states)];
        for (kind, lines) in kinds {
            lines.iter().for_each(|line| println!("{kind} {line}"));
        }
    }
}

/// The positions state I of TEXT is looked up at: the ends of the edit that
/// made it (START..END), and 8 offsets SEED and I pick.
fn positions(lines: &Lines, start: usize, end: usize, seed: u64, i: usize) -> Vec<(usize, Point)> {
    let mut rng = Rng((seed ^ (i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)) | 1);
    let offsets = (0..8).map(|_| rng.below(lines.len as u64 + 1) as usize);
    [start, end].into_iter().chain(offsets).map(|byte| (byte, lines.point(byte))).collect()
}

/// A fresh parse of TEXT, with a newline before it where PLANT puts one.
fn fresh_parse(parser: &mut Parser, text: &[u8], plant: &str) -> Tree {
    let tree = parser.parse(text, None).expect("a tree");
    let error = tree.root_node().has_error();
    if (plant == "fresh-clean" && !error) || (plant == "fresh-errors" && error) {
        parser.parse([b"\n", text].concat(), None).expect("a tree")
    } else {
        tree
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (name, path, edits, seed, dump) = match &args[..] {
        [_, name, path, edits, seed] => (name, path, edits, seed, None),
        [_, name, path, edits, seed, dump] => (name, path, edits, seed, Some(dump.parse().expect("DUMP"))),
        _ => panic!("usage: treesit-fuzz GRAMMAR FILE EDITS SEED [DUMP]"),
    };
    let (edits, seed): (usize, u64) = (edits.parse().expect("EDITS"), seed.parse().expect("SEED"));
    let plant = std::env::var("TREESIT_FUZZ_PERTURB").unwrap_or_default();
    let mut text = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    println!("grammar {}", if plant == "wrong-grammar" { "planted" } else { name.as_str() });
    let (mut parser, mut fresh_parser) = (Parser::new(), Parser::new());
    parser.set_language(&language(name)).expect("a compatible grammar");
    fresh_parser.set_language(&language(name)).expect("a compatible grammar");
    let mut tree = parser.parse(&text, None).expect("a tree");
    let fresh = fresh_parse(&mut fresh_parser, &text, &plant);
    let lines = Lines::new(&text);
    let looked = positions(&lines, 0, text.len(), seed, 0);
    report(0, State { tree: &tree, ranges: vec![], fresh, lines: &lines, positions: looked, micros: 0 }, dump, &plant, seed);
    let mut rng = Rng(seed.max(1));
    let mut at = text.len() / 2;
    let mut undo = Vec::new();
    for i in 1..=edits {
        let Edit { start, old_end, new } = next_edit(&mut rng, &text, &mut at, &mut undo);
        let (start_position, old_end_position) = (point(&text, start), point(&text, old_end));
        text.splice(start..old_end, new.iter().copied());
        let new_end = start + new.len();
        tree.edit(&InputEdit { start_byte: start, old_end_byte: old_end, new_end_byte: new_end, start_position,
                               old_end_position, new_end_position: point(&text, new_end) });
        let started = std::time::Instant::now();
        let new_tree = parser.parse(&text, Some(&tree)).expect("a tree");
        let micros = started.elapsed().as_micros();
        let fresh = fresh_parse(&mut fresh_parser, &text, &plant);
        if !(plant == "drop-row" && i == edits) {
            let lines = Lines::new(&text);
            let looked = positions(&lines, start, new_end, seed, i);
            let state = State { tree: &new_tree, ranges: changed(&tree, &new_tree), fresh, lines: &lines, positions: looked, micros };
            report(i, state, dump, &plant, seed);
        }
        tree = new_tree;
    }
    if plant == "exit" {
        std::process::exit(77);
    }
}
