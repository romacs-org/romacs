//! Forward list scans through the index (`SyntaxIndex::scan_lists`)
//! against the plain scanner (`Scanner::scan_lists`), each run as the
//! primitive runs it: a stop for `internal--syntax-propertize' answered by
//! a callback that moves the frontier and may write `syntax-table'
//! properties, a quit poll that does not quit resumed where it stopped.
//! Fixed cases put a leaf cut at the position each is about; a seeded
//! differential covers every count class, depths -2 to 3, both scanner
//! modes, frontiers and edits, and asserts the share the index folded.
use super::*;
use crate::syntax_scan::tests::{random_table, Rng, ALPHABET};
use crate::syntax_scan::{Halt, Options, Pos, Scan, Stopped, SCLOSE, SMATH, SOPEN, SWHITESPACE, SWORD};
use std::cell::Cell;
use std::collections::BTreeMap;

/// ASCII text whose `syntax-table' properties a callback may change
/// between runs, whose scans are asked to poll every POLLS steps.
struct Live<'a> {
    text: &'a [u8],
    table: &'a [i32; 128],
    properties: &'a [Option<i32>],
    polls: Option<u64>,
    // CELL(Plain): a test text's step count, no Value.
    steps: Cell<u64>,
    // CELL(Plain): whether its polls are held.
    held: Cell<bool>,
}
impl Text for Live<'_> {
    fn char_at(&self, at: Pos) -> u32 { self.text.get(at.byte).copied().map_or(0, u32::from) }
    fn inc(&self, at: Pos) -> Pos { Pos { char: at.char + 1, byte: at.byte + 1 } }
    fn dec(&self, at: Pos) -> Pos { Pos { char: at.char - 1, byte: at.byte - 1 } }
    fn pos(&self, charpos: i64) -> Pos {
        let c = charpos.clamp(1, i64::try_from(self.text.len()).expect("a test text") + 1);
        Pos { char: c, byte: usize::try_from(c - 1).expect("a position") }
    }
    fn syntax(&self, at: Pos) -> i32 {
        self.properties.get(at.byte).copied().flatten().unwrap_or_else(|| self.table_syntax(self.char_at(at)))
    }
    fn table_syntax(&self, c: u32) -> i32 { self.table.get(c as usize).copied().unwrap_or(SWORD) }
    fn stop_due(&self) -> bool {
        if self.held.get() {
            return false;
        }
        self.steps.set(self.steps.get() + 1);
        self.polls.is_some_and(|every| self.steps.get().is_multiple_of(every))
    }
    fn stop_uncounted(&self) -> bool { self.stop_due() }
    fn hold_polls(&self) -> u32 { u32::from(self.held.replace(true)) }
    fn release_polls(&self, held: u32) { self.held.set(held != 0); }
}

/// What a callback did: the frontier it left, and the ranges whose
/// `syntax-table' property it wrote.
struct Answer {
    done: i64,
    touched: Vec<(usize, usize)>,
}

/// A scan's inputs: FROM, COUNT, DEPTH and the scanner mode.
#[derive(Clone, Copy, Debug)]
struct Call {
    from: i64,
    count: i64,
    depth: i64,
    sexpflag: bool,
}

/// What a scan gave: its value or error, and the arguments of each stop
/// for Lisp in order.
#[derive(Debug, PartialEq, Eq)]
struct Ran {
    result: R<Option<i64>>,
    stops: Vec<(i64, i64)>,
}

