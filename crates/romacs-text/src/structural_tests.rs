use super::test_support::iterations;
use super::*;

#[test]
fn short_line_queries_do_not_descend_the_chunk_tree() {
    let mut bytes = vec![b'x'; 4096];
    for i in (31..bytes.len()).step_by(32) {
        bytes[i] = b'\n';
    }
    let mut text = Text::from_bytes(&bytes, true);
    text.move_gap_to(BytePos(1010));
    for (from, limit, count, expected) in [(1001, 4096, 1, 1024), (1017, 0, -1, 991)] {
        TREE_LOOKUPS.with(|n| n.set(0));
        assert_eq!(
            text.find_newline(BytePos(from), BytePos(limit), count),
            (BytePos(expected), 1)
        );
        assert_eq!(TREE_LOOKUPS.with(|n| n.get()), 0);
    }
}

#[test]
fn gap_motion_preserves_the_logical_insert_cache() {
    let (mut text, mut model) = cached_text();
    text.move_gap_to(BytePos(0));
    verify_cached_chunk(&text);
    TREE_LOOKUPS.with(|n| n.set(0));
    text.insert(BytePos(20), b"z", false);
    assert_eq!(TREE_LOOKUPS.with(|n| n.get()), 0);
    model.insert(20, b'z');
    verify(&text, &model);
    verify_cached_chunk(&text);
}

#[test]
fn rightmost_chunk_updates_include_inserts_before_a_suffix() {
    for suffix in ["", "\n> "] {
        let mut model = "αx\n".repeat(32).into_bytes();
        model.extend_from_slice(suffix.as_bytes());
        let mut text = Text::from_bytes(&model, true);
        for _ in 0..40 {
            let at = model.len() - suffix.len();
            text.insert(BytePos(at), "β\n".as_bytes(), false);
            model.splice(at..at, "β\n".bytes());
            verify_cached_chunk(&text);
            verify(&text, &model);
        }
        // Clone and mutate away from the right edge to exercise the general
        // rank walk after the specialized updates and structural splits.
        let snapshot = text.clone();
        text.insert(BytePos(2), b"z", false);
        verify(&snapshot, &model);
        model.insert(2, b'z');
        verify(&text, &model);
    }
}

#[test]
fn tiny_tail_inserts_do_no_ancestor_work_between_splits() {
    for suffix in ["", "λ\n> "] {
        let mut model = "αx\n".repeat(64).into_bytes();
        model.extend_from_slice(suffix.as_bytes());
        let mut text = Text::from_bytes(&model, true);
        let mut splits = 0;
        for step in 0..512 {
            let at = model.len() - suffix.len();
            let payload = if step % 3 == 0 { "β\n" } else { "x" };
            let old_len = text.chunks.len();
            let tail = text.chunks[old_len - 1];
            let fits = tail.bytes as usize + payload.len() <= 2 * T;
            text.index_work = 0;
            text.chunks.work = 0;
            text.insert(BytePos(at), payload.as_bytes(), false);
            if fits {
                assert_eq!(text.index_work, 0, "tail summary update visited the AVL");
                assert_eq!(text.chunks.len(), old_len);
            } else {
                splits += 1;
                assert_eq!(text.chunks.len(), old_len + 1);
                assert!(text.index_work <= 8 * (old_len.ilog2() as usize + 1));
            }
            model.splice(at..at, payload.bytes());
            verify_cached_chunk(&text);
            verify(&text, &model);
            assert!(text.chunks.len() <= text.len_bytes() / (T / 4) + 1);
        }
        assert!(splits > 16);
        let snapshot = text.clone();
        text.delete(BytePos(model.len() - suffix.len()), BytePos(model.len()));
        verify(&snapshot, &model);
        model.truncate(model.len() - suffix.len());
        verify(&text, &model);
    }
}

