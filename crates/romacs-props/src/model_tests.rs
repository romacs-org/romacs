use super::*;
use proptest::prelude::*;
#[path = "../tests/support/mod.rs"]
mod support;
use support::{Env, FRONT, REAR};
type Plist = Vec<(u64, u64)>;
fn get(p: &[(u64, u64)], k: u64) -> Option<u64> {
    p.iter().find(|x| x.0 == k).map(|x| x.1)
}
fn eq(a: &Plist, b: &Plist) -> bool {
    a.len() == b.len() && a.iter().all(|&(k, v)| get(b, k) == Some(v))
}
fn add(p: &mut Plist, ps: &[(u64, u64)]) {
    for &(k, v) in ps {
        if let Some(x) = p.iter_mut().find(|x| x.0 == k) {
            x.1 = v;
        } else {
            p.insert(0, (k, v));
        }
    }
}
// Independent literal per-character GNU sticky model. Its own list arena issues
// the same identities in the same order as the owner, never kernel IDs.
fn sticky(m: &[Plist], at: usize, e: &Env) -> Plist {
    let empty = vec![];
    let l = if at > 0 { &m[at - 1] } else { &empty };
    let r = if at < m.len() { &m[at] } else { &empty };
    let lf = get(l, FRONT).unwrap_or(0);
    let lr = get(l, REAR).unwrap_or(0);
    let rf = get(r, FRONT).unwrap_or(0);
    let rr = get(r, REAR).unwrap_or(0);
    if at > 0 && at < m.len() && l == r {
        let split = if lr != 0 && lr < 1000 {
            true
        } else if lf != 0 && lf < 1000 {
            false
        } else {
            l.iter().any(|&(k, _)| {
                !e.tmem(k, lf) && (e.tmem(k, lr) || e.default_nonsticky(k).is_some())
            })
        };
        if !split {
            return l.clone();
        }
    }
    let mut out = vec![];
    let mut f = vec![];
    let mut rear = vec![];
    for &(k, v) in r {
        if k == FRONT || k == REAR {
            continue;
        }
        let lv = get(l, k);
        let d = e.default_nonsticky(k);
        let mut ul = lv.is_some() && !e.tmem(k, lr) && d != Some(true);
        let mut ur = e.tmem(k, rf) || d == Some(false);
        if ul && ur {
            if lv == Some(0) {
                ul = false;
            } else if v == 0 {
                ur = false;
            }
        }
        if ul {
            out.push((k, lv.unwrap()));
            if e.tmem(k, lf) {
                f.push(k);
            }
            if e.tmem(k, lr) {
                rear.push(k);
            }
        } else if ur {
            out.push((k, v));
            if e.tmem(k, rf) {
                f.push(k);
            }
            if e.tmem(k, rr) {
                rear.push(k);
            }
        }
    }
    for &(k, v) in l {
        if k == FRONT || k == REAR || get(r, k).is_some() {
            continue;
        }
        let d = e.default_nonsticky(k);
        if !e.tmem(k, lr) && d != Some(true) {
            out.push((k, v));
            if e.tmem(k, lf) {
                f.push(k);
            }
        } else if e.tmem(k, rf) || d == Some(false) {
            f.push(k);
            if e.tmem(k, rr) {
                rear.push(k);
            }
        }
    }
    if !rear.is_empty() {
        out.insert(0, (REAR, e.list(rear)));
    }
    if !f.is_empty() && !e.category_front_sticky_t(&out) {
        out.insert(0, (FRONT, e.list(f)));
    }
    if eq(&out, l) {
        l.clone()
    } else if eq(&out, r) {
        r.clone()
    } else {
        out
    }
}
fn check(p: &Props, s: &PropSets<u64>, m: &[Plist]) {
    p.check_invariants(s);
    assert_eq!(p.len(), m.len());
    let mut runs = vec![];
    let mut a = 0;
    while a < m.len() {
        let mut b = a + 1;
        while b < m.len() && m[b] == m[a] {
            b += 1;
        }
        runs.push((CharPos(a), CharPos(b), m[a].clone()));
        a = b;
    }
    assert_eq!(
        p.runs(CharPos(0), CharPos(m.len()))
            .map(|(a, b, id)| (a, b, s.pairs(id).to_vec()))
            .collect::<Vec<_>>(),
        runs
    );
    for pos in 0..=m.len() {
        if pos < m.len() {
            assert_eq!(s.pairs(p.run_at(CharPos(pos)).2), m[pos]);
        }
        for key in [None, Some(2), Some(3), Some(FRONT), Some(REAR)] {
            let same = |a: &Plist, b: &Plist| {
                key.map_or_else(
                    || eq(a, b),
                    |k| get(a, k).unwrap_or(0) == get(b, k).unwrap_or(0),
                )
            };
            for limit in [m.len(), (pos + 3).min(m.len()), m.len() + 7] {
                let want = if pos == m.len() {
                    None
                } else {
                    (pos + 1..m.len().min(limit)).find(|&x| !same(&m[pos], &m[x]))
                };
                let got = match key {
                    None => p.next_change(s, CharPos(pos), CharPos(limit)),
                    Some(k) => {
                        p.next_single(CharPos(pos), CharPos(limit), |id| s.get(id, k).unwrap_or(0))
                    }
                };
                assert_eq!(got.map(|p| p.0), want);
            }
            for limit in [0, pos.saturating_sub(3), pos] {
                let want = if pos == 0 {
                    None
                } else {
                    (limit + 1..pos)
                        .rev()
                        .find(|&x| !same(&m[pos - 1], &m[x - 1]))
                };
                let got = match key {
                    None => p.prev_change(s, CharPos(pos), CharPos(limit)),
                    Some(k) => {
                        p.prev_single(CharPos(pos), CharPos(limit), |id| s.get(id, k).unwrap_or(0))
                    }
                };
                assert_eq!(got.map(|p| p.0), want);
            }
        }
    }
}
fn exercise(ops: &[(u8, usize, usize, u8)]) {
    let mut s = PropSets::new();
    let mut p = Props::filled(30, SetId::EMPTY);
    let mut m = vec![vec![]; 30];
    let e = Env::default();
    let me = Env::default();
    e.list(vec![2]);
    e.list(vec![3]);
    me.list(vec![2]);
    me.list(vec![3]);
    for &(kind, seed, n, val) in ops {
        let at = match seed % 5 {
            0 => 0,
            1 => m.len(),
            2 if !m.is_empty() => p.run_at(CharPos(seed % m.len())).0.0,
            _ => seed % (m.len() + 1),
        };
        let end = (at + 2 + n % 39).min(m.len());
        let k = [2, 3, FRONT, REAR][val as usize % 4];
        let v = [0, 1, 10, 1000, 1001][val as usize % 5];
        let op = if m.len() > 180 { 1 } else { kind };
        match op {
            0 | 6 => {
                let len = 2 + n % 19;
                let id = if op == 6 {
                    e.inherited(&p, &mut s, at)
                } else {
                    s.intern(&[(k, v)])
                };
                let expected = if op == 6 {
                    sticky(&m, at, &me)
                } else {
                    vec![(k, v)]
                };
                p.insert(CharPos(at), len, id);
                m.splice(at..at, vec![expected; len]);
            }
            1 => {
                p.delete(CharPos(at), CharPos(end));
                m.drain(at..end);
            }
            2..=4 => {
                let pairs = [(k, v)];
                let keys = [k];
                let id = s.intern(&pairs);
                let operation = match op {
                    2 => Op::Add(&pairs),
                    3 => Op::Set(id),
                    _ => Op::Remove(&keys),
                };
                let before = m.clone();
                for cell in &mut m[at..end] {
                    match op {
                        2 => add(cell, &[(k, v)]),
                        3 => *cell = vec![(k, v)],
                        _ => cell.retain(|x| x.0 != k),
                    }
                }
                let changed = before != m;
                assert_eq!(
                    p.would_change(&s, CharPos(at), CharPos(end), &operation),
                    changed
                );
                assert_eq!(
                    p.apply(&mut s, CharPos(at), CharPos(end), operation),
                    changed
                );
            }
            5 | 7 | 8 => {
                let mut src = Props::new();
                let mut sm = vec![];
                for i in 0..1 + n % 5 {
                    let ps = vec![([2, 3][i % 2], v)];
                    let id = s.intern(&ps);
                    let len = 2 + (n + i) % 5;
                    src.insert(CharPos(src.len()), len, id);
                    src.check_invariants(&s);
                    sm.extend(vec![ps; len]);
                }
                if op == 7 {
                    p.delete(CharPos(at), CharPos(end));
                    p.check_invariants(&s);
                    m.drain(at..end);
                }
                if op == 8 {
                    let id = e.inherited(&p, &mut s, at);
                    let inherited = sticky(&m, at, &me);
                    p.insert(CharPos(at), sm.len(), id);
                    p.check_invariants(&s);
                    p.merge_under(&mut s, CharPos(at), &src);
                    for ps in &mut sm {
                        let mut t = inherited.clone();
                        for &(k, v) in ps.iter() {
                            if get(&t, k).is_none() {
                                t.insert(0, (k, v));
                            }
                        }
                        *ps = t;
                    }
                } else {
                    p.insert_props(CharPos(at), &src);
                }
                m.splice(at..at, sm);
            }
            _ => {
                let copy = p.slice(CharPos(at), CharPos(end));
                let cm = m[at..end].to_vec();
                let dest = seed % (m.len() + 1);
                p.insert_props(CharPos(dest), &copy);
                m.splice(dest..dest, cm);
            }
        }
        check(&p, &s, &m);
        if seed % 19 == 0 {
            p.trace(&s, &mut |_| {});
            s.trace_young(&mut |_| {});
            s.sweep();
            p.trace(&s, &mut |_| {});
            s.sweep();
            check(&p, &s, &m);
        }
    }
}
proptest! {
 #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 8 } else { 256 }))]
 #[test] fn batch_edits(ops in prop::collection::vec((0u8..10,any::<usize>(),any::<usize>(),any::<u8>()),1..151)) {exercise(&ops);}
}
#[test]
// Tier0 keeps every per-position assertion; the release lane runs the full 20k stream.
fn fixed_seed_stream() {
    let mut seed = 0x329cab_u64;
    let iterations = if cfg!(debug_assertions) { 256 } else { 20000 };
    let ops: Vec<_> = (0..iterations)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (
                (seed % 10) as u8,
                (seed >> 8) as usize,
                (seed >> 32) as usize,
                (seed >> 50) as u8,
            )
        })
        .collect();
    exercise(&ops);
}
#[test]
fn epochs_and_cache_roots() {
    let mut s = PropSets::new();
    let id = s.intern(&[(2u64, 17)]);
    s.set_list(id, 91);
    let p = Props::filled(4, id);
    let mut seen = vec![];
    p.trace(&s, &mut |x| seen.push(x));
    p.trace(&s, &mut |x| seen.push(x));
    assert_eq!(seen, vec![2, 17, 91]);
    assert_eq!(s.sweep(), 0);
    p.trace(&s, &mut |_| {});
    assert_eq!(s.sweep(), 0);
    drop(p);
    assert_eq!(s.sweep(), 1);
    let young = s.intern(&[(3, 18)]);
    assert_eq!(s.sweep(), 0);
    assert_eq!(s.pairs(young), &[(3, 18)]);
    assert_eq!(s.sweep(), 1);
    assert_eq!(s.live(), 1);
}