/// The scan CALL over TEXT, through INDEX when there is one, as the
/// primitive runs it: each stop for Lisp answered by CALLBACK, which may
/// write PROPERTIES, each poll resumed. DONE is the frontier, or none
/// where properties are not honoured.
fn run(mut index: Option<&mut SyntaxIndex>, text: &[u8], table: &[i32; 128], properties: &mut Vec<Option<i32>>,
       (options, mut done, polls): (Options, Option<i64>, Option<u64>), call: Call,
       callback: &mut dyn FnMut(i64, i64, &mut Vec<Option<i32>>) -> Answer) -> Ran {
    let (mut resume, mut stops) = (None::<Scan>, Vec::new());
    let ppss = BTreeMap::new();
    let steps = Cell::new(0);
    for _ in 0..100_000 {
        let live = Live { text, table, properties, polls, steps: Cell::new(steps.get()), held: Cell::new(false) };
        let options = Options { propertize_done: done, ..options };
        let mut scanner = Scanner { begv: live.pos(1), zv: live.pos(i64::MAX), text: &live, options, ppss: &ppss, defun_start: None, modiff: 1 };
        let args = (call.from, call.count, call.depth);
        let result = match index.as_deref_mut() {
            Some(index) => {
                let context = Context { table: 1, writes: 0, escaped_ends: options.comment_end_can_be_escaped, properties: done.is_some() };
                index.scan_lists(&mut scanner, text.len(), context, resume.take(), args, call.sexpflag)
            }
            None => match resume.take() {
                Some(scan) => scanner.lists(scan, call.sexpflag),
                None => scanner.scan_lists(call.from, call.count, call.depth, call.sexpflag),
            },
        };
        steps.set(live.steps.get());
        match result {
            Ok(value) => return Ran { result: Ok(value), stops },
            Err(Stopped { halt: Halt::Poll, suspended }) => resume = suspended.map(|scan| *scan),
            Err(Stopped { halt: Halt::Propertize(arg, charpos), suspended }) => {
                stops.push((arg, charpos));
                let answer = callback(arg, charpos, properties);
                // parse_sexp_propertize's check: past the stop, or to the end.
                assert!(answer.done > charpos || answer.done >= position(text.len()), "a callback moves the frontier past {charpos}");
                done = Some(answer.done);
                if let Some(index) = index.as_deref_mut() {
                    for (start, end) in answer.touched { index.touch(start, end); }
                }
                resume = suspended.map(|scan| *scan);
            }
            Err(Stopped { halt, .. }) => return Ran { result: Err(halt), stops },
        }
    }
    panic!("{call:?}: no end after 100000 stops");
}

fn lists_table() -> [i32; 128] {
    let mut table = [SWORD; 128];
    for (c, syntax) in [(b'(', SOPEN), (b'[', SOPEN), (b'{', SOPEN), (b')', SCLOSE), (b']', SCLOSE), (b'}', SCLOSE),
                        (b'"', scan::SSTRING), (b'\\', scan::SESCAPE), (b'\'', scan::SQUOTE | (1 << 20)), (b'$', SMATH),
                        (b'/', scan::SPUNCT | (1 << 16) | (1 << 19)), (b'*', scan::SPUNCT | (1 << 17) | (1 << 18)),
                        (b';', scan::SCOMMENT), (b'\n', scan::SENDCOMMENT), (b' ', SWHITESPACE), (b'.', scan::SPUNCT)] {
        table[usize::from(c)] = syntax;
    }
    table
}

fn no_callback(arg: i64, _charpos: i64, _: &mut Vec<Option<i32>>) -> Answer {
    Answer { done: arg, touched: Vec::new() }
}

/// CALL over TEXT, plain and then twice through a fresh index of LEAF
/// characters, the second time warm: all three are EXPECTED, and the warm
/// one folded at least FOLDS spans of each kind.
fn fixed(text: &[u8], leaf: usize, call: Call, expected: R<Option<i64>>, folds: [u64; 2]) {
    let table = lists_table();
    let mut properties = vec![None; text.len()];
    let inputs = (Options::default(), None, None);
    let plain = run(None, text, &table, &mut properties, inputs, call, &mut no_callback);
    assert_eq!(plain.result, expected, "{call:?} plain over {:?}", String::from_utf8_lossy(text));
    let mut index = SyntaxIndex { leaf, ..SyntaxIndex::default() };
    for round in 0..2 {
        let before = index.stats.folds;
        let served = run(Some(&mut index), text, &table, &mut properties, inputs, call, &mut no_callback);
        assert_eq!(served.result, expected, "{call:?} round {round} over {:?}", String::from_utf8_lossy(text));
        if round == 1 {
            let folded = [index.stats.folds[0] - before[0], index.stats.folds[1] - before[1]];
            assert!(folded[0] >= folds[0] && folded[1] >= folds[1], "{call:?}: folded {folded:?}, wanted {folds:?}");
        }
    }
}