#[test]
fn advancing_cached_splits_scan_only_the_suffix() {
    // Exercise the tail and a middle chunk; a multibyte prompt also checks
    // subtraction of character and newline summaries at the split boundary.
    for suffix in ["", "\n> ", "λ\n>"] {
        let mut model = "αx\n".repeat(8).into_bytes();
        model.extend_from_slice(suffix.as_bytes());
        let mut text = Text::from_bytes(&model, true);
        text.move_gap_to(BytePos(model.len() - suffix.len()));
        let mut splits = 0;
        for step in 0..128 {
            let payload = if step % 3 == 0 { "β\n" } else { "x" };
            let at = model.len() - suffix.len();
            let before = text.chunks.len();
            SCANNED_BYTES.with(|n| n.set(0));
            TREE_LOOKUPS.with(|n| n.set(0));
            text.insert_measured(
                text.gap_start(),
                payload.as_bytes(),
                payload.chars().count(),
                false,
            );
            assert!(SCANNED_BYTES.with(|n| n.get()) <= payload.len() + suffix.len());
            assert_eq!(TREE_LOOKUPS.with(|n| n.get()), 0);
            if text.chunks.len() != before {
                splits += 1;
                assert_eq!(text.chunks.len(), before + 1);
                let (_, _, right) = text.cached.unwrap();
                assert_eq!(right.bytes as usize, payload.len() + suffix.len());
            }
            model.splice(at..at, payload.bytes());
            verify_cached_chunk(&text);
            verify(&text, &model);
        }
        assert!(splits >= 8, "must exercise repeated overflows");
    }

    let mut model = vec![b'x'; 16 * T];
    let mut text = Text::from_bytes(&model, true);
    let mut at = 8 * T - 3;
    text.insert(BytePos(at), b"\n", false);
    model.insert(at, b'\n');
    at += 1;
    let mut splits = 0;
    for step in 0..1024 {
        let before = text.chunks.len();
        SCANNED_BYTES.with(|n| n.set(0));
        text.insert_measured(text.gap_start(), b"\n", 1, false);
        assert!(SCANNED_BYTES.with(|n| n.get()) <= 4);
        if text.chunks.len() != before {
            splits += 1;
            assert_eq!(text.chunks.len(), before + 1);
            assert_eq!(text.cached.unwrap().2.bytes, 4);
        }
        model.insert(at, b'\n');
        at += 1;
        verify_cached_chunk(&text);
        if step % 64 == 0 {
            verify(&text, &model);
        }
    }
    assert!(splits >= 64);
    assert!(text.chunks.len() <= text.len_bytes() / (T / 4) + 1);
    verify(&text, &model);
}

#[test]
fn cached_split_guards_preserve_generic_fallback() {
    for (at, bytes) in [(0, 1), (2 * T, 1), (T / 4, 2 * T)] {
        let mut model = vec![b'x'; 2 * T];
        let mut text = Text::from_bytes(&model, true);
        text.insert(BytePos(model.len()), b"x", false);
        model.push(b'x');
        // The first chunk is now a full interior chunk. Reject a short left,
        // a short interior right, and an oversized right without editing it.
        let old = text.chunks[0];
        let prefix = Sum::default();
        text.cached = Some((0, prefix, old));
        let ins = Sum {
            bytes,
            chars: bytes,
            nl: 0,
        };
        assert!(
            text.split_cached_insert(BytePos(at), ins, 0, prefix, old)
                .is_none()
        );
        verify(&text, &model);
        let payload = vec![b'x'; bytes];
        text.insert(BytePos(at), &payload, false);
        model.splice(at..at, payload);
        verify(&text, &model);
        verify_cached_chunk(&text);
    }
}

#[test]
fn short_chunks_grow_without_structural_fallback() {
    let mut text = Text::new(true);
    for _ in 0..2 {
        TOPOLOGY_REFRESHES.with(|n| n.set(0));
        TREE_LOOKUPS.with(|n| n.set(0));
        text.insert(BytePos(text.len_bytes()), b"x", false);
        assert_eq!(TOPOLOGY_REFRESHES.with(|n| n.get()), 0);
        assert_eq!(TREE_LOOKUPS.with(|n| n.get()), 0);
        verify_cached_chunk(&text);
        text.check_invariants();
    }
}

#[test]
fn measured_bulk_load_scans_each_byte_once_and_builds_linearly() {
    let bytes = vec![b'\n'; 2500 * T];
    let mut text = Text::new(true);
    SCANNED_BYTES.with(|n| n.set(0));
    text.index_work = 0;
    text.insert_measured(
        TextPos {
            byte: BytePos(0),
            char: CharPos(0),
        },
        &bytes,
        bytes.len(),
        false,
    );
    assert_eq!(SCANNED_BYTES.with(|n| n.get()), bytes.len());
    assert!(text.index_work <= text.chunks.len() + 1);
    text.check_invariants();
}

