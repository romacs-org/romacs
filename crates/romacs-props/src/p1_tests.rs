use super::*;

#[test]
fn u32_overflow_runs_hide_boundaries() {
    let mut sets = PropSets::new();
    let id = sets.intern(&[(1u64, 2)]);
    let max = u32::MAX as usize;
    let mut p = Props::filled(max + 123, id);
    assert_eq!(p.next_change(&sets, CharPos(0), CharPos(p.len())), None);
    p.insert(CharPos(50), 5, SetId::EMPTY);
    p.check_invariants(&sets);
    p.delete(CharPos(50), CharPos(55));
    p.check_invariants(&sets);
    assert_eq!(p.len(), max + 123);
    assert_eq!(
        p.next_single(CharPos(0), CharPos(p.len()), |s| sets.get(s, 1)),
        None
    );
    p.apply(
        &mut sets,
        CharPos(max - 5),
        CharPos(max + 5),
        Op::Set(SetId::EMPTY),
    );
    p.check_invariants(&sets);
    assert_eq!(p.run_at(CharPos(max)).2, SetId::EMPTY);
    p.delete(CharPos(0), CharPos(max));
    p.check_invariants(&sets);
    let mut p = Props::filled(10, id);
    p.insert(CharPos(10), 10, SetId::EMPTY);
    p.insert(CharPos(10), max + 1, id);
    p.check_invariants(&sets);
    assert_eq!(p.runs(CharPos(0), CharPos(p.len())).count(), 3);
    let mut p = Props::filled(10, id);
    p.insert(CharPos(5), max + 10, id);
    p.check_invariants(&sets);
    let mut p = Props::filled(max - 5, id);
    p.insert(CharPos(p.len()), 10, id);
    p.check_invariants(&sets);
}

#[test]
fn append_random_and_cursor_work_gates() {
    let mut s = PropSets::new();
    let id = s.intern(&[(1u64, 2)]);
    let mut p = Props::new();
    p.tree.work.set(0);
    for i in 0..200_000 {
        p.insert(
            CharPos(p.len()),
            2,
            if i % 2 == 0 { id } else { SetId::EMPTY },
        );
    }
    let append_work = p.tree.work.get();
    assert!(
        append_work > 200_000,
        "must account for spine work: {append_work}"
    );
    assert!(
        append_work < 6 * 200_000,
        "tail-only amortized work: {append_work}"
    );
    let runs = p.runs(CharPos(0), CharPos(p.len())).count();
    assert!(p.allocated_bytes() as f64 / runs as f64 <= 30.25);
    for pos in [3, 177_777, 398_123, 19, 244_444] {
        p.finger.set(None);
        p.reset_work();
        p.run_at(CharPos(pos));
        assert!(p.measured_work() <= 6, "one leaf plus depth nodes");
    }
    p.reset_work();
    let mut at = CharPos(0);
    let mut boundaries = 0;
    while let Some(next) = p.next_single(at, CharPos(p.len()), |s| s) {
        boundaries += 1;
        at = next;
    }
    assert_eq!(boundaries, 199_999);
    assert!(p.measured_work() <= 2 * boundaries + 6);
}

#[test]
fn flag_summary_matches_naive_after_mutation_and_collection() {
    let mut s = PropSets::with_flag_classifier(|key: u64, _| {
        if key == 9 {
            PropSets::<u64>::INVISIBLE
        } else {
            0
        }
    });
    let marked = s.intern(&[(9, 1)]);
    let other = s.intern(&[(2, 3)]);
    let mut p = Props::new();
    for i in 0..4096 {
        p.insert(
            CharPos(p.len()),
            3,
            if i == 3051 {
                marked
            } else if i % 2 == 0 {
                other
            } else {
                SetId::EMPTY
            },
        );
    }
    fn compare(p: &mut Props, s: &PropSets<u64>, from: usize, to: usize) {
        let want = p
            .runs(CharPos(from), CharPos(to))
            .find(|(_, _, id)| s.flags(*id) & PropSets::<u64>::INVISIBLE != 0);
        assert_eq!(
            p.next_flagged(s, CharPos(from), CharPos(to), PropSets::<u64>::INVISIBLE),
            want
        );
    }
    for (a, b) in [
        (0, p.len()),
        (9154, 10000),
        (0, 10),
        (9155, 9156),
        (9154, 9154),
        (0, 0),
        (p.len(), p.len()),
    ] {
        compare(&mut p, &s, a, b);
    }
    p.insert(CharPos(0), 4, marked);
    compare(&mut p, &s, 0, 100);
    p.delete(CharPos(1), CharPos(999));
    let end = p.len();
    compare(&mut p, &s, 1, end);
    p.apply(&mut s, CharPos(1), CharPos(2000), Op::Add(&[(9, 2)]));
    compare(&mut p, &s, 0, 3000);
    p.trace(&s, &mut |_| {});
    s.sweep();
    let end = p.len();
    compare(&mut p, &s, 0, end);
}

