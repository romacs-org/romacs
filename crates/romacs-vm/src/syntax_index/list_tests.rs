//! Unbounded forward list scans through the index against the plain scanner.
use super::*;
use super::tests::Propertied;
use crate::syntax_scan::tests::{Plain, Rng, random_table, random_text, ALPHABET};
use std::collections::BTreeMap;

/// `scan-sexps' from FROM over COUNT sexps, through INDEX.
fn lists<T: Text>(index: &mut SyntaxIndex, scanner: &mut Scanner<'_, T>, chars: usize, context: Context,
                  (from, count): (i64, i64)) -> Option<R<Option<i64>>> {
    Some(index.scan_lists(scanner, chars, context, None, (from, count, 0), true).map_err(|stopped| stopped.halt))
}

fn dialect(byte: u8) -> i32 {
    match byte {
        b'(' | b'[' | b'{' => scan::SOPEN,
        b')' | b']' | b'}' => scan::SCLOSE,
        b'"' => scan::SSTRING,
        b'\\' => scan::SESCAPE,
        b'\'' => scan::SQUOTE | (1 << 20),
        b'$' => scan::SMATH,
        b'/' => scan::SPUNCT | (1 << 16) | (1 << 19),
        b'*' => scan::SPUNCT | (1 << 17) | (1 << 18),
        b'_' => scan::SSYMBOL | (1 << 16),
        b' ' | b'\n' => scan::SWHITESPACE,
        _ => scan::SWORD,
    }
}

#[test]
fn a_warm_list_scan_reads_only_edited_and_end_leaves() {
    let reads = std::cell::Cell::new(0);
    let table = |c| {
        reads.set(reads.get() + 1);
        dialect(c)
    };
    let mut text = Plain { text: b"(ab [cd] ef)\n".repeat(9000), table };
    let mut index = SyntaxIndex::default();
    let context = Context { table: 1, writes: 0, escaped_ends: false, properties: false };
    let ppss = BTreeMap::new();
    let scan = |index: &mut SyntaxIndex, text: &Plain<_>| {
        let options = scan::Options::default();
        let mut scanner = Scanner { text, begv: text.pos(1), zv: text.pos(i64::MAX), options, ppss: &ppss, defun_start: None, modiff: 0 };
        lists(index, &mut scanner, text.text.len(), context.clone(), (1, i64::MAX))
    };
    assert_eq!(scan(&mut index, &text), Some(Ok(None)));
    text.text.insert(99, b'x');
    index.edit(99, 99, 1);
    reads.set(0);
    assert_eq!(scan(&mut index, &text), Some(Ok(None)));
    assert!(reads.get() <= 8 * LEAF, "{} syntax reads after one local edit", reads.get());
}

#[test]
fn indexed_list_spans_match_plain_scans_before_and_after_edits() {
    let mut rng = Rng(0x0e22_1ca1);
    let ppss = BTreeMap::new();
    let mut served = 0;
    let chunks: &[&[u8]] = &[b"(a [b] {c}) ", b"/*)*/ ", b"$$a$$ ", b"\"a\\\"b\"", b"'x ", b"a_* / ) */ "];
    for ignore_comments in [false, true] {
        for seed in 0..160 {
            let mut text = Plain { text: vec![], table: dialect as fn(u8) -> i32 };
            for _ in 0..36 {
                text.text.extend_from_slice(chunks[rng.below(if seed % 4 == 0 { 6 } else { 5 })]);
            }
            let mut index = SyntaxIndex { leaf: 9 + rng.below(24), ..SyntaxIndex::default() };
            let options = scan::Options { ignore_comments, comment_end_can_be_escaped: seed % 2 == 0, ..scan::Options::default() };
            let context = Context { table: 0, writes: 0, escaped_ends: options.comment_end_can_be_escaped, properties: false };
            for edit in 0..4 {
                if edit > 0 {
                    let at = rng.below(text.text.len());
                    let byte = b" a(]\\"[rng.below(5)];
                    text.text.insert(at, byte);
                    index.edit(at, at, 1);
                }
                let end = text.pos(i64::MAX);
                let from = if edit == 0 { 1 } else { position(rng.below(text.text.len() / 2)) };
                let mut scanner = Scanner { text: &text, begv: text.pos(1), zv: end, options, ppss: &ppss, defun_start: None, modiff: 0 };
                let expected = scanner.scan_lists(from, end.char, 0, true).map_err(|stopped| stopped.halt);
                for _ in 0..2 {
                    if let Some(actual) = lists(&mut index, &mut scanner, text.text.len(), context.clone(), (from, end.char)) {
                        assert_eq!(actual, expected, "seed {seed}, edit {edit}, from {from}, text {:?}", String::from_utf8_lossy(&text.text));
                        served += 1;
                    }
                }
            }
        }
    }
    assert!(served >= 600, "only {served} list scans used the index");
}