#[test]
fn point_insert_updates_summaries_without_topology_refresh() {
    let mut text = Text::from_bytes(&vec![b'x'; 128 * T], true);
    // Distant inserts and subsequent gap-local inserts must both avoid rebuilding
    // heights/lengths or rereading sibling summaries when no chunk splits.
    for at in [3 * T + 2, 80 * T + 2, 20 * T + 2] {
        for at in [at, at + 1] {
            TOPOLOGY_REFRESHES.with(|n| n.set(0));
            TREE_LOOKUPS.with(|n| n.set(0));
            text.index_work = 0;
            text.chunks.work = 0;
            text.insert(BytePos(at), b"\n", false);
            let refreshes = TOPOLOGY_REFRESHES.with(|n| n.get());
            println!(
                "point insert at={at}: topology={refreshes} summary_nodes={}",
                text.index_work
            );
            assert_eq!(refreshes, 0, "point insertion cannot change AVL topology");
            assert!(text.index_work <= 8, "more than one update path");
            assert_eq!(
                TREE_LOOKUPS.with(|n| n.get()),
                0,
                "separate tree lookup before update"
            );
            text.check_invariants();
        }
    }
}

#[test]
fn gap_accessors_follow_edits_without_changing_content() {
    let mut text = Text::new(true);
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(0),
            char: CharPos(0)
        }
    );
    assert_eq!(text.gap_len(), 0);
    text.insert(BytePos(0), "αβ".as_bytes(), false);
    let initial_size = text.gap_len();
    assert!(initial_size > 0);
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(4),
            char: CharPos(2)
        }
    );
    text.insert(BytePos(2), b"x", false);
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(3),
            char: CharPos(2)
        }
    );
    assert_eq!(text.gap_len(), initial_size - 1);
    text.delete(BytePos(0), BytePos(2));
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(0),
            char: CharPos(0)
        }
    );
    assert_eq!(text.gap_len(), initial_size + 1);
    let snapshot = text.clone();
    text.replace(BytePos(0), BytePos(1), "λ".as_bytes());
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(2),
            char: CharPos(1)
        }
    );
    assert_eq!(
        snapshot.gap_start(),
        TextPos {
            byte: BytePos(0),
            char: CharPos(0)
        }
    );
    verify(&text, "λβ".as_bytes());
    verify(&snapshot, "xβ".as_bytes());
}

#[test]
fn content_epoch_survives_tick_restores() {
    let mut text = Text::new(true);
    let mut previous = text.epoch();
    text.insert(BytePos(0), "αβ".as_bytes(), false);
    assert!(text.epoch() > previous);
    previous = text.epoch();
    text.set_modification_ticks(0, 0);
    text.bump_modiff(10);
    assert_eq!(text.epoch(), previous);
    text.replace(BytePos(0), BytePos(2), b"x");
    assert!(text.epoch() > previous);
    previous = text.epoch();
    text.delete(BytePos(0), BytePos(1));
    assert!(text.epoch() > previous);
    previous = text.epoch();
    text.insert(BytePos(0), b"", false);
    text.delete(BytePos(0), BytePos(0));
    assert_eq!(text.epoch(), previous);
    let snapshot = text.clone();
    text.insert(BytePos(0), b"y", false);
    assert_eq!(snapshot.epoch(), previous);
    assert!(text.epoch() > snapshot.epoch());
}

#[test]
fn conversions_near_gap_scan_only_the_distance() {
    let text = Text::from_bytes("α".repeat(T / 2).as_bytes(), true);
    SCANNED_BYTES.with(|s| s.set(0));
    assert_eq!(text.char_of_byte(BytePos(T - 2)), CharPos(T / 2 - 1));
    assert!(SCANNED_BYTES.with(|s| s.get()) <= 2);
    SCANNED_BYTES.with(|s| s.set(0));
    assert_eq!(text.byte_of_char(CharPos(T / 2 - 1)), BytePos(T - 2));
    assert!(SCANNED_BYTES.with(|s| s.get()) <= 2);
}

