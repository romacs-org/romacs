use romacs_chars::CharPos as P;
use romacs_props::{Op, PropSets, Props, SetId, Sticky};
mod support;
use support::Env;
fn pairs(text: &str, e: &Env) -> Vec<(u64, u64)> {
    text.split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (k, v) = s.split_once(':').unwrap();
            (
                k.parse().unwrap(),
                if let Some(l) = v.strip_prefix('L') {
                    e.list(l.split('+').map(|x| x.parse().unwrap()).collect())
                } else {
                    v.parse().unwrap()
                },
            )
        })
        .collect()
}
fn visible(ps: &[(u64, u64)], e: &Env) -> String {
    ps.iter()
        .map(|&(k, v)| {
            let value = if v >= 1000 {
                format!(
                    "L{}",
                    e.lists.borrow()[(v - 1000) as usize]
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join("+")
                )
            } else {
                v.to_string()
            };
            format!("{k}:{value}")
        })
        .collect::<Vec<_>>()
        .join(",")
}
#[test]
fn recorded_hidden_boundary_inheritance_gap() {
    for (hidden, gnu_value) in [(false, Some(17)), (true, None)] {
        let mut s = PropSets::new();
        let bold = s.intern(&[(2u64, 10), (support::FRONT, 1)]);
        let mut p = Props::filled(4, bold);
        if hidden {
            p.apply(&mut s, P(2), P(4), Op::Add(&[(2, 11)]));
            p.apply(&mut s, P(2), P(4), Op::Add(&[(2, 10)]));
        }
        let e = Env::default();
        let x = e.inherited(&p, &mut s, 2);
        p.insert(P(2), 1, x);
        p.apply(&mut s, P(2), P(3), Op::Add(&[(3, 17)]));
        let y = e.inherited(&p, &mut s, 2);
        p.insert(P(2), 1, y);
        let kernel_value = s.get(p.run_at(P(2)).2, 3);
        assert_eq!(kernel_value, Some(17));
        assert_eq!(kernel_value == gnu_value, !hidden);
        p.check_invariants(&s);
    }
}

