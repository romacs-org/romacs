use romacs_chars::CharPos as P;
use romacs_props::{Op, PropSets, Props, SetId};

#[test]
fn nil_absent_next() {
    let mut s = PropSets::new();
    let nil = s.intern(&[(2u64, 0)]);
    let mut p = Props::filled(2, nil);
    p.insert(P(2), 2, SetId::EMPTY);
    assert_eq!(
        p.next_single(P(0), P(100), |id| s.get(id, 2).unwrap_or(0)),
        None
    );
    assert_eq!(p.next_change(&s, P(0), P(4)), Some(P(2)));
}

#[test]
fn nil_absent_previous() {
    let mut s = PropSets::new();
    let nil = s.intern(&[(2u64, 0)]);
    let mut p = Props::filled(2, nil);
    p.insert(P(2), 2, SetId::EMPTY);
    assert_eq!(
        p.prev_single(P(4), P(0), |id| s.get(id, 2).unwrap_or(0)),
        None
    );
}

#[test]
fn category_boundaries() {
    let mut s = PropSets::new();
    let a = s.intern(&[(9u64, 10)]);
    let b = s.intern(&[(9, 11)]);
    let mut p = Props::filled(2, a);
    p.insert(P(2), 2, b);
    let effective = |id| s.get(id, 2).unwrap_or_else(|| s.get(id, 9).unwrap_or(0));
    assert_eq!(p.next_single(P(0), P(4), effective), Some(P(2)));
    assert_eq!(p.prev_single(P(4), P(0), effective), Some(P(2)));
    assert_eq!(p.next_single(P(0), P(2), effective), None);
    assert_eq!(p.prev_single(P(4), P(2), effective), None);
    p.apply(&mut s, P(0), P(2), Op::Add(&[(2, 0)]));
    assert_eq!(
        p.next_single(P(0), P(100), |id| {
            s.get(id, 2).unwrap_or_else(|| s.get(id, 9).unwrap_or(0))
        }),
        Some(P(2))
    );
}

#[test]
fn last_leaf_collapse() {
    let mut s = PropSets::new();
    let a = s.intern(&[(2u64, 10)]);
    let b = s.intern(&[(2, 11)]);
    let mut p = Props::new();
    for i in 0..130 {
        p.insert(P(i), 1, if i % 2 == 0 { a } else { b });
        p.check_invariants(&s);
    }
    p.apply(&mut s, P(64), P(130), Op::Set(a));
    for i in 0..130 {
        assert_eq!(p.run_at(P(i)).2, if i >= 64 || i % 2 == 0 { a } else { b });
    }
    p.check_invariants(&s);
}

#[test]
fn reintern_grace() {
    let mut s = PropSets::new();
    s.intern(&[(2u64, 17)]);
    s.trace_young(&mut |_| {});
    s.sweep();
    let id = s.intern(&[(2, 17)]);
    s.trace_young(&mut |_| {});
    assert_eq!(s.sweep(), 0);
    assert_eq!(s.pairs(id), &[(2, 17)]);
    assert_eq!(s.sweep(), 1);
}

#[test]
fn production_split_merge_model() {
    fn check(p: &Props, s: &PropSets<u64>, model: &[SetId]) {
        p.check_invariants(s);
        assert_eq!(p.len(), model.len());
        for (i, &id) in model.iter().enumerate() {
            assert_eq!(p.run_at(P(i)).2, id);
        }
    }
    let mut s = PropSets::new();
    let a = s.intern(&[(2u64, 10)]);
    let b = s.intern(&[(2, 11)]);
    for descending in [false, true] {
        let n = if cfg!(debug_assertions) { 130 } else { 2048 };
        let mut p = Props::filled(n * 8, SetId::EMPTY);
        let mut model = vec![SetId::EMPTY; n * 8];
        for step in 0..n {
            let i = if descending { n - step - 1 } else { step };
            let pos = i * 8 + 1;
            p.run_at(P(p.len() - 1));
            p.apply(&mut s, P(pos), P(pos + 5), Op::Set(a));
            model[pos..pos + 5].fill(a);
            check(&p, &s, &model);
        }
        for i in (0..n).rev() {
            p.apply(&mut s, P(i * 8), P((i + 1) * 8), Op::Set(a));
            model[i * 8..(i + 1) * 8].fill(a);
            check(&p, &s, &model);
        }
        for step in 0..if cfg!(debug_assertions) { 32 } else { 800 } {
            let mut src = Props::filled(5, a);
            src.insert(P(5), 7, b);
            p.insert_props(P(0), &src);
            model.splice(0..0, [a; 5].into_iter().chain([b; 7]));
            check(&p, &s, &model);
            let start = (step * 17) % (p.len() - 31);
            p.delete(P(start), P(start + 19));
            model.drain(start..start + 19);
            check(&p, &s, &model);
            if step % 17 == 0 {
                let end = (start + 30).min(p.len());
                let copy = p.slice(P(start), P(end));
                let copied = model[start..end].to_vec();
                p.insert_props(P(0), &copy);
                model.splice(0..0, copied);
                check(&p, &s, &model);
            }
        }
    }
}