#[test]
fn multibyte_typing_scans_only_inserted_text() {
    let mut text = Text::from_bytes("α".repeat(T / 2).as_bytes(), true);
    SCANNED_BYTES.with(|s| s.set(0));
    let at = text.len_bytes();
    let edit = text.insert(BytePos(at), "β".as_bytes(), false);
    assert_eq!(edit.at.char, CharPos(T / 2));
    assert!(SCANNED_BYTES.with(|s| s.get()) <= 2);
    assert_eq!(text.char_of_byte(BytePos(at + 2)), CharPos(T / 2 + 1));
    assert_eq!(text.byte_of_char(CharPos(T / 2 + 1)), BytePos(at + 2));
}

#[test]
fn split_growth_index_work_is_subquadratic() {
    // The reviewer's 8193-byte stream, scaled to the unit-test chunk size.
    // Every edit splits; keep the insertion point advancing inside old text.
    fn work(n: usize) -> usize {
        let mut t = Text::from_bytes(&vec![b'x'; 2500 * T], true);
        let batch = vec![b'\n'; 2 * T + 1];
        let mut p = t.len_bytes() / 2;
        t.index_work = 0;
        for _ in 0..n {
            t.insert(BytePos(p), &batch, false);
            p += batch.len();
        }
        let work = t.index_work;
        assert_eq!(t.len_newlines(), n * batch.len());
        t.check_invariants();
        work
    }
    let a = work(8192);
    let b = work(16384);
    println!("split-growth index nodes: N={a} 2N={b}");
    assert!(b * 10 < a * 26, "quadratic index maintenance: {a} -> {b}");
}

fn verify(t: &Text, model: &[u8]) {
    let mut got = vec![];
    t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut got);
    assert_eq!(got, model);
    let (mut chars, mut lines) = (0, 0);
    for (i, &b) in model.iter().enumerate() {
        assert_eq!(t.newlines_before(BytePos(i)), lines);
        if char_head_p(b) {
            assert_eq!(t.byte_of_char(CharPos(chars)), BytePos(i));
            assert_eq!(t.char_of_byte(BytePos(i)), CharPos(chars));
            chars += 1;
        }
        lines += usize::from(b == b'\n');
    }
    assert_eq!(t.newlines_before(BytePos(model.len())), lines);
    assert_eq!(t.byte_of_char(CharPos(chars)), BytePos(model.len()));
    t.check_invariants();
}

#[test]
fn split_merge_preserves_chunk_count() {
    let mut m = vec![b'x'; 2 * T + 1];
    let mut t = Text::from_bytes(&m, true);
    verify(&t, &m);
    t.insert(BytePos(0), &[b'x'; 2 * T + 1], false);
    m.splice(0..0, vec![b'x'; 2 * T + 1]);
    verify(&t, &m);
    t.insert(BytePos(3 * T + 1), &[b'\n'; T], false);
    m.splice(3 * T + 1..3 * T + 1, vec![b'\n'; T]);
    assert_eq!(t.newlines_before(BytePos(4 * T + 2)), T);
    verify(&t, &m);
}

#[test]
fn adversarial_large_and_ends() {
    let mut atom = vec![];
    for c in [97, 10, 0xe9, 0x20ac, 0x1f600, 0x234567, 0x3fff80] {
        let mut b = [0; 5];
        let n = romacs_chars::char_string(c, &mut b);
        atom.extend_from_slice(&b[..n]);
    }
    let huge = atom.repeat(iterations(256, 50000));
    let mut t = Text::from_bytes(&huge, true);
    verify(&t, &huge);
    t.delete(BytePos(0), BytePos(t.len_bytes()));
    verify(&t, &[]);
    let mut model = vec![];
    for i in 0..iterations(128, 1000) {
        let f = if i % 2 == 0 { 0 } else { model.len() };
        t.insert(BytePos(f), &atom, false);
        model.splice(f..f, atom.iter().copied());
        if i % 20 == 0 {
            verify(&t, &model);
        }
    }
    verify(&t, &model);
    while !model.is_empty() {
        let n = atom.len().min(model.len());
        let f = if model.len() % 2 == 0 {
            0
        } else {
            model.len() - n
        };
        t.delete(BytePos(f), BytePos(f + n));
        model.drain(f..f + n);
        verify(&t, &model);
    }
}