/// `(a)`: up one level and back, `(scan-lists 1 1 -1)` is 2, the opener
/// the first character of a leaf after leaves read at depth -1; and
/// `(scan-lists 2 1 1)` is 4, the closer the first of a leaf after leaves
/// carried at depth 1.
#[test]
fn a_list_entered_at_depth_minus_one_and_left_at_depth_one() {
    let pad = "x".repeat(32);
    let text = format!("{pad}(a)");
    fixed(text.as_bytes(), 8, Call { from: 1, count: 1, depth: -1, sexpflag: false }, Ok(Some(34)), [0, 3]);
    // The closer, at offset 33, starts the last leaf of 11.
    let text = format!("({pad})");
    fixed(text.as_bytes(), 11, Call { from: 2, count: 1, depth: 1, sexpflag: false }, Ok(Some(35)), [2, 0]);
}

/// `((`: `(scan-lists 1 1 -2)` is 3, the second opener rising to depth 0
/// across a cut between the two.
#[test]
fn two_openers_rise_to_depth_zero_across_a_cut() {
    let text = format!("{}((", "x".repeat(31));
    fixed(text.as_bytes(), 8, Call { from: 1, count: 1, depth: -2, sexpflag: false }, Ok(Some(34)), [0, 3]);
}

/// `($x)` with `$` as math, a cut between the opener and the math: as a
/// list the math is nothing, `(scan-lists 1 1 0)` is 5; as a sexp it opens
/// a level the closer does not end, and `(scan-sexps 1 1)` signals
/// Unbalanced parentheses from the opener to the end.
#[test]
fn math_carried_across_a_cut() {
    let text = format!("{}($x{}x)", " ".repeat(15), "x".repeat(40));
    let end = i64::try_from(text.len()).unwrap() + 1;
    fixed(text.as_bytes(), 16, Call { from: 1, count: 1, depth: 0, sexpflag: false }, Ok(Some(end)), [1, 0]);
    fixed(text.as_bytes(), 16, Call { from: 1, count: 1, depth: 0, sexpflag: true },
          Err(Halt::ScanError("Unbalanced parentheses", 16, end)), [1, 0]);
}

/// Forward counts over many top-level lists, and a match many leaves on:
/// before and after edits, every one folds.
#[test]
fn counts_over_many_lists_and_a_far_match() {
    let lists: String = (0..60).map(|i| format!("(f{i} (a) \"s(\" b)\n")).collect();
    let text = lists.as_bytes();
    for count in [5, 30, 59] {
        let plain = run(None, text, &lists_table(), &mut vec![None; text.len()], (Options::default(), None, None),
                        Call { from: 1, count, depth: 0, sexpflag: true }, &mut no_callback);
        fixed(text, 16, Call { from: 1, count, depth: 0, sexpflag: true }, plain.result, [0, 1]);
    }
    let far = format!("({})", lists);
    let end = i64::try_from(far.len()).unwrap() + 1;
    fixed(far.as_bytes(), 16, Call { from: 1, count: 1, depth: 0, sexpflag: true }, Ok(Some(end)), [1, 0]);
    fixed(far.as_bytes(), 16, Call { from: 1, count: 1, depth: 0, sexpflag: false }, Ok(Some(end)), [1, 0]);
}

