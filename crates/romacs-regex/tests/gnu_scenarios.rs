mod support;
use romacs_regex::*;
use support::*;
#[test]
fn gap_bounds_empty_and_backward() {
    for k in 0..=11 {
        let text = b"hello world";
        assert_eq!(
            find(
                &Regex::literal(b"llo wo", false, false, None),
                text,
                k,
                false,
                0,
                11,
                11
            ),
            Some((2, 8))
        );
        assert_eq!(
            find(
                &Regex::literal(b"lo w", false, false, None),
                text,
                k,
                false,
                11,
                -11,
                11
            ),
            Some((3, 7))
        );
        assert_eq!(
            find(
                &Regex::literal(b"llo wo", false, false, None),
                text,
                k,
                false,
                0,
                2,
                7
            ),
            None
        );
        assert_eq!(
            find(
                &Regex::literal(b"", false, false, None),
                text,
                k,
                false,
                3,
                -3,
                11
            ),
            Some((3, 3))
        );
    }
    let re = Regex::literal(b"abc", false, false, None);
    assert_eq!(find(&re, b"abcabc", 3, false, 0, 2, 6), Some((0, 3)));
    assert_eq!(find(&re, b"abcabc", 3, false, 4, -2, 6), Some((3, 6)));
    assert_eq!(find(&re, b"abcabc", 3, false, 0, 6, 2), None);
}
#[test]
fn gnu_case_cycles_at_every_byte_split() {
    for (pat, text, want) in [
        ("σ", "Σς", Some((0, 2))),
        ("ς", "Σ", Some((0, 2))),
        ("ǆ", "xǅy", Some((1, 3))),
        ("Ǆ", "xǅy", Some((1, 3))),
        ("ǅ", "ǆ", Some((0, 2))),
        ("k", "K", None),
        ("s", "ſ", None),
        ("σας", "ΣΑΣ", Some((0, 6))),
        ("straße", "Straße STRASSE strasse", Some((0, 7))),
    ] {
        let re = Regex::literal(pat.as_bytes(), true, true, Some(&Fold));
        for k in 0..=text.len() {
            assert_eq!(
                find(
                    &re,
                    text.as_bytes(),
                    k,
                    true,
                    0,
                    text.len() as isize,
                    text.len()
                ),
                want,
                "{pat} / {text}, {k}"
            );
        }
    }
}
#[test]
fn accepted_fold_byte_length_divergences() {
    // D-R8: GNU rejects the short subject when canonical pattern bytes exceed lim.
    for pair in [["Ⱥ", "ⱥ"], ["Ⱦ", "ⱦ"], ["ẞ", "ß"]] {
        for pat in pair {
            let re = Regex::literal(pat.as_bytes(), true, true, Some(&Fold));
            for subject in pair {
                for text in [subject.to_owned(), format!("{subject}x")] {
                    for split in 0..=text.len() {
                        for back in [false, true] {
                            for stop in [0, subject.len(), text.len()] {
                                let start = if back { text.len() } else { 0 };
                                let range = if back {
                                    -(start as isize)
                                } else {
                                    text.len() as isize
                                };
                                assert_eq!(
                                    find(&re, text.as_bytes(), split, true, start, range, stop),
                                    (stop >= subject.len()).then_some((0, subject.len())),
                                    "{pat}/{text} split={split} back={back} stop={stop}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    struct Wide(u32);
    impl CaseTable for Wide {
        fn canon(&self, c: u32) -> u32 {
            if c == 97 { self.0 } else { c }
        }
        fn eqv_next(&self, c: u32) -> u32 {
            if c == 97 {
                self.0
            } else if c == self.0 {
                97
            } else {
                c
            }
        }
    }
    // D-R9: GNU truncates the translated unibyte pattern, but not the subject.
    for (canonical, pat, pm) in [
        (256, &b"a"[..], false),
        (256, "Ā".as_bytes(), true),
        (233, &b"a"[..], false),
    ] {
        let re = Regex::literal(pat, pm, false, Some(&Wide(canonical)));
        // Initial copy_text coercion still applies: multibyte Ā becomes byte zero.
        for text in [&b"a"[..], &b"ax"[..]] {
            for split in 0..=text.len() {
                for back in [false, true] {
                    for stop in 0..=text.len() {
                        let n = text.len() as isize;
                        assert_eq!(
                            find(
                                &re,
                                text,
                                split,
                                false,
                                if back { text.len() } else { 0 },
                                if back { -n } else { n },
                                stop
                            ),
                            (stop >= 1 && !pm).then_some((0, 1))
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn raw_bytes_and_copy_text_coercion() {
    let raw = romacs_chars::str_to_multibyte(&[0xff]);
    for (pat, pm, text, tm, want) in [
        (&[0xff][..], false, raw.as_slice(), true, Some((0, 2))),
        ("é".as_bytes(), true, &[0xe9][..], false, Some((0, 1))),
        ("Ā".as_bytes(), true, &[0][..], false, Some((0, 1))),
        (&[0xff][..], false, "ÿ".as_bytes(), true, None),
    ] {
        for k in 0..=text.len() {
            assert_eq!(
                find(
                    &Regex::literal(pat, pm, tm, None),
                    text,
                    k,
                    tm,
                    0,
                    text.len() as isize,
                    text.len()
                ),
                want
            );
        }
    }
}
#[test]
fn custom_cycles_change_match_length_without_expansion() {
    // A window can begin inside é and contain a non-crossing short match.

    struct Variable;
    impl CaseTable for Variable {
        fn canon(&self, c: u32) -> u32 {
            if c == 0xe9 { 97 } else { c }
        }
        fn eqv_next(&self, c: u32) -> u32 {
            match c {
                97 => 0xe9,
                0xe9 => 97,
                _ => c,
            }
        }
    }
    for (pat, text, end) in [("aa", "éaé", 3), ("éé", "aaé", 2)] {
        let re = Regex::literal(pat.as_bytes(), true, true, Some(&Variable));
        for split in 0..=text.len() {
            assert_eq!(
                find(
                    &re,
                    text.as_bytes(),
                    split,
                    true,
                    0,
                    text.len() as isize,
                    text.len()
                ),
                Some((0, end))
            );
            let last = if text == "éaé" { 2 } else { 1 };
            assert_eq!(
                find(
                    &re,
                    text.as_bytes(),
                    split,
                    true,
                    text.len(),
                    -(text.len() as isize),
                    text.len()
                ),
                Some((last, text.len()))
            );
        }
    }
    let re = Regex::literal(b"aa", true, true, Some(&Variable));
    for split in 0..=5 {
        assert_eq!(
            find(&re, "éaax".as_bytes(), split, true, 5, -5, 5),
            Some((2, 4))
        );
    }
}

#[test]
fn matches_returns_length_and_failure_preserves_regs() {
    let re = Regex::literal(b"bc", false, false, None);
    let h = Haystack {
        s1: b"ab",
        s2: b"c",
        multibyte: false,
        point: Some(1),
    };
    let mut regs = Regs::default();
    let mut st = Stats::default();
    assert_eq!(
        re.matches(&h, 1, 3, &(), &mut regs, &mut st).unwrap(),
        Some(2)
    );
    assert_eq!(re.matches(&h, 0, 3, &(), &mut regs, &mut st).unwrap(), None);
    assert_eq!(regs.start, [1]);
    assert_eq!(regs.end, [3]);
    assert_eq!(re.nsub(), 0);
}