#[test]
fn duplicate_add_final_identity_does_not_split_single_run() {
    let mut s = PropSets::new();
    let id = s.intern(&[(7u64, 2)]);
    let mut p = Props::filled(20, id);
    assert!(p.apply(&mut s, CharPos(3), CharPos(17), Op::Add(&[(7, 1), (7, 2)])));
    p.check_invariants(&s);
    assert_eq!(
        p.runs(CharPos(0), CharPos(20)).collect::<Vec<_>>(),
        vec![(CharPos(0), CharPos(20), id)]
    );
}

#[test]
fn duplicate_add_final_identity_across_runs_coalesces() {
    let mut s = PropSets::new();
    let id = s.intern(&[(7u64, 2)]);
    let other = s.intern(&[(7, 2), (8, 3)]);
    let mut p = Props::filled(10, id);
    p.insert(CharPos(10), 10, other);
    p.insert(CharPos(20), 10, id);
    let before = p.runs(CharPos(0), CharPos(30)).collect::<Vec<_>>();
    assert!(p.apply(&mut s, CharPos(3), CharPos(27), Op::Add(&[(7, 1), (7, 2)])));
    p.check_invariants(&s);
    assert_eq!(p.runs(CharPos(0), CharPos(30)).collect::<Vec<_>>(), before);
}

#[test]
fn dirty_flags_stay_local_and_epoch_rebuilds() {
    let mut s = PropSets::with_flag_classifier(|k: u64, _| u8::from(k == 9));
    let id = s.intern(&[(1, 2)]);
    let flagged = s.intern(&[(9, 1)]);
    let mut p = Props::new();
    for i in 0..8192 {
        p.insert(
            CharPos(p.len()),
            4,
            if i % 2 == 0 { id } else { SetId::EMPTY },
        );
    }
    assert_eq!(p.next_flagged(&s, CharPos(0), CharPos(p.len()), 1), None);
    p.apply(&mut s, CharPos(16001), CharPos(16002), Op::Set(flagged));
    p.tree.work.set(0);
    assert_eq!(
        p.next_flagged(&s, CharPos(0), CharPos(p.len()), 1),
        Some((CharPos(16001), CharPos(16002), flagged))
    );
    assert!(
        p.tree.work.get() < 256,
        "local edit scanned {} units",
        p.tree.work.get()
    );
    p.trace(&s, &mut |_| {});
    s.sweep();
    p.tree.work.set(0);
    assert!(
        p.next_flagged(&s, CharPos(0), CharPos(p.len()), 1)
            .is_some()
    );
    assert!(p.tree.work.get() >= 8192, "collection must rebuild");
}

#[test]
fn branch_crossing_edit_model_and_flag_summaries() {
    let mut s = PropSets::with_flag_classifier(|k: u64, _| u8::from(k == 9));
    let ids = [SetId::EMPTY, s.intern(&[(1, 2)]), s.intern(&[(9, 1)])];
    let mut p = Props::new();
    let mut model = Vec::new();
    for i in 0..8192 {
        let id = ids[i % ids.len()];
        p.insert(CharPos(p.len()), 3, id);
        model.extend([id; 3]);
    }
    let mut seed = 971u64;
    for step in 0..600 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = (seed as usize >> 8) % model.len();
        let n = (seed as usize >> 24) % 300 + 1;
        let b = (a + n).min(model.len());
        let id = ids[(seed as usize >> 40) % ids.len()];
        match step % 3 {
            0 => {
                p.insert(CharPos(a), n, id);
                model.splice(a..a, std::iter::repeat_n(id, n));
            }
            1 => {
                p.delete(CharPos(a), CharPos(b));
                model.drain(a..b);
            }
            _ => {
                p.apply(&mut s, CharPos(a), CharPos(b), Op::Set(id));
                model[a..b].fill(id);
            }
        }
        if step % 37 == 0 {
            p.trace(&s, &mut |_| {});
            // Fixtures remain rooted even if the current model loses an ID.
            for id in ids {
                s.trace_set(id, &mut |_| {});
            }
            s.sweep();
        }
        p.check_invariants(&s);
        let got: Vec<_> = p
            .runs(CharPos(0), CharPos(p.len()))
            .flat_map(|(a, b, id)| std::iter::repeat_n(id, b - a))
            .collect();
        assert_eq!(got, model, "step {step}");
        for (from, to) in [
            (0, model.len()),
            (a.min(model.len()), (a + 500).min(model.len())),
        ] {
            let want = p
                .runs(CharPos(from), CharPos(to))
                .find(|(_, _, id)| s.flags(*id) & 1 != 0);
            assert_eq!(
                p.next_flagged(&s, CharPos(from), CharPos(to), 1),
                want,
                "step {step}"
            );
        }
    }
}

#[test]
fn dirty_queue_is_bounded_across_leaf_reuse_without_queries() {
    let mut s = PropSets::new();
    let id = s.intern(&[(1u64, 1)]);
    let mut p = Props::new();
    for i in 0..64 {
        p.insert(
            CharPos(p.len()),
            1,
            if i % 2 == 0 { id } else { SetId::EMPTY },
        );
    }
    let before = p.allocated_bytes();
    for _ in 0..10_000 {
        p.insert(CharPos(64), 1, id);
        p.delete(CharPos(64), CharPos(65));
    }
    p.check_invariants(&s);
    assert!(p.allocated_bytes() < before + 4096);
    assert_eq!(p.next_flagged(&s, CharPos(0), CharPos(64), 255), None);
}