/// A scan stopped for Lisp at every few leaves, whose callback, on its
/// second call, makes an opener the scan already stepped over punctuation,
/// with a forced quit poll that does not quit every seven steps: the
/// stops, their arguments and the value are the plain scanner's, and the
/// index folded below each frontier. A second scan reads the opener anew.
#[test]
fn a_callback_that_rewrites_an_opener_already_read() {
    let body: String = (0..40).map(|i| format!("(g{i} \"s\" 'x) ")).collect();
    let text = format!("(a ({body}) ({body}) b) tail");
    let text = text.as_bytes();
    let table = lists_table();
    /// Frontiers 70 characters on; the second call makes the opener at
    /// offset 3, which the scan entered first, punctuation.
    fn callback(calls: &Cell<u32>) -> impl FnMut(i64, i64, &mut Vec<Option<i32>>) -> Answer + '_ {
        move |arg, _charpos, properties| {
            calls.set(calls.get() + 1);
            let mut touched = Vec::new();
            if calls.get() == 2 {
                properties[3] = Some(scan::SPUNCT);
                touched.push((3, 4));
            }
            Answer { done: (arg + 70).min(position(properties.len())), touched }
        }
    }
    let call = Call { from: 1, count: 1, depth: 0, sexpflag: true };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    // Warm, with nothing left to propertize.
    let mut warm = vec![None; text.len()];
    run(Some(&mut index), text, &table, &mut warm, (Options::default(), Some(i64::MAX), None), call, &mut no_callback);
    let (plain_calls, served_calls) = (Cell::new(0), Cell::new(0));
    let mut plain_properties = vec![None; text.len()];
    let plain = run(None, text, &table, &mut plain_properties, (Options::default(), Some(1), Some(7)), call, &mut callback(&plain_calls));
    let mut properties = vec![None; text.len()];
    let before = index.stats.folds;
    let served = run(Some(&mut index), text, &table, &mut properties, (Options::default(), Some(1), Some(7)), call, &mut callback(&served_calls));
    assert_eq!(served, plain);
    assert!(index.stats.folds[0] > before[0], "{:?}: no excursion folded below a frontier", index.stats);
    let again = |index: Option<&mut SyntaxIndex>, properties: &mut Vec<Option<i32>>| {
        run(index, text, &table, properties, (Options::default(), Some(i64::MAX), None), call, &mut no_callback)
    };
    assert_eq!(again(Some(&mut index), &mut properties), again(None, &mut plain_properties));
}

/// A quit poll at a scan's entry stops it before it reads or keeps
/// anything, warm summaries or not; the next scan is exact.
#[test]
fn a_quit_on_entry_publishes_nothing() {
    let body: String = (0..80).map(|i| format!("(h{i} x) ")).collect();
    let text = format!("({body})");
    let text = text.as_bytes();
    let table = lists_table();
    let call = Call { from: 1, count: 1, depth: 0, sexpflag: true };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let mut properties = vec![None; text.len()];
    let warm = run(Some(&mut index), text, &table, &mut properties, (Options::default(), None, None), call, &mut no_callback);
    let ppss = BTreeMap::new();
    let live = Live { text, table: &table, properties: &properties, polls: Some(1), steps: Cell::new(0), held: Cell::new(false) };
    let mut scanner = Scanner { begv: live.pos(1), zv: live.pos(i64::MAX), text: &live, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
    let (scans, folds) = (index.stats.leaf_scans, index.stats.folds);
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
    let stopped = index.scan_lists(&mut scanner, text.len(), context, None, (1, 1, 0), true);
    assert!(matches!(stopped, Err(Stopped { halt: Halt::Poll, .. })), "a poll at entry stops the scan");
    assert_eq!((index.stats.leaf_scans, index.stats.folds), (scans, folds), "nothing read or folded");
    assert_eq!(run(Some(&mut index), text, &table, &mut properties, (Options::default(), None, None), call, &mut no_callback), warm);
    assert!(index.stats.folds[0] > folds[0], "the warm scan after it folds");
}

/// A scan of CALL over TEXT through INDEX alone, polling as POLLS says:
/// its end.
fn indexed(index: &mut SyntaxIndex, text: &[u8], table: &[i32; 128], (polls, done): (Option<u64>, Option<i64>),
           resume: Option<Scan>, call: Call) -> Result<Option<i64>, Stopped<Box<Scan>>> {
    let properties = vec![None; text.len()];
    let ppss = BTreeMap::new();
    let live = Live { text, table, properties: &properties, polls, steps: Cell::new(0), held: Cell::new(false) };
    let options = Options { propertize_done: done, ..Options::default() };
    let mut scanner = Scanner { begv: live.pos(1), zv: live.pos(i64::MAX), text: &live, options, ppss: &ppss, defun_start: None, modiff: 1 };
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: done.is_some() };
    index.scan_lists(&mut scanner, text.len(), context, resume, (call.from, call.count, call.depth), call.sexpflag)
}

