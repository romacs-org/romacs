//! Served parses (`SyntaxIndex::parse`) against the plain scanner.
use super::*;
use crate::syntax_scan::tests::{nested_text, random_table, random_text, with_parens, Plain, Rng, ALPHABET};
use crate::syntax_scan::{Options, ParseState, Pos, Scanner, SCLOSE, SCOMMENT, SENDCOMMENT, SESCAPE, SOPEN, SSTRING, SWHITESPACE, SWORD};
use std::collections::BTreeMap;

/// PLAIN with a syntax per position where PROPERTIES has one, as
/// `syntax-table' properties honoured.
pub(super) struct Propertied<'a, F: Fn(u8) -> i32> {
    pub(super) plain: &'a Plain<F>,
    pub(super) properties: &'a [Option<i32>],
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

/// The leaves a served parse of FROM..TO from START scans between its
/// first and its last when every other one has a summary: whole runs
/// of unsafe ends, each with the leaf that ends it, up to the first
/// leaf after which no key resumes the state, where it scans on to TO.
fn unsafe_runs<T: Text>(index: &SyntaxIndex, scanner: &mut Scanner<'_, T>, start: &ParseState, from: i64, to: i64) -> u64 {
    let root = index.root.as_ref().expect("a parse built it");
    let bit = 1 << index.current.expect("a parse chose a context");
    let (last_start, ..) = locate(root, offset((to - 1).max(from)), bit);
    let (mut at, mut leaves) = (safe_end(root, offset(from), bit).0, 0);
    while at < last_start {
        let (next, run) = safe_end(root, at, bit);
        let mut there = start.clone();
        scanner.scan_sexps_forward(&mut there, scanner.text.pos(from), position(next), i64::MIN, false, 0).unwrap();
        if next > last_start || !resumable(&there) { break; }
        if run > 1 { leaves += run; }
        at = next;
    }
    leaves
}

/// Served parses, after random edits, in random tables, from the start
/// or from a state Lisp gives, are the plain scanner's, field for
/// field, in texts that close past the bottom level and nest past
/// GNU's cap among them. A leaf is scanned outside a summary only in a
/// run of unsafe ends, or where a comment no key resumes cuts the
/// summaries short, which takes escaped comment ends.
#[test]
fn summaries_serve_what_the_scanner_gives() {
    let mut rng = Rng(0x005e_ed0f_5ca1_ab1e);
    let mut stats = Stats::default();
    let (mut bottomed, mut capped) = (0, 0);
    for case in 0..300 {
        let nested = case % 3 == 0;
        let fresh = |rng: &mut Rng| if nested { with_parens(random_table(rng)) } else { random_table(rng) };
        let mut table = fresh(&mut rng);
        let len = rng.below(400);
        let mut text = if nested { nested_text(&mut rng, len) } else { random_text(&mut rng, len) };
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
                table = fresh(&mut rng);
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
                let plain_leaves = index.stats.plain_leaves;
                if index.parse(&mut scanner, text.len(), context.clone(), from, to, &mut served).unwrap() {
                    assert_eq!(index.stats.plain_leaves - plain_leaves, unsafe_runs(&index, &mut scanner, &start, from, to),
                               "case {case} round {round}: leaves scanned, leaves in unsafe runs");
                } else {
                    assert_eq!(served, start, "a parse not served leaves its state");
                    scanner.scan_sexps_forward(&mut served, plain.pos(from), to, i64::MIN, false, 0).unwrap();
                }
                assert_eq!(served, expected, "case {case} round {round}: {:?} {from}..{to} from {start:?}, leaf {}, properties {properties:?}",
                           String::from_utf8_lossy(&text), index.leaf);
                // Closed past the bottom level, or opened at the cap, in FROM..TO itself.
                let levels = i64::try_from(start.levelstarts.len()).expect("few levels");
                bottomed += usize::from(expected.mindepth < start.depth - levels && expected.maxdepth < 100);
                capped += usize::from(levels + expected.maxdepth - start.depth >= 100);
            }
        }
        let s = index.stats;
        (stats.served, stats.leaf_scans, stats.composed, stats.plain_leaves) =
            (stats.served + s.served, stats.leaf_scans + s.leaf_scans, stats.composed + s.composed, stats.plain_leaves + s.plain_leaves);
    }
    // The summaries did the work, not the scans at the ends.
    assert!(stats.served > 1000 && stats.composed > 1000 && stats.leaf_scans > 1000, "{stats:?}");
    assert!(bottomed > 1000 && capped > 300, "{bottomed} parses closed past the bottom level, {capped} reached the cap");
    let _ = ALPHABET;
}

