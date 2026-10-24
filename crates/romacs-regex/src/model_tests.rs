use crate::*;

#[test]
fn literal_non_idempotent_fold_does_not_assume_canonical_targets_are_identity() {
    struct Fold;
    impl CaseTable for Fold {
        fn canon(&self, c: u32) -> u32 {
            match c {
                0x416 => 0x436,
                65 => 97,
                97 => 122,
                _ => c,
            }
        }
        fn eqv_next(&self, c: u32) -> u32 {
            match c {
                0x416 => 0x436,
                0x436 => 0x416,
                97 => 122,
                122 => 97,
                _ => c,
            }
        }
    }
    let pattern = encoded(&[0x416, 65], true);
    let text = encoded(&[0x436, 97], true);
    let regex = Regex::literal(&pattern, true, true, Some(&Fold));
    for split in 0..=text.len() {
        for back in [false, true] {
            let hay = Haystack {
                s1: &text[..split],
                s2: &text[split..],
                multibyte: true,
                point: None,
            };
            let got = regex
                .search(
                    &hay,
                    if back { text.len() } else { 0 },
                    if back {
                        -(text.len() as isize)
                    } else {
                        text.len() as isize
                    },
                    text.len(),
                    &(),
                    &mut Regs::default(),
                    &mut Stats::default(),
                )
                .unwrap();
            assert_eq!(got, None, "split={split}, backward={back}");
        }
    }
}

#[test]
fn translated_literal_is_lazy_split_safe_and_call_local() {
    let raw = romacs_chars::byte8_to_char(255);
    let text = encoded(&[0xc9, raw, 65, 0xe9, raw, 97], true);
    let pat = encoded(&[0xe9, raw, 97], true);
    for split in 0..=text.len() {
        for back in [false, true] {
            let h = Haystack {
                s1: &text[..split],
                s2: &text[split..],
                multibyte: true,
                point: None,
            };
            let mut calls = Vec::new();
            let mut st = Stats::default();
            let got = translated_literal(
                &pat,
                true,
                &h,
                back,
                |c| {
                    calls.push(c);
                    match c {
                        0xc9 => 0xe9,
                        65 => 97,
                        _ => c,
                    }
                },
                &mut st,
            );
            let middle = text.len() / 2;
            assert_eq!(
                got,
                Some(if back {
                    (middle, text.len())
                } else {
                    (0, middle)
                })
            );
            assert_eq!(calls.len(), if back { 9 } else { 6 });
            assert!(calls.iter().all(|c| [0xc9, 0xe9, raw, 65, 97].contains(c)));
            assert_eq!(st.window_bytes, 0);
            // No compiled table or previous call may retain the old canonicalization.
            assert_eq!(
                translated_literal(&pat, true, &h, false, |c| c, &mut st),
                Some((middle, text.len()))
            );
        }
    }
}

#[test]
fn translated_literal_empty_and_representation_conversion() {
    for pm in [false, true] {
        for tm in [false, true] {
            let source = if pm {
                romacs_chars::byte8_to_char(255)
            } else {
                255
            };
            let target = if pm == tm {
                source
            } else if tm {
                romacs_chars::unibyte_to_char(source as u8)
            } else {
                source & 255
            };
            let pat = encoded(&[source], pm);
            let text = encoded(&[target], tm);
            for split in 0..=text.len() {
                let h = Haystack {
                    s1: &text[..split],
                    s2: &text[split..],
                    multibyte: tm,
                    point: None,
                };
                for back in [false, true] {
                    let mut st = Stats::default();
                    assert_eq!(
                        translated_literal(&pat, pm, &h, back, |c| c, &mut st),
                        Some((0, text.len()))
                    );
                    let at = if back { text.len() } else { 0 };
                    assert_eq!(
                        translated_literal(
                            &[],
                            pm,
                            &h,
                            back,
                            |_| panic!("empty must not translate"),
                            &mut st
                        ),
                        Some((at, at))
                    );
                    assert_eq!(st.window_bytes, 0);
                }
            }
        }
    }
}

