use romacs_chars::CharPos as P;
use romacs_props::{Op, PropSets, Props, SetId, Sticky, StickyEnv, StickyKey};
use std::{hint::black_box, time::Instant};
struct Env;
impl StickyEnv<u64> for Env {
    fn textget(&self, _: &[(u64, u64)], _: StickyKey) -> u64 {
        0
    }
    fn tmem(&self, _: u64, v: u64) -> bool {
        v != 0
    }
    fn is_nil(&self, v: u64) -> bool {
        v == 0
    }
    fn is_cons(&self, _: u64) -> bool {
        false
    }
    fn default_nonsticky(&self, _: u64) -> Option<bool> {
        None
    }
    fn category_front_sticky_t(&self, _: &[(u64, u64)]) -> bool {
        false
    }
    fn is_sticky_key(&self, _: u64) -> bool {
        false
    }
}
fn sample(name: &str, mut f: impl FnMut() -> (f64, usize)) {
    let mut times = vec![];
    let mut answer = 0;
    for _ in 0..11 {
        let (t, a) = f();
        times.push(t);
        answer = a;
        black_box(a);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{name}: {:.4} [{:.4}, {:.4}] ms answer={answer}",
        times[5], times[0], times[10]
    );
}
fn main() {
    sample("k-textprop-1e5", || {
        let t = Instant::now();
        let mut s = PropSets::new();
        let mut p = Props::filled(400000, SetId::EMPTY);
        for i in 0..100000 {
            p.apply(&mut s, P(4 * i), P(4 * i + 2), Op::Add(&[(2u64, 10)]));
        }
        let copy = p.slice(P(0), P(p.len()));
        let ms = t.elapsed().as_secs_f64() * 1000.;
        let count = copy.runs(P(0), P(copy.len())).count();
        assert_eq!(count, 200000);
        assert_eq!(copy.run_at(P(0)).1, P(2));
        (ms, count)
    });
    sample("k-chat-props", || {
        let t = Instant::now();
        let mut s = PropSets::new();
        let mut p = Props::filled(3, SetId::EMPTY);
        let mut at = 0;
        let mut expected = Vec::new();
        for msg in 0..2000 {
            let id = s.intern(&[(2u64, 10), (9, 1)]);
            let n = format!("\n* assistant {msg}\n").len();
            p.insert_props(P(at), &Props::filled(n, id));
            at += n;
            expected.push((n, id));
            let mut delta = 0;
            for d in 0..10 {
                let id = s.intern(&[(2, 11), (3, msg)]);
                let n = format!("delta {d} of message {msg} with `code` and words. ").len();
                p.insert_props(P(at), &Props::filled(n, id));
                at += n;
                delta += n;
            }
            expected.push((delta, s.intern(&[(2, 11), (3, msg)])));
        }
        expected.push((3, SetId::EMPTY));
        let mut n = 1;
        let mut q = P(0);
        while let Some(next) = p.next_change(&s, q, P(p.len())) {
            n += 1;
            q = next;
        }
        let ms = t.elapsed().as_secs_f64() * 1000.;
        assert_eq!(n, 4001);
        assert_eq!(p.len(), at + 3);
        assert_eq!(
            p.runs(P(0), P(p.len()))
                .map(|(a, b, id)| (b - a, id))
                .collect::<Vec<_>>(),
            expected
        );
        (ms, n)
    });
    let mut s = PropSets::new();
    let id = s.intern(&[(2u64, 10)]);
    let mut big = Props::new();
    for i in 0..1000000 {
        big.insert(P(big.len()), 8, if i % 2 == 0 { id } else { SetId::EMPTY });
    }
    sample("k-typing-early", || {
        let mut p = big.clone();
        let t = Instant::now();
        let mut seed = 17usize;
        for i in 0..10000 {
            p.insert(P(1), 1, SetId::EMPTY);
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let q = seed % 8000000;
            let (a, b, _) = p.run_at(P(q));
            assert!(a.0 <= q && b.0 > q);
            let original = q.saturating_sub(i + 1);
            let expected = if q == 0 || (q > i + 1 && (original / 8).is_multiple_of(2)) {
                id
            } else {
                SetId::EMPTY
            };
            assert_eq!(p.run_at(P(q)).2, expected);
            assert_eq!(p.len(), 8000000 + i + 1);
        }
        (t.elapsed().as_secs_f64() * 1000., p.len())
    });
    sample("k-sticky-typing", || {
        let mut p = Props::filled(2, id);
        let t = Instant::now();
        for _ in 0..100000 {
            let Sticky::Set(fill) = p.sticky_at(&mut s, P(p.len()), &Env) else {
                panic!()
            };
            p.insert(P(p.len()), 1, fill);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.;
        assert_eq!(p.runs(P(0), P(p.len())).count(), 1);
        (ms, p.len())
    });
    sample("k-next-single", || {
        let t = Instant::now();
        let mut q = P(0);
        let mut n = 0;
        while let Some(next) = big.next_single(q, P(big.len()), |id| s.get(id, 2).unwrap_or(0)) {
            n += 1;
            q = next;
        }
        assert_eq!(n, 999999);
        (t.elapsed().as_secs_f64() * 1000., n)
    });
    let live = big.slice(P(0), P(1600000));
    sample("k-gc-live", || {
        let t = Instant::now();
        live.trace(&s, &mut |x| {
            black_box(x);
        });
        s.sweep();
        (t.elapsed().as_secs_f64() * 1000., s.live())
    });
    sample("k-gc-dead", || {
        let mut sets = PropSets::new();
        for i in 0..100000 {
            sets.intern(&[(2u64, i)]);
        }
        sets.sweep();
        let t = Instant::now();
        assert_eq!(sets.sweep(), 100000);
        (t.elapsed().as_secs_f64() * 1000., sets.live())
    });
}