/// Texts of parens and two kinds of string whose first character is by
/// turns a word, one string's start and the other's: every leaf below
/// is entered in three keys in turn, each with its own count of levels
/// where the leaf closes past the bottom level or reaches the cap. A
/// node keeps all six summaries, so after each key's first round no
/// leaf is scanned again, and every parse is the scanner's.
#[test]
fn three_keys_by_turns_with_exact_counts_are_served() {
    let table = |c: u8| match c { b'(' => SOPEN, b')' => SCLOSE, b'"' | b'\'' => SSTRING, b' ' => SWHITESPACE, _ => SWORD };
    let mut rng = Rng(0x7a1_10fc_a5e5);
    let (mut served, mut bottomed, mut capped, mut remade, mut kept) = (0, 0, 0, 0, 0);
    for case in 0..150 {
        let len = 60 + rng.below(400);
        let run = if rng.chance(50) { 90 + rng.below(40) } else { 0 };
        let mut text: Vec<u8> = (0..len).map(|i| if i <= run { b'(' } else { b"))))( x\"'"[rng.below(9)] }).collect();
        let mut index = SyntaxIndex { leaf: 1 + rng.below(12), ..SyntaxIndex::default() };
        let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
        for round in 0..12 {
            text[0] = b"x\"'"[round % 3];
            index.edit(0, 1, 1);
            let plain = Plain { text: text.clone(), table };
            let ppss = BTreeMap::new();
            let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
            let scans = index.stats.leaf_scans;
            for to in [position(len), position(rng.below(len + 1))] {
                let mut expected = ParseState::default();
                scanner.scan_sexps_forward(&mut expected, plain.pos(1), to, i64::MIN, false, 0).unwrap();
                let mut state = ParseState::default();
                if index.parse(&mut scanner, len, context.clone(), 1, to, &mut state).unwrap() {
                    served += 1;
                } else {
                    scanner.scan_sexps_forward(&mut state, plain.pos(1), to, i64::MIN, false, 0).unwrap();
                }
                assert_eq!(state, expected, "case {case} round {round}: {:?} to {to}, leaf {}", String::from_utf8_lossy(&text), index.leaf);
                bottomed += usize::from(expected.mindepth < 0 && expected.maxdepth < 100);
                capped += usize::from(expected.maxdepth >= 100);
            }
            // Each key was here three rounds ago.
            if round >= 3 {
                (remade, kept) = (remade + usize::from(index.stats.leaf_scans > scans), kept + 1);
            }
        }
        assert_eq!((index.stats.plain_leaves, index.stats.declined[Decline::Unresumable as usize]), (0, 0), "case {case}");
    }
    assert!(served > 3000 && bottomed > 2000 && capped > 200 && kept > 1000, "served {served}, bottomed {bottomed}, capped {capped}, rounds {kept}");
    assert_eq!(remade, 0, "rounds after each key's first that scanned a leaf");
}

/// One leaf that opens to GNU's cap and then closes past the bottom
/// level: no general summary carries a state across it, its exact one
/// does, and parses across it are the scanner's, the second from kept
/// summaries.
#[test]
fn a_leaf_that_reaches_the_cap_and_closes_past_the_bottom_is_served() {
    let middle = [b"(".repeat(120), b")".repeat(130), b" ".repeat(50)].concat();
    let text: Vec<u8> = [b"a ".repeat(150), middle, b"b ".repeat(300)].concat();
    let plain = Plain { text: text.clone(), table: |c: u8| match c { b'(' => SOPEN, b')' => SCLOSE, b' ' => SWHITESPACE, _ => SWORD } };
    let ppss = BTreeMap::new();
    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
    let mut index = SyntaxIndex { leaf: 300, ..SyntaxIndex::default() };
    let end = position(text.len());
    let mut expected = ParseState::default();
    scanner.scan_sexps_forward(&mut expected, plain.pos(1), end, i64::MIN, false, 0).unwrap();
    assert!(expected.maxdepth > 100 && expected.mindepth < 0, "{expected:?}");
    for round in 0..2 {
        let scans = index.stats.leaf_scans;
        let mut served = ParseState::default();
        assert!(index.parse(&mut scanner, text.len(), context.clone(), 1, end, &mut served).unwrap());
        assert_eq!(served, expected);
        assert_eq!(index.stats.plain_leaves, 0);
        assert_eq!(round == 1, index.stats.leaf_scans == scans, "kept summaries serve the second parse");
    }
}

