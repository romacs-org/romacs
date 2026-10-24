use super::*;
fn alternating(n: usize, id: SetId) -> Props {
    let mut p = Props::new();
    for i in 0..n {
        p.insert(
            CharPos(p.len()),
            8,
            if i % 2 == 0 { id } else { SetId::EMPTY },
        );
    }
    p
}
#[test]
fn cached_leaf_after_structural_edits() {
    let mut sets = PropSets::new();
    let id = sets.intern(&[(2u64, 10)]);
    let mut p = alternating(32, id);
    let mut shifted = p.clone();
    assert_eq!(shifted.run_at(CharPos(248)).2, SetId::EMPTY);
    shifted.insert(CharPos(1), 5, SetId::EMPTY);
    assert_eq!(shifted.run_at(CharPos(248)).2, id);
    let mut end_graft = Props::filled(100, SetId::EMPTY);
    end_graft.run_at(CharPos(0));
    end_graft.insert_props(CharPos(100), &alternating(32, id));
    end_graft.check_invariants(&sets);
    for from in [1, 7, 8, 15, 16, 23, 31, 63, 127] {
        for to in [17, 33, 65, 129, 255] {
            if from >= to {
                continue;
            }
            let mut candidate = p.clone();
            candidate.delete(CharPos(from), CharPos(to));
            for q in 0..candidate.len() {
                let original = if q < from { q } else { q + to - from };
                let expected = if (original / 8).is_multiple_of(2) {
                    id
                } else {
                    SetId::EMPTY
                };
                assert_eq!(
                    candidate.run_at(CharPos(q)).2,
                    expected,
                    "delete {from}..{to} query {q}"
                );
            }
            candidate.check_invariants(&sets);
        }
    }
    assert_eq!(p.run_at(CharPos(248)).2, SetId::EMPTY);
    p.delete(CharPos(0), CharPos(128));
    assert_eq!(
        p.run_at(CharPos(120)),
        (CharPos(120), CharPos(128), SetId::EMPTY)
    );
    p.insert(CharPos(1), 5, SetId::EMPTY);
    assert_eq!(
        p.run_at(CharPos(125)),
        (CharPos(125), CharPos(133), SetId::EMPTY)
    );
    p.check_invariants(&sets);
    let src = alternating(16, id);
    p.insert_props(CharPos(0), &src);
    assert_eq!(
        p.run_at(CharPos(253)),
        (CharPos(253), CharPos(261), SetId::EMPTY)
    );
    p.apply(&mut sets, CharPos(0), CharPos(253), Op::Set(id));
    assert_eq!(
        p.run_at(CharPos(253)),
        (CharPos(253), CharPos(261), SetId::EMPTY)
    );
    p.check_invariants(&sets);
}

#[test]
fn consecutive_advancing_insertions_refresh_summary() {
    let mut sets = PropSets::new();
    let id = sets.intern(&[(2u64, 10)]);
    let mut single = Props::filled(512, id);
    for at in 1..64 {
        single.insert(CharPos(at), 1, id);
        single.check_invariants(&sets);
        assert_eq!(single.run_at(CharPos(single.len() - 1)).2, id);
    }
    let mut p = alternating(128, id);
    for step in 0..128 {
        let at = 8 + step;
        p.run_at(CharPos(p.len() - 1));
        p.insert(CharPos(at), 1, id);
        assert_eq!(p.len(), 1024 + step + 1);
        assert_eq!(p.run_at(CharPos(p.len() - 1)).2, SetId::EMPTY);
        p.check_invariants(&sets);
    }
}

#[test]
fn arena_split_and_reuse() {
    let mut sets = PropSets::new();
    let id = sets.intern(&[(2u64, 10)]);
    let mut p = alternating(8192, id);
    p.check_invariants(&sets);
    p.delete(CharPos(0), CharPos(p.len()));
    p.check_invariants(&sets);
    for i in 0..8192 {
        p.insert(
            CharPos(p.len()),
            8,
            if i % 2 == 0 { id } else { SetId::EMPTY },
        );
    }
    p.check_invariants(&sets);
}

#[test]
fn every_structural_operation() {
    for op in 0..7 {
        let mut counts = vec![];
        for n in [1024, 2048] {
            let mut s = PropSets::new();
            let id = s.intern(&[(2u64, 10)]);
            let base = alternating(n, id);
            let mut work = 0;
            for fraction in [0, 2, 4] {
                let mut p = base.clone();
                let at = p.len() * fraction / 8;
                let end = (at + p.len() / 2).min(p.len());
                p.reset_work();
                match op {
                    0 => {
                        p.apply(&mut s, CharPos(at), CharPos(end), Op::Set(id));
                    }
                    1 => {
                        p.apply(&mut s, CharPos(at), CharPos(end), Op::Add(&[(3, 11)]));
                    }
                    2 => {
                        p.apply(&mut s, CharPos(0), CharPos(p.len()), Op::Add(&[(3, 11)]));
                        p.apply(&mut s, CharPos(at), CharPos(end), Op::Remove(&[3]));
                    }
                    3 => p.insert_props(CharPos(at), &base),
                    4 => {
                        let src = base.slice(CharPos(0), CharPos(end - at));
                        p.merge_under(&mut s, CharPos(at), &src);
                    }
                    5 => p.delete(CharPos(at), CharPos(end)),
                    _ => {
                        p.insert(CharPos(at + 1), 3, SetId::EMPTY);
                        p.run_at(CharPos(p.len() - 1));
                    }
                }
                work += p.measured_work();
                p.check_invariants(&s);
            }
            counts.push(work);
        }
        println!("op={op} n=1024 {} n=2048 {}", counts[0], counts[1]);
        assert!(counts[0] > 0 && counts[1] < 3 * counts[0]);
    }
}
#[test]
fn advancing_splits_and_collapsing_merges() {
    for front in [false, true] {
        let mut counts = vec![];
        for n in [1024, 2048] {
            let mut s = PropSets::new();
            let id = s.intern(&[(2u64, 10)]);
            let mut p = Props::filled(8 * n, SetId::EMPTY);
            for i in 0..n {
                let at = if front { 4 * (n - i - 1) } else { 4 * i };
                p.apply(&mut s, CharPos(at), CharPos(at + 2), Op::Set(id));
            }
            p.apply(&mut s, CharPos(0), CharPos(p.len()), Op::Set(id));
            p.check_invariants(&s);
            assert_eq!(p.runs(CharPos(0), CharPos(p.len())).count(), 1);
            counts.push(p.measured_work());
        }
        println!("advancing front={front}: {} -> {}", counts[0], counts[1]);
        assert!(counts[1] < 3 * counts[0]);
    }
}

#[test]
fn structural_work_scaling() {
    fn stream(n: usize) -> usize {
        let mut p = Props::new();
        let mut s = PropSets::new();
        let id = s.intern(&[(2u64, 3)]);
        for i in 0..n {
            p.insert(
                CharPos(p.len()),
                3,
                if i % 2 == 0 { id } else { SetId::EMPTY },
            );
        }
        for _ in 0..n {
            let end = p.len();
            p.apply(&mut s, CharPos(end - 2), CharPos(end), Op::Set(id));
            p.delete(CharPos(end - 2), CharPos(end));
            p.insert(CharPos(p.len()), 3, SetId::EMPTY);
        }
        p.check_invariants(&s);
        p.measured_work()
    }
    let a = stream(2000);
    let b = stream(4000);
    println!("structural work n=2000 {a}, n=4000 {b}");
    assert!(b < 3 * a, "quadratic structural work: {a} -> {b}");
}