#[test]
fn newline_search_scans_only_the_requested_lines() {
    let text = Text::from_bytes(&b"x\n".repeat(20_000), true);
    for (from, limit, count, expected) in [
        (T - 2, text.len_bytes(), 1, T),
        (T, 0, -1, T - 1),
        (0, text.len_bytes(), 10_000, 20_000),
        (text.len_bytes(), 0, -10_000, 20_001),
    ] {
        SCANNED_BYTES.with(|s| s.set(0));
        assert_eq!(
            text.find_newline(BytePos(from), BytePos(limit), count),
            (BytePos(expected), count.unsigned_abs())
        );
        let budget = if count.unsigned_abs() == 1 {
            2 + 8
        } else {
            2 * T
        };
        assert!(SCANNED_BYTES.with(|s| s.get()) <= budget);
    }
}

proptest::proptest! {
    #[test]
    fn newline_search_matches_scan(
        bytes in proptest::collection::vec(proptest::sample::select(vec![b'x', b'\n', 0xce, 0xbb]), 0..200),
        a in 0usize..300, b in 0usize..300, count in -30isize..30,
    ) {
        let mut text = Text::from_bytes(&bytes, false);
        let gap = a % (bytes.len() + 1);
        text.insert(BytePos(gap), b"z", false);
        text.delete(BytePos(gap), BytePos(gap + 1));
        let f = a % (bytes.len() + 1);
        let z = b % (bytes.len() + 1);
        let (from, limit) = if count >= 0 { (f.min(z), f.max(z)) } else { (f.max(z), f.min(z)) };
        let mut found = 0;
        let mut pos = from;
        while found < count.unsigned_abs() && pos != limit {
            let at = if count > 0 { let at = pos; pos += 1; at } else { pos -= 1; pos };
            found += usize::from(bytes[at] == b'\n');
        }
        proptest::prop_assert_eq!(text.find_newline(BytePos(from), BytePos(limit), count), (BytePos(pos), found));
    }
}

#[test]
fn newline_directional_contract() {
    let t = Text::from_bytes(b"a\nb\n", true);
    for (from, limit, count, pos, found) in [
        (1, 4, 1, 2, 1),
        (2, 4, 1, 4, 1),
        (1, 1, 1, 1, 0),
        (0, 1, 1, 1, 0),
        (0, 4, 3, 4, 2),
        (2, 0, -1, 1, 1),
        (1, 0, -1, 0, 0),
        (2, 1, -1, 1, 1),
        (4, 2, -1, 3, 1),
        (4, 0, -3, 0, 2),
        (2, 2, -1, 2, 0),
        (2, 2, 0, 2, 0),
    ] {
        assert_eq!(
            t.find_newline(BytePos(from), BytePos(limit), count),
            (BytePos(pos), found)
        );
    }
}

#[test]
fn batch_fuzz_100k() {
    let mut seed = 91u64;
    for _case in 0..iterations(10, 100) {
        let mut t = Text::new(true);
        let mut m = vec![];
        for _step in 0..1000 {
            let mut next = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 24) as usize
            };
            let a = next() % (m.len() + 1);
            let b = next() % (m.len() + 1);
            let (f, z) = (a.min(b), a.max(b));
            let n = next() % 80;
            let s: Vec<_> = (0..n)
                .map(|_| if next() % 5 == 0 { 10 } else { b'a' })
                .collect();
            match next() % 3 {
                0 => {
                    t.insert(BytePos(a), &s, false);
                    m.splice(a..a, s);
                }
                1 => {
                    t.delete(BytePos(f), BytePos(z));
                    m.drain(f..z);
                }
                _ => {
                    t.replace(BytePos(f), BytePos(z), &s);
                    m.splice(f..z, s);
                }
            }
            verify(&t, &m);
        }
    }
}

#[test]
fn cached_chunk_is_invalidated_by_structural_edits() {
    for replace in [false, true] {
        let mut model = vec![b'x'; 3 * T];
        let mut text = Text::from_bytes(&model, true);
        if replace {
            text.replace(BytePos(2 * T), BytePos(3 * T), b"\n");
            model.splice(2 * T..3 * T, [b'\n']);
        } else {
            text.delete(BytePos(2 * T), BytePos(3 * T));
            model.truncate(2 * T);
        }
        let at = text.gap.start;
        text.insert(BytePos(at), b"a\n", false);
        model.splice(at..at, *b"a\n");
        verify(&text, &model);
    }
}

