mod support;
use romacs_regex::*;
#[test]
fn folded_repeated_near_matches_have_linear_work() {
    for (pair, tail) in [("ba", "ab"), ("Σa", "aσ")] {
        let pattern = pair.repeat(1023) + tail;
        let text = pair.repeat(32768).into_bytes();
        let re = Regex::literal(pattern.as_bytes(), true, true, Some(&support::Fold));
        for split in [0, text.len() / 2 + 1] {
            for back in [false, true] {
                let h = Haystack {
                    s1: &text[..split],
                    s2: &text[split..],
                    multibyte: true,
                    point: None,
                };
                let mut st = Stats::default();
                let mut regs = Regs::default();
                assert_eq!(
                    re.search(
                        &h,
                        if back { text.len() } else { 0 },
                        if back {
                            -(text.len() as isize)
                        } else {
                            text.len() as isize
                        },
                        text.len(),
                        &(),
                        &mut regs,
                        &mut st,
                    )
                    .unwrap(),
                    None,
                );
                assert!(
                    st.verify_bytes <= 12 * (text.len() + pattern.len()) as u64,
                    "pair={pair:?} split={split} back={back}: {st:?}",
                );
                assert_eq!(st.steps, 0);
            }
        }
    }
}

#[test]
fn folded_fallback_preserves_hits_bounds_raw_and_custom_cycles() {
    struct Custom(bool);
    impl CaseTable for Custom {
        fn canon(&self, c: u32) -> u32 {
            if self.0 {
                match c {
                    97 | 0xe9 => 97,
                    c if c == romacs_chars::unibyte_to_char(0xff) => 97,
                    _ => c,
                }
            } else {
                match c {
                    97 | 113 => 113,
                    98 | 122 => 122,
                    _ => c,
                }
            }
        }
        fn eqv_next(&self, c: u32) -> u32 {
            if self.0 {
                match c {
                    97 => 0xe9,
                    0xe9 => romacs_chars::unibyte_to_char(0xff),
                    c if c == romacs_chars::unibyte_to_char(0xff) => 97,
                    _ => c,
                }
            } else {
                match c {
                    97 => 113,
                    113 => 97,
                    98 => 122,
                    122 => 98,
                    _ => c,
                }
            }
        }
    }
    let pat = ("ba".repeat(63) + "ab").into_bytes();
    let raw = romacs_chars::str_to_multibyte(&[0xff]);
    let raw_pair = [b"b".as_slice(), &raw].concat();
    let cases: Vec<(Vec<u8>, Vec<u8>, &dyn CaseTable)> = vec![
        (
            b"ba".repeat(4096),
            ("BA".repeat(63) + "AB").into_bytes(),
            &support::Fold,
        ),
        (
            b"zq".repeat(4096),
            ("zq".repeat(63) + "qz").into_bytes(),
            &Custom(false),
        ),
        (
            raw_pair.repeat(4096),
            [raw_pair.repeat(63), "éb".as_bytes().to_vec()].concat(),
            &Custom(true),
        ),
    ];
    for (near, hit, table) in cases {
        let re = Regex::literal(&pat, true, true, Some(table));
        let want = near.len() + 1;
        let end = want + hit.len();
        let text = [&near[..], b"|", &hit, b"|", &hit, b"|", &near].concat();
        for split in [0, near.len() / 2 + 1, want + hit.len() / 2 + 1] {
            for back in [false, true] {
                let h = Haystack {
                    s1: &text[..split],
                    s2: &text[split..],
                    multibyte: true,
                    point: None,
                };
                let mut st = Stats::default();
                let mut regs = Regs::default();
                assert_eq!(
                    re.search(
                        &h,
                        if back { text.len() } else { 0 },
                        if back {
                            -(text.len() as isize)
                        } else {
                            text.len() as isize
                        },
                        text.len(),
                        &(),
                        &mut regs,
                        &mut st
                    )
                    .unwrap(),
                    Some(if back { end + 1 } else { want })
                );
                assert_eq!(
                    regs.end,
                    vec![(if back { end + 1 + hit.len() } else { end }) as isize]
                );
                assert!(
                    st.verify_bytes <= 12 * (text.len() + pat.len()) as u64,
                    "{st:?}"
                );
                assert_eq!(
                    support::find(&re, &text, split, true, want, 0, text.len()),
                    Some((want, end))
                );
                assert_eq!(
                    support::find(&re, &text, split, true, 0, (want - 1) as isize, text.len()),
                    None
                );
                // Stop on a character boundary before the first hit's final byte.
                assert_eq!(
                    support::find(
                        &re,
                        &text,
                        split,
                        true,
                        if back { text.len() } else { 0 },
                        if back {
                            -(text.len() as isize)
                        } else {
                            text.len() as isize
                        },
                        end - 1
                    ),
                    None
                );
            }
        }
    }
}