/// A cold scan whose quit probe comes due right after its first fold, the
/// fold whose leaf summary it made then: the run publishes nothing. Its
/// counters are as they were but for the quit, it goes on from where it
/// stopped to the scanner's value, and the next scan makes every summary
/// a scan that never stopped makes, that first one included.
#[test]
fn a_quit_after_building_a_fold_publishes_nothing() {
    let text = format!("({})", "x".repeat(200));
    let (text, table) = (text.as_bytes(), lists_table());
    let call = Call { from: 1, count: 1, depth: 0, sexpflag: true };
    let mut control = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let expected = indexed(&mut control, text, &table, (None, None), None, call).ok();
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let before = index.stats;
    // The probe's first call is on entry, its second after the first fold.
    let stopped = indexed(&mut index, text, &table, (Some(2), None), None, call);
    let Err(Stopped { halt: Halt::Poll, suspended: Some(scan) }) = stopped else { panic!("a poll after the first fold") };
    assert!(scan.from.char > 16, "the scan stopped after its first fold, at {}", scan.from.char);
    let after = index.stats;
    assert_eq!((after.leaf_scans, after.folds, after.skipped, after.list_served), (before.leaf_scans, before.folds, before.skipped, before.list_served),
               "the stopped run's counts are dropped");
    assert_eq!(after.list_declined[Decline::Quit as usize], before.list_declined[Decline::Quit as usize] + 1);
    assert_eq!(indexed(&mut index, text, &table, (None, None), Some(*scan), call).ok(), expected, "it goes on where it stopped");
    let mut again = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    indexed(&mut again, text, &table, (Some(2), None), None, call).ok();
    let scans = again.stats.leaf_scans;
    indexed(&mut again, text, &table, (None, None), None, call).ok();
    assert_eq!(again.stats.leaf_scans - scans, control.stats.leaf_scans, "the stopped run's summary is made again");
}

/// The fields of a forward scan's continuation the scanner reads next.
fn going_on(scan: &Scan) -> (crate::syntax_scan::ScanAt, i64, i64, i64, i64, bool, i64) {
    (scan.at, scan.reached().char, scan.count, scan.depth, scan.min_depth, scan.mathexit, scan.last_good)
}