#[test]
fn recorded_stickiness() {
    let mut cases = 0;
    for row in include_str!("sticky-cases.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = row.split('|').collect();
        let mut e = Env::default();
        e.defaults
            .push((if c[0].ends_with("override") { 2 } else { 8 }, true));
        let mut s = PropSets::new();
        let l = s.intern(&pairs(c[1], &e));
        let r = s.intern(&pairs(c[2], &e));
        let mut p = Props::filled(2, l);
        p.insert(P(2), 2, r);
        let at = c[3].parse().unwrap();
        let sticky = p.sticky_at(&mut s, P(at), &e);
        assert_eq!(
            matches!(sticky, Sticky::Build { .. }),
            c[5] == "yes",
            "{}",
            c[0]
        );
        let id = e.materialize(&mut s, sticky);
        assert_eq!(visible(s.pairs(id), &e), c[4], "{}", c[0]);
        p.insert(P(at), 1, id);
        p.check_invariants(&s);
        cases += 1;
    }
    assert_eq!(cases, 19);
}
#[test]
fn order_identity_nil_and_changes() {
    let mut s = PropSets::new();
    let mut p = Props::filled(4, SetId::EMPTY);
    for (k, v) in [(2u64, 10), (3, 11), (2, 12)] {
        p.apply(&mut s, P(0), P(2), Op::Add(&[(k, v)]));
    }
    assert_eq!(s.pairs(p.run_at(P(0)).2), &[(3, 11), (2, 12)]);
    let reverse = s.intern(&[(2, 12), (3, 11)]);
    p.apply(&mut s, P(2), P(4), Op::Set(reverse));
    assert_eq!(p.next_change(&s, P(0), P(4)), None);
    assert!(!p.apply(&mut s, P(0), P(4), Op::Add(&[(2, 12)])));
    assert!(!p.apply(&mut s, P(0), P(4), Op::Remove(&[9])));
    for (a, b) in [(1000, 1001), (0, 1)] {
        let l = s.intern(&[(2, a)]);
        let r = s.intern(&[(2, b)]);
        let mut p = Props::filled(2, l);
        p.insert(P(2), 2, r);
        assert_eq!(p.next_change(&s, P(0), P(4)), Some(P(2)));
    }
    let nil = s.intern(&[(2, 0)]);
    let mut p = Props::filled(2, nil);
    p.insert(P(2), 2, SetId::EMPTY);
    assert_eq!(p.next_change(&s, P(0), P(4)), Some(P(2)));
}
#[test]
fn plain_inherit_copy_delete_replace_queries() {
    let mut s = PropSets::new();
    let bold = s.intern(&[(2u64, 10)]);
    let k = s.intern(&[(3, 1)]);
    let italic = s.intern(&[(2, 11), (3, 1)]);
    let mut p = Props::filled(4, bold);
    p.insert(P(2), 2, SetId::EMPTY);
    p.insert_props(P(4), &Props::filled(1, k));
    assert_eq!(
        p.runs(P(0), P(7))
            .map(|(a, b, s)| (a.0, b.0, s))
            .collect::<Vec<_>>(),
        vec![(0, 2, bold), (2, 4, SetId::EMPTY), (4, 5, k), (5, 7, bold)]
    );
    let e = Env::default();
    let mut p = Props::filled(4, bold);
    let id = e.inherited(&p, &mut s, 2);
    p.insert(P(2), 2, id);
    p.merge_under(&mut s, P(2), &Props::filled(2, italic));
    assert_eq!(s.pairs(p.run_at(P(2)).2), &[(3, 1), (2, 10)]);
    let mut p = Props::filled(6, bold);
    p.apply(&mut s, P(2), P(4), Op::Set(SetId::EMPTY));
    p.delete(P(2), P(4));
    assert_eq!(p.next_change(&s, P(0), P(4)), None);
    for (n, id) in [(5, SetId::EMPTY), (1, SetId::EMPTY), (2, italic)] {
        let mut p = Props::filled(11, SetId::EMPTY);
        p.apply(&mut s, P(4), P(7), Op::Set(bold));
        p.delete(P(4), P(7));
        p.insert_props(P(4), &Props::filled(n, id));
        assert_eq!(p.len(), 8 + n);
        assert_eq!(p.run_at(P(4)).2, id);
    }
    let mut p = Props::filled(8, SetId::EMPTY);
    p.apply(&mut s, P(1), P(3), Op::Add(&[(2, 10)]));
    p.apply(&mut s, P(2), P(5), Op::Add(&[(3, 1)]));
    assert_eq!(
        p.next_single(P(0), P(8), |id| s.get(id, 2).unwrap_or(0)),
        Some(P(1))
    );
    assert_eq!(
        p.next_single(P(1), P(8), |id| s.get(id, 2).unwrap_or(0)),
        Some(P(3))
    );
    assert_eq!(
        p.next_single(P(3), P(100), |id| s.get(id, 2).unwrap_or(0)),
        None
    );
    assert_eq!(
        p.prev_single(P(8), P(0), |id| s.get(id, 3).unwrap_or(0)),
        Some(P(5))
    );
    assert_eq!(
        p.next_single(P(3), P(5), |id| s.get(id, 2).unwrap_or(0)),
        None
    );
    assert_eq!(
        p.next_single(P(0), P(1), |id| s.get(id, 3).unwrap_or(0)),
        None
    );
    assert_eq!(
        p.prev_single(P(2), P(0), |id| s.get(id, 2).unwrap_or(0)),
        Some(P(1))
    );
    assert_eq!(p.next_change(&s, P(0), P(8)), Some(P(1)));
    assert_eq!(p.next_change(&s, P(5), P(8)), None);
    assert_eq!(p.next_change(&s, P(5), P(6)), None);
    assert_eq!(p.prev_change(&s, P(8), P(0)), Some(P(5)));
    assert_eq!(
        p.runs(P(0), P(8))
            .find(|x| s.get(x.2, 3) == Some(1))
            .unwrap()
            .0,
        P(2)
    );
    assert_eq!(
        p.runs(P(0), P(8))
            .find(|x| s.get(x.2, 2).is_some())
            .unwrap()
            .0,
        P(1)
    );
    let empty = Props::new();
    assert_eq!(e.inherited(&empty, &mut s, 0), SetId::EMPTY);
    let mut across = Props::filled(8, SetId::EMPTY);
    across.apply(&mut s, P(0), P(2), Op::Set(k));
    across.apply(&mut s, P(2), P(4), Op::Set(bold));
    across.apply(&mut s, P(4), P(6), Op::Set(k));
    across.delete(P(1), P(5));
    assert_eq!(across.run_at(P(0)), (P(0), P(2), k));
    assert_eq!(across.run_at(P(2)), (P(2), P(4), SetId::EMPTY));
    let copy = p.slice(P(2), P(6));
    assert_eq!(
        copy.runs(P(0), P(4))
            .map(|(a, b, id)| (a.0, b.0, s.pairs(id).to_vec()))
            .collect::<Vec<_>>(),
        vec![
            (0, 1, vec![(3, 1), (2, 10)]),
            (1, 3, vec![(3, 1)]),
            (3, 4, vec![])
        ]
    );
    p.apply(&mut s, P(0), P(8), Op::Remove(&[2, 3]));
    assert!(p.is_plain());
    assert_eq!(p.next_change(&s, P(0), P(8)), None);
    let mut p = Props::filled(400000, SetId::EMPTY);
    for i in 0..100000 {
        p.apply(&mut s, P(4 * i), P(4 * i + 2), Op::Add(&[(2, 10)]));
    }
    assert_eq!(p.runs(P(0), P(400000)).count(), 200000);
}