#[test]
fn production_size_cached_work_bounds() {
    let mut text = vec![b' '; 1_300_306];
    let mut seed = 17u32;
    for b in &mut text {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *b = b" abcdefhijklmnopqrstuvxyz_
"[(seed >> 16) as usize % 26];
    }
    for i in 0..502 {
        let at = i * 2500 + 31;
        text[at..at + 9].copy_from_slice(b"glyph_row");
    }
    // Frequent isolated w/W anchors, but only ~one packed-pair candidate per hit.
    for at in (100..text.len() - 9).step_by(61) {
        if (at % 2500).abs_diff(31) > 11 {
            text[at] = b'w';
            text[at - 2] = b' ';
        }
    }
    let mut cache = Cache::new();
    let mut st = Stats::default();
    let mut count = 0;
    for _ in 0..20 {
        let mut p = 0;
        loop {
            let re = cache
                .get(
                    KeyRef {
                        kind: Kind::Literal,
                        pattern: b"glyph_row",
                        pattern_multibyte: false,
                        target_multibyte: false,
                        posix: false,
                        whitespace: None,
                        case: Some((1, 1)),
                    },
                    || {
                        Ok(Regex::literal(
                            b"glyph_row",
                            false,
                            false,
                            Some(&support::Fold),
                        ))
                    },
                    &mut st,
                )
                .unwrap();
            let h = Haystack {
                s1: &text[..650_153],
                s2: &text[650_153..],
                multibyte: false,
                point: None,
            };
            let mut regs = Regs::default();
            let before = st.window_bytes;
            let found = re
                .search(
                    &h,
                    p,
                    (h.len() - p) as isize,
                    h.len(),
                    &(),
                    &mut regs,
                    &mut st,
                )
                .unwrap();
            assert!(st.window_bytes - before <= 16);
            if found.is_none() {
                break;
            }
            p = regs.end[0] as usize;
            count += 1;
        }
    }
    assert_eq!(count, 10_040);
    assert_eq!(st.compiles, 1);
    assert!(st.candidates <= 3 * count);
    assert!(st.verify_bytes <= 9 * st.candidates);
    assert_eq!(st.steps, 0);
    assert_eq!(st.memo_engaged, 0);
    for (pat, fold, back) in [
        (b"glyph_row", false, false),
        (b"GLYPH_ROW", true, false),
        (b"glyph_row", true, true),
    ] {
        let re = Regex::literal(
            pat,
            false,
            false,
            fold.then_some(&support::Fold as &dyn CaseTable),
        );
        let h = Haystack {
            s1: &text[..650_153],
            s2: &text[650_153..],
            multibyte: false,
            point: None,
        };
        let mut p = if back { h.len() } else { 0 };
        let mut hits = 0;
        let mut regs = Regs::default();
        let mut work = Stats::default();
        loop {
            let before = work.window_bytes;
            let found = re
                .search(
                    &h,
                    p,
                    if back {
                        -(p as isize)
                    } else {
                        (h.len() - p) as isize
                    },
                    if back { p } else { h.len() },
                    &(),
                    &mut regs,
                    &mut work,
                )
                .unwrap();
            assert!(work.window_bytes - before <= 16);
            if found.is_none() {
                break;
            }
            p = if back {
                regs.start[0] as usize
            } else {
                regs.end[0] as usize
            };
            hits += 1;
        }
        assert_eq!(hits, 502);
        assert!(work.candidates <= 3 * hits);
        assert!(work.verify_bytes <= 9 * work.candidates);
    }
}