/// A warm scan whose frontier is its first cut, with a quit poll that does
/// not quit coming due at the fold that reaches it: the stop for
/// `internal--syntax-propertize' comes first, with the scanner's arguments
/// and continuation, and the poll after the callback.
#[test]
fn a_frontier_on_a_cut_comes_before_a_poll_due_there() {
    let text = format!("({})", "x".repeat(100));
    let (text, table) = (text.as_bytes(), lists_table());
    let call = Call { from: 1, count: 1, depth: 0, sexpflag: true };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    indexed(&mut index, text, &table, (None, Some(i64::MAX)), None, call).ok();
    // The cut after the first leaf, at position 17, is the frontier.
    let plain = {
        let properties = vec![None; text.len()];
        let ppss = BTreeMap::new();
        let live = Live { text, table: &table, properties: &properties, polls: None, steps: Cell::new(0), held: Cell::new(false) };
        let options = Options { propertize_done: Some(17), ..Options::default() };
        let mut scanner = Scanner { begv: live.pos(1), zv: live.pos(i64::MAX), text: &live, options, ppss: &ppss, defun_start: None, modiff: 1 };
        let Err(Stopped { halt, suspended: Some(scan) }) = scanner.scan_lists(1, 1, 0, true) else { panic!("a stop at the frontier") };
        (halt, going_on(&scan))
    };
    assert_eq!(plain.0, Halt::Propertize(18, 17));
    let folds = index.stats.folds;
    let mut events = Vec::new();
    // The probe's second call, after the first fold, is due; so is its fourth.
    let first = indexed(&mut index, text, &table, (Some(2), Some(17)), None, call);
    let Err(Stopped { halt, suspended: Some(scan) }) = first else { panic!("a stop") };
    assert!(index.stats.folds[1] > folds[1] || index.stats.folds[0] > folds[0], "the first leaf folded");
    assert_eq!((halt, going_on(&scan)), plain, "the stop and the scanner's continuation");
    events.push("propertize");
    let next = indexed(&mut index, text, &table, (Some(2), Some(i64::MAX)), Some(*scan), call);
    if matches!(next, Err(Stopped { halt: Halt::Poll, .. })) {
        events.push("poll");
    }
    assert_eq!(events, ["propertize", "poll"]);
}