#[test]
fn randomized_list_scans_match_the_plain_scanner() {
    randomized_lists(0x8e6a_11c9_a97f_8b3d, 400);
}

#[test]
#[ignore = "larger syntax-index differential run"]
fn randomized_list_scans_extended() {
    randomized_lists(0x01e7_1064_f5a8_ab2b, 4000);
}

fn randomized_lists(seed: u64, cases: usize) {
    let mut rng = Rng(seed);
    let mut served = 0;
    let mut errors = 0;
    let mut calls = 0;
    for case in 0..cases {
        let options = scan::Options {
            ignore_comments: case.is_multiple_of(2),
            comment_end_can_be_escaped: case % 4 < 2,
            multibyte_syntax_as_symbol: case % 8 < 4,
            propertize_done: Some(i64::MAX),
            ..scan::Options::default()
        };
        let mut context = Context { table: 1, writes: 0, escaped_ends: options.comment_end_can_be_escaped, properties: true };
        let mut index = SyntaxIndex { leaf: 1 + rng.below(16), ..SyntaxIndex::default() };
        let mut table = random_table(&mut rng);
        let len = 64 + rng.below(192);
        let mut bytes = random_text(&mut rng, len);
        let mut properties = vec![None; len];
        for round in 0..10 {
            if round > 0 {
                edit_list_text(&mut rng, &mut index, &mut bytes, &mut properties, &table, options.multibyte_syntax_as_symbol);
                if rng.chance(15) {
                    table = random_table(&mut rng);
                    context.writes += 1;
                }
            }
            let plain = Plain { text: bytes.clone(), table: |c| table.get(usize::from(c)).copied().unwrap_or(scan::SCLOSE) };
            let text = Propertied { plain: &plain, properties: &properties };
            let (begv, end) = list_bounds(&mut rng, bytes.len());
            let ppss = BTreeMap::new();
            let mut scanner = Scanner { text: &text, begv: text.pos(begv), zv: text.pos(end), options, ppss: &ppss, defun_start: None, modiff: 0 };
            let accessible = usize::try_from(end - begv).unwrap();
            let random_from = begv + i64::try_from(rng.below(accessible + 1)).unwrap();
            for from in [0, begv, random_from] {
                let count = end - from.clamp(begv, end) + 1 + i64::try_from(rng.below(3)).unwrap();
                let expected = scanner.scan_lists(from, count, 0, true).map_err(|stopped| stopped.halt);
                if let Some(actual) = lists(&mut index, &mut scanner, bytes.len(), context.clone(), (from, count)) {
                    assert_eq!(actual, expected,
                               "seed {seed:#x}, case {case}, round {round}, leaf {}, bounds {begv}..{end}, from {from}, count {count}",
                               index.leaf);
                    errors += usize::from(actual.is_err());
                }
                calls += 1;
            }
        }
        served += index.stats.list_served;
    }
    println!("list index served {served}/{calls} scans ({errors} scan errors)");
    assert!(served * 2 > calls && errors > 0, "served {served}/{calls}, errors {errors}");
}

fn list_bounds(rng: &mut Rng, len: usize) -> (i64, i64) {
    if rng.chance(35) {
        let start = rng.below(len + 1);
        (position(start), position(start + rng.below(len - start + 1)))
    } else {
        (1, position(len))
    }
}

fn edit_list_text(rng: &mut Rng, index: &mut SyntaxIndex, bytes: &mut Vec<u8>, properties: &mut Vec<Option<i32>>,
                  table: &[i32; 128], multibyte_symbol: bool) {
    if rng.chance(80) {
        let start = rng.below(bytes.len() + 1);
        let end = (start + rng.below(6)).min(bytes.len());
        let len = rng.below(6);
        let mut inserted = random_text(rng, len);
        if multibyte_symbol && rng.chance(35) {
            inserted.push(0xe9);
        }
        index.edit(start, end, inserted.len());
        properties.splice(start..end, std::iter::repeat_n(None, inserted.len()));
        bytes.splice(start..end, inserted);
    }
    if rng.chance(40) && !bytes.is_empty() {
        let start = rng.below(bytes.len());
        let end = (start + 1 + rng.below(8)).min(bytes.len());
        let syntax = rng.chance(70).then(|| table[usize::from(ALPHABET[rng.below(ALPHABET.len())])]);
        properties[start..end].fill(syntax);
        index.touch(start, end);
    }
}