#[test]
fn translated_literal_linear_work_and_large_pattern() {
    let mut text = vec![b'A'; 4096];
    text.push(b'b');
    let mut pat = vec![b'a'; 2048];
    pat.push(b'b');
    let h = Haystack {
        s1: &text[..2049],
        s2: &text[2049..],
        multibyte: false,
        point: None,
    };
    let mut st = Stats::default();
    let mut calls = 0;
    assert_eq!(
        translated_literal(
            &pat,
            false,
            &h,
            true,
            |c| {
                calls += 1;
                if c == 65 { 97 } else { c }
            },
            &mut st
        ),
        Some((2048, text.len()))
    );
    assert_eq!(calls, pat.len() + text.len());
    assert!(st.verify_bytes <= 2 * text.len() as u64);
    assert_eq!(st.window_bytes, 0);
    let short = Haystack {
        s1: b"A",
        s2: &[],
        multibyte: false,
        point: None,
    };
    assert_eq!(
        translated_literal(&pat, false, &short, false, |c| c, &mut st),
        None
    );
}
struct Cycle;
impl CaseTable for Cycle {
    fn canon(&self, c: u32) -> u32 {
        match c {
            65 => 97,
            0xc9 => 0xe9,
            _ => c,
        }
    }
    fn eqv_next(&self, c: u32) -> u32 {
        match c {
            65 => 97,
            97 => 65,
            0xc9 => 0xe9,
            0xe9 => 0xc9,
            _ => c,
        }
    }
}
fn encoded(cs: &[u32], mb: bool) -> Vec<u8> {
    let mut v = Vec::new();
    for &c in cs {
        if mb {
            let mut b = [0; 5];
            let n = romacs_chars::char_string(c, &mut b);
            v.extend_from_slice(&b[..n]);
        } else {
            v.push(c as u8);
        }
    }
    v
}
#[test]
fn naive_char_model_all_starts_directions_splits_and_modes() {
    let rounds = if cfg!(debug_assertions) { 32 } else { 512 };
    let mut seed = 71u32;
    for _ in 0..rounds {
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed as usize
        };
        for pm in [false, true] {
            for tm in [false, true] {
                for fold in [false, true] {
                    let alphabet = [97, 65, 98, 0xe9, 0xc9, 10, romacs_chars::byte8_to_char(255)];
                    let text: Vec<_> = (0..next() % 15)
                        .map(|_| alphabet[next() % 7])
                        .map(|c| if tm { c } else { c & 255 })
                        .collect();
                    let pat: Vec<_> = (0..next() % 5)
                        .map(|_| alphabet[next() % 7])
                        .map(|c| if pm { c } else { c & 255 })
                        .collect();
                    let bytes = encoded(&text, tm);
                    let pattern = encoded(&pat, pm);
                    let wanted: Vec<_> = pat
                        .iter()
                        .map(|&c| {
                            if pm == tm {
                                c
                            } else if tm {
                                romacs_chars::unibyte_to_char(c as u8)
                            } else {
                                c & 255
                            }
                        })
                        .collect();
                    let canon = |c| if fold { Cycle.canon(c) } else { c };
                    let mut offsets = vec![0];
                    for c in &text {
                        offsets.push(
                            offsets.last().unwrap()
                                + if tm { romacs_chars::char_bytes(*c) } else { 1 },
                        );
                    }
                    let re =
                        Regex::literal(&pattern, pm, tm, fold.then_some(&Cycle as &dyn CaseTable));
                    for (s, &start) in offsets.iter().enumerate() {
                        for back in [false, true] {
                            let possible = |i: usize| {
                                i + wanted.len() <= text.len()
                                    && text[i..i + wanted.len()]
                                        .iter()
                                        .zip(&wanted)
                                        .all(|(&a, &b)| canon(a) == canon(b))
                            };
                            let expected = if back {
                                (0..=s).rev().find(|&i| possible(i))
                            } else {
                                (s..=text.len()).find(|&i| possible(i))
                            };
                            for k in 0..=bytes.len() {
                                let h = Haystack {
                                    s1: &bytes[..k],
                                    s2: &bytes[k..],
                                    multibyte: tm,
                                    point: None,
                                };
                                if start == if back { bytes.len() } else { 0 } {
                                    let mut st = Stats::default();
                                    let translated =
                                        translated_literal(&pattern, pm, &h, back, canon, &mut st);
                                    assert_eq!(
                                        translated,
                                        expected.map(|i| (offsets[i], offsets[i + wanted.len()])),
                                        "translated {text:?} {pat:?} {pm}/{tm}/{fold} {back}/{k}",
                                    );
                                    assert_eq!(st.window_bytes, 0);
                                }
                                let mut regs = Regs::default();
                                let got = re
                                    .search(
                                        &h,
                                        start,
                                        if back {
                                            -(start as isize)
                                        } else {
                                            (bytes.len() - start) as isize
                                        },
                                        bytes.len(),
                                        &(),
                                        &mut regs,
                                        &mut Stats::default(),
                                    )
                                    .unwrap();
                                assert_eq!(
                                    got,
                                    expected.map(|i| offsets[i]),
                                    "{text:?} {pat:?} {pm}/{tm}/{fold} {start}/{back}/{k}"
                                );
                                if let Some(i) = expected {
                                    assert_eq!(regs.start, [offsets[i] as isize]);
                                    assert_eq!(regs.end, [offsets[i + wanted.len()] as isize]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