/// Nesting past GNU's 100 levels over many leaves, then closed past
/// the bottom level: parses to the end and back into the middle are
/// the scanner's, by summaries alone, and a second time by kept ones.
#[test]
fn nesting_past_the_level_cap_is_served() {
    let text: Vec<u8> = [b"(a ".repeat(150), b"b) ".repeat(170), b"(c) ".repeat(20)].concat();
    let plain = Plain { text: text.clone(), table: |c: u8| match c { b'(' => SOPEN, b')' => SCLOSE, b' ' => SWHITESPACE, _ => SWORD } };
    let ppss = BTreeMap::new();
    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let places = [position(text.len()), 400, 300, 700, 900, position(text.len())];
    for round in 0..2 {
        let scans = index.stats.leaf_scans;
        for to in places {
            let mut expected = ParseState::default();
            scanner.scan_sexps_forward(&mut expected, plain.pos(1), to, i64::MIN, false, 0).unwrap();
            let mut served = ParseState::default();
            assert!(index.parse(&mut scanner, text.len(), context.clone(), 1, to, &mut served).unwrap());
            assert_eq!(served, expected, "to {to}");
        }
        assert_eq!(index.stats.plain_leaves, 0);
        assert_eq!(round == 1, index.stats.leaf_scans == scans, "kept summaries serve the second round");
    }
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
/// unsafe ends: a parse scans across them and takes summaries again
/// after, and is the scanner's.
#[test]
fn a_long_comment_starter_run_is_crossed() {
    // `-' is both characters of "--", a comment starter.
    let table = |c: u8| match c { b'-' => 1 | 1 << 16 | 1 << 17, b'\n' => crate::syntax_scan::SENDCOMMENT, _ => crate::syntax_scan::SWORD };
    let text: Vec<u8> = [b"ab\n".repeat(20), vec![b'-'; 300], b"cd\n".repeat(20)].concat();
    let plain = Plain { text: text.clone(), table };
    let ppss = BTreeMap::new();
    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let end = position(text.len());
    let mut expected = ParseState::default();
    scanner.scan_sexps_forward(&mut expected, plain.pos(1), end, i64::MIN, false, 0).unwrap();
    let mut served = ParseState::default();
    assert!(index.parse(&mut scanner, text.len(), context, 1, end, &mut served).unwrap());
    assert_eq!(served, expected);
    let stats = index.stats;
    // The run's leaves and the one that ends it, and no other.
    assert_eq!((stats.plain_leaves, stats.declined[Decline::Unresumable as usize]), (unsafe_runs(&index, &mut scanner, &ParseState::default(), 1, end), 0));
    assert!((19..=21).contains(&stats.plain_leaves) && stats.leaf_scans >= 5, "{stats:?}");
}

/// A nested comment an escaped end used up never ends, and no key
/// resumes it. GNU's resume makes one from a Lisp state between the
/// escape and the end: that parse is the scanner's.
#[test]
fn a_spent_comment_is_not_served() {
    const NESTED: i32 = 1 << 22;
    let table = |c: u8| match c { b'{' => SCOMMENT | NESTED, b'}' => SENDCOMMENT | NESTED, b'\\' => SESCAPE, b' ' => SWHITESPACE, _ => SWORD };
    let text: Vec<u8> = [b"ab ".repeat(40), b"{ x \\} y ".to_vec(), b"cd ".repeat(40)].concat();
    let plain = Plain { text: text.clone(), table };
    let ppss = BTreeMap::new();
    let options = Options { comment_end_can_be_escaped: true, ..Options::default() };
    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
    let context = Context { table: 1, writes: 0, escaped_ends: true, properties: false };
    let mut index = SyntaxIndex { leaf: 16, ..SyntaxIndex::default() };
    let (from, end) = (position(125), position(text.len()));
    let mut start = ParseState::default();
    scanner.scan_sexps_forward(&mut start, plain.pos(1), from, i64::MIN, false, 0).unwrap();
    let start = listed(&start);
    let mut expected = start.clone();
    scanner.scan_sexps_forward(&mut expected, plain.pos(from), end, i64::MIN, false, 0).unwrap();
    assert!(expected.continuation.is_some_and(|c| c.spent));
    let mut served = start.clone();
    assert!(!index.parse(&mut scanner, text.len(), context, from, end, &mut served).unwrap());
    assert_eq!((served, index.stats.declined[Decline::Unresumable as usize]), (start, 1));
}