/// Five tables over one text, one of them making a character next to a
/// cut an escape: four warm, the oldest used again, then the fifth. The
/// one used least recently gives its slot up, the one used again keeps its
/// summaries, and every scan, after an edit at a cut and after a write to
/// a table too, is the scanner's.
#[test]
fn a_fifth_table_takes_the_least_recently_used_slot() {
    let mut text = b"(ab (cd) e) ".repeat(40);
    let leaf = 16;
    let tables: Vec<[i32; 128]> = (0..5).map(|k| {
        let mut table = lists_table();
        // Table 4 makes `e', a leaf's last character in places, an escape.
        if k == 4 { table[usize::from(b'e')] = scan::SESCAPE; }
        if k == 1 { table[usize::from(b'c')] = scan::SSYMBOL; }
        table
    }).collect();
    let mut index = SyntaxIndex { leaf, ..SyntaxIndex::default() };
    let call = Call { from: 1, count: 100, depth: 0, sexpflag: true };
    let ppss = BTreeMap::new();
    let mut scan = |index: &mut SyntaxIndex, text: &[u8], k: usize, writes: u64| {
        let properties = vec![None; text.len()];
        let live = Live { text, table: &tables[k], properties: &properties, polls: None, steps: Cell::new(0), held: Cell::new(false) };
        let mut scanner = Scanner { begv: live.pos(1), zv: live.pos(i64::MAX), text: &live, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
        let expected = scanner.scan_lists(call.from, call.count, call.depth, true).map_err(|stopped| stopped.halt);
        let context = Context { table: u64::try_from(k).unwrap() + 1, writes, escaped_ends: false, properties: false };
        let scans = index.stats.leaf_scans;
        let actual = index.scan_lists(&mut scanner, text.len(), context, None, (call.from, call.count, call.depth), true).map_err(|stopped| stopped.halt);
        assert_eq!(actual, expected, "table {k}");
        index.stats.leaf_scans - scans
    };
    for k in 0..4 { scan(&mut index, &text, k, 0); }
    assert_eq!(scan(&mut index, &text, 0, 0), 0, "table 0 warm");
    assert_eq!(index.stats.context_evictions, 0);
    assert!(scan(&mut index, &text, 4, 0) > 0, "table 4 cold");
    assert_eq!(index.stats.context_evictions, 1);
    assert_eq!(scan(&mut index, &text, 0, 0), 0, "table 0, used again before table 4 came, keeps its summaries");
    assert!(scan(&mut index, &text, 1, 0) > 0, "table 1, least recently used, gave its slot up");
    // An edit at the cut after the first leaf, then a write to table 0.
    text.splice(16..16, b"e(".iter().copied());
    index.edit(16, 16, 2);
    for k in [4, 0, 2, 3, 1] { scan(&mut index, &text, k, 0); }
    let invalidations = index.stats.context_invalidations;
    scan(&mut index, &text, 0, 1);
    assert_eq!(index.stats.context_invalidations, invalidations + 1);
}

/// Text of lists, strings, comments, math, escapes and words, as chunks.
fn list_text(rng: &mut Rng, chunks: usize) -> Vec<u8> {
    const CHUNKS: [&[u8]; 14] = [b"(a [b] {c}) ", b"/*)*/ ", b"$$a$$ ", b"\"a\\\"b(\"", b"'x ", b"a_* / ) */ ",
        b"(((( ", b")))) ", b"\\( ", b"; c (\n", b"word ", b"( ", b") ", b"(ab (cd) \"e\" 'f)\n"];
    let mut text = Vec::new();
    for _ in 0..chunks {
        if rng.chance(10) {
            text.push(ALPHABET[rng.below(ALPHABET.len())]);
        } else {
            text.extend_from_slice(CHUNKS[rng.below(CHUNKS.len())]);
        }
    }
    text
}

/// A random call: every count class, depths -2 to 3, both modes.
fn random_call(rng: &mut Rng, len: usize) -> Call {
    let count = match rng.below(4) {
        0 => 1,
        1 => 2,
        2 => 1 + i64::try_from(rng.below(6)).unwrap(),
        _ => i64::try_from(len).unwrap() + 2,
    };
    let from = if rng.chance(30) { 1 } else { position(rng.below(len + 1)) };
    Call { from, count, depth: i64::try_from(rng.below(6)).unwrap() - 2, sexpflag: rng.chance(60) }
}

/// A callback for CASE's Nth call: the frontier moves on by up to a few
/// leaves, and now and then it writes a property ahead or behind.
fn random_callback(seed: u64, leaf: usize, table: &[i32; 128], calls: &Cell<u64>)
                   -> impl FnMut(i64, i64, &mut Vec<Option<i32>>) -> Answer {
    move |arg: i64, _charpos: i64, properties: &mut Vec<Option<i32>>| {
        calls.set(calls.get() + 1);
        let mut rng = Rng(seed ^ calls.get().wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let len = properties.len();
        let done = (arg + i64::try_from(1 + rng.below(4 * leaf)).unwrap()).min(position(len));
        let mut touched = Vec::new();
        if rng.chance(30) && len > 0 {
            // Ahead of the stop, or (one time in four) over text it read.
            let ahead = usize::try_from(arg - 1).unwrap().min(len.saturating_sub(1));
            let start = if rng.chance(25) { rng.below(ahead + 1) } else { ahead + rng.below(len - ahead) };
            let end = (start + 1 + rng.below(4)).min(len);
            let syntax = rng.chance(70).then(|| table[usize::from(b"()[]\"$ ;\nab"[rng.below(11)])]);
            properties[start..end].fill(syntax);
            touched.push((start, end));
        }
        Answer { done: done.max(arg), touched }
    }
}

#[test]
fn randomized_forward_scans_match_the_plain_scanner() {
    randomized_forward(0x5_1a_f01d_5ca1_e5, 160);
}

#[test]
#[ignore = "larger forward-scan differential run"]
fn randomized_forward_scans_extended() {
    randomized_forward(0x0f01_d1e5_7a11_0c4d, 2000);
}

/// Seeded differential: random texts and tables, random edits and
/// property writes between rounds, frontiers whose callbacks write
/// properties, forced polls; the index's run is the plain scanner's, stop
/// for stop, and it folded a fair share of what it crossed.
fn randomized_forward(seed: u64, cases: u64) {
    let mut rng = Rng(seed);
    let (mut calls, mut stopped, mut skipped, mut folds) = (0u64, 0u64, 0u64, [0u64; 2]);
    for case in 0..cases {
        let table = if rng.chance(75) { lists_table() } else { random_table(&mut rng) };
        let options = Options {
            ignore_comments: rng.chance(60),
            comment_end_can_be_escaped: rng.chance(30),
            multibyte_syntax_as_symbol: rng.chance(30),
            ..Options::default()
        };
        let leaf = 4 + rng.below(40);
        let chunks = 40 + rng.below(260);
        let mut text = list_text(&mut rng, chunks);
        let mut properties: Vec<Option<i32>> = vec![None; text.len()];
        let mut index = SyntaxIndex { leaf, ..SyntaxIndex::default() };
        for round in 0..8 {
            if round > 0 && rng.chance(70) && !text.is_empty() {
                let start = rng.below(text.len() + 1);
                let end = (start + rng.below(5)).min(text.len());
                let pieces = rng.below(3);
                let inserted = list_text(&mut rng, pieces);
                index.edit(start, end, inserted.len());
                properties.splice(start..end, std::iter::repeat_n(None, inserted.len()));
                text.splice(start..end, inserted);
            }
            let call = random_call(&mut rng, text.len());
            // A frontier within a few leaves of FROM half the time: most calls
            // end before one further on.
            // A frontier within a few leaves of FROM most of the time: most
            // calls end before one further on. Now and then one before FROM,
            // or on a cut of the fresh partition.
            let done = match rng.below(8) {
                0 => None,
                1 => Some(i64::MAX),
                2 => Some((call.from - i64::try_from(rng.below(2 * leaf)).unwrap()).max(1)),
                3 => Some(position(leaf * (1 + rng.below(text.len() / leaf + 1)))),
                _ => Some(call.from.max(1) + i64::try_from(rng.below(4 * leaf + 1)).unwrap()),
            };
            let polls = rng.chance(25).then(|| 3 + rng.next() % 60);
            let callback_seed = rng.next();
            let (plain_calls, served_calls) = (Cell::new(0), Cell::new(0));
            let mut plain_properties = properties.clone();
            let plain = run(None, &text, &table, &mut plain_properties, (options, done, polls), call,
                            &mut random_callback(callback_seed, leaf, &table, &plain_calls));
            let before = (index.stats.folds, index.stats.skipped);
            let served = run(Some(&mut index), &text, &table, &mut properties, (options, done, polls), call,
                             &mut random_callback(callback_seed, leaf, &table, &served_calls));
            assert_eq!(served, plain, "seed {seed:#x} case {case} round {round}: {call:?}, leaf {leaf}, frontier {done:?}, polls {polls:?}, text {:?}",
                       String::from_utf8_lossy(&text));
            assert_eq!(properties, plain_properties);
            calls += 1;
            stopped += u64::from(!served.stops.is_empty());
            skipped += index.stats.skipped - before.1;
            folds = [folds[0] + index.stats.folds[0] - before.0[0], folds[1] + index.stats.folds[1] - before.0[1]];
        }
    }
    println!("forward scans: {calls} calls, {stopped} stopped for Lisp, folds {folds:?}, {skipped} characters skipped");
    assert!(stopped * 5 > calls, "{stopped} of {calls} calls stopped for Lisp");
    assert!(folds[0] * 10 > calls && folds[1] * 10 > calls && skipped > 20 * calls,
            "folds {folds:?} and {skipped} characters skipped over {calls} calls");
}
