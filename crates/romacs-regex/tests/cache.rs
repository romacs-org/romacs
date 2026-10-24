use romacs_regex::*;
fn key() -> KeyRef<'static> {
    KeyRef {
        kind: Kind::Literal,
        pattern: b"key",
        pattern_multibyte: false,
        target_multibyte: false,
        posix: false,
        whitespace: None,
        case: Some((1, 1)),
    }
}
fn get(c: &mut Cache, k: KeyRef, pat: &[u8], st: &mut Stats) -> std::rc::Rc<Regex> {
    c.get(k, || Ok(Regex::literal(pat, false, false, None)), st)
        .unwrap()
}
fn result(r: &Regex) -> Option<usize> {
    r.search(
        &Haystack {
            s1: b"ab",
            s2: b"",
            multibyte: false,
            point: None,
        },
        0,
        2,
        2,
        &(),
        &mut Regs::default(),
        &mut Stats::default(),
    )
    .unwrap()
}
#[test]
fn each_key_component_separates_programs() {
    for i in 0..8 {
        let mut c = Cache::new();
        let mut st = Stats::default();
        let a = key();
        let mut b = a;
        match i {
            0 => b.case = Some((1, 2)),
            1 => b.case = Some((2, 1)),
            2 => b.posix = true,
            3 => b.target_multibyte = true,
            4 => b.pattern_multibyte = true,
            5 => b.whitespace = Some(b" "),
            6 => b.kind = Kind::Regexp,
            _ => b.pattern = b"other",
        }
        assert_eq!(result(&get(&mut c, a, b"a", &mut st)), Some(0));
        assert_eq!(
            result(&get(&mut c, b, b"b", &mut st)),
            Some(1),
            "key component {i}"
        );
        assert_eq!(result(&get(&mut c, a, b"wrong", &mut st)), Some(0));
        assert_eq!(st.compiles, 2);
        assert_eq!(st.hits, 1);
    }
}
#[test]
fn case_table_mutation_rebuilds_snapshot() {
    struct Table(bool);
    impl CaseTable for Table {
        fn canon(&self, c: u32) -> u32 {
            if self.0 && c == 97 { 98 } else { c }
        }
        fn eqv_next(&self, c: u32) -> u32 {
            if self.0 {
                match c {
                    97 => 98,
                    98 => 97,
                    _ => c,
                }
            } else {
                c
            }
        }
    }
    let mut table = Table(false);
    let mut cache = Cache::new();
    let mut st = Stats::default();
    let mut k = KeyRef {
        pattern: b"b",
        ..key()
    };
    let original = cache
        .get(
            k,
            || Ok(Regex::literal(b"b", false, false, Some(&table))),
            &mut st,
        )
        .unwrap();
    assert_eq!(result(&original), Some(1));
    table.0 = true; // Mutate both canon and eqv; the old compiled snapshot stays immutable.
    k.case = Some((1, 2));
    let changed = cache
        .get(
            k,
            || Ok(Regex::literal(b"b", false, false, Some(&table))),
            &mut st,
        )
        .unwrap();
    assert_eq!(result(&changed), Some(0));
    assert_eq!(result(&original), Some(1));
    let reused = cache
        .get(k, || panic!("stable key must reuse"), &mut st)
        .unwrap();
    assert!(std::rc::Rc::ptr_eq(&changed, &reused));
    table.0 = false;
    k.case = Some((2, 2)); // A separate owner identity must not reuse the previous table.
    let replaced = cache
        .get(
            k,
            || Ok(Regex::literal(b"b", false, false, Some(&table))),
            &mut st,
        )
        .unwrap();
    assert_eq!(result(&replaced), Some(1));
    assert_eq!((st.compiles, st.hits), (3, 1));
}
#[test]
fn lru_promotes_hits_and_evicts_tail() {
    let mut c = Cache::new();
    let mut st = Stats::default();
    for i in 0..128 {
        get(
            &mut c,
            KeyRef {
                pattern: &[i],
                ..key()
            },
            b"a",
            &mut st,
        );
    }
    get(
        &mut c,
        KeyRef {
            pattern: &[0],
            ..key()
        },
        b"bad",
        &mut st,
    );
    get(
        &mut c,
        KeyRef {
            pattern: &[128],
            ..key()
        },
        b"b",
        &mut st,
    );
    get(
        &mut c,
        KeyRef {
            pattern: &[0],
            ..key()
        },
        b"bad",
        &mut st,
    );
    assert_eq!(st.compiles, 129);
    assert_eq!(st.hits, 2);
    get(
        &mut c,
        KeyRef {
            pattern: &[1],
            ..key()
        },
        b"a",
        &mut st,
    );
    assert_eq!(st.compiles, 130);
}
#[test]
fn failed_build_is_not_cached() {
    let mut c = Cache::new();
    let mut st = Stats::default();
    assert!(c.get(key(), || Err(RegexError), &mut st).is_err());
    assert_eq!(result(&get(&mut c, key(), b"b", &mut st)), Some(1));
    assert_eq!(st.hits, 0);
}

#[path = "../bench/org_corpus.rs"]
mod org_corpus;

#[test]
fn cyclic_org_working_set_only_compiles_first_round() {
    let patterns = org_corpus::patterns();
    let mut cache = Cache::new();
    let mut stats = Stats::default();
    for round in 0..100 {
        for pattern in &patterns {
            let k = KeyRef {
                kind: Kind::Regexp,
                pattern,
                ..key()
            };
            cache
                .get(
                    k,
                    || {
                        assert_eq!(round, 0, "warm Org pattern must be retained");
                        Ok(Regex::literal(pattern, false, false, None))
                    },
                    &mut stats,
                )
                .unwrap();
        }
        assert_eq!(stats.compiles, 60);
        assert_eq!(stats.hits, round * 60);
    }
}