fn cached_text() -> (Text, Vec<u8>) {
    let mut model = vec![b'x'; 8 * T];
    let mut text = Text::from_bytes(&model, true);
    text.insert(BytePos(2 * T + 4), b"a", false);
    model.insert(2 * T + 4, b'a');
    verify(&text, &model);
    (text, model)
}

fn verify_cached_chunk(text: &Text) {
    let (rank, prefix, chunk) = text.cached.expect("insertion publishes a cache");
    assert!(rank < text.chunks.len());
    assert_eq!(prefix, text.chunks.prefix(rank));
    assert_eq!(chunk.wide(), text.chunks[rank].wide());
}

#[test]
fn consecutive_cached_insertions_refresh_summary() {
    for payload in [b"z\n".as_slice(), "é\n".as_bytes()] {
        let (mut text, mut model) = cached_text();
        let at = 2 * T + 5;
        let stay = text.marker_new(BytePos(at), false);
        let advance = text.marker_new(BytePos(at), true);
        for bytes in [b"q".as_slice(), payload] {
            let at = text.gap.start;
            let char_at = model[..at].iter().filter(|&&b| char_head_p(b)).count();
            let chars = bytes.iter().filter(|&&b| char_head_p(b)).count();
            assert_eq!(
                text.insert(BytePos(at), bytes, false),
                Edit {
                    at: TextPos {
                        byte: BytePos(at),
                        char: CharPos(char_at)
                    },
                    del: (0, 0),
                    ins: (bytes.len(), chars),
                }
            );
            model.splice(at..at, bytes.iter().copied());
            // Exercise content invariants and public queries before cache metadata.
            text.check_invariants();
            verify(&text, &model);
        }
        assert_eq!(
            text.marker_pos(stay),
            Some(TextPos {
                byte: BytePos(at),
                char: CharPos(at),
            })
        );
        assert_eq!(
            text.marker_pos(advance),
            Some(TextPos {
                byte: BytePos(at + 1 + payload.len()),
                char: CharPos(at + 3),
            })
        );
        verify_cached_chunk(&text);
    }
}

#[test]
fn generic_edits_never_retain_stale_cached_chunks() {
    // Same-chunk delete/replace, cross-chunk removal, merge, split, erase-all.
    for (from, to, replacement) in [
        (20, 21, vec![]),
        (20, 21, vec![b'\n']),
        (16, 48, vec![]),
        (17, 24, vec![]),
        (20, 40, vec![b'\n'; 8 * T]),
        (0, 8 * T + 1, vec![]),
    ] {
        let (mut text, mut model) = cached_text();
        if replacement.is_empty() {
            text.delete(BytePos(from), BytePos(to));
        } else {
            text.replace(BytePos(from), BytePos(to), &replacement);
        }
        model.splice(from..to, replacement);
        if (from == 20 && to == 21) || text.cached.is_some() {
            verify_cached_chunk(&text);
        }
        let at = text.gap.start;
        text.insert(BytePos(at), b"z\n", false);
        model.splice(at..at, *b"z\n");
        verify(&text, &model);
        verify_cached_chunk(&text);
    }
}

#[test]
fn generic_split_publishes_final_cached_chunk() {
    let (mut text, mut model) = cached_text();
    let bytes = vec![b'\n'; 16 * T];
    text.insert(BytePos(20), &bytes, false);
    model.splice(20..20, bytes);
    verify(&text, &model);
    verify_cached_chunk(&text);
    let at = text.gap.start;
    text.insert(BytePos(at), b"z\n", false);
    model.splice(at..at, *b"z\n");
    verify(&text, &model);
}

#[test]
fn shared_queries_preserve_cached_chunk() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Text>();
    let (text, model) = cached_text();
    let cached = text.cached.map(|(i, p, c)| (i, p, c.wide()));
    let (a, b) = (&text, &text);
    assert_eq!(a.char_of_byte(BytePos(20)), b.char_of_byte(BytePos(20)));
    verify(&text, &model);
    assert_eq!(text.cached.map(|(i, p, c)| (i, p, c.wide())), cached);
}
