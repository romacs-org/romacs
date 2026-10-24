use super::*;
use crate::{
    HeapConfig, Trace,
    foreign_tests::{Fixture, install_fixture, place_fixture},
    raw::{LegacyWeak, Marker},
    spawn_isolate,
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn undo_owner_symbols_and_static_leaves_survive_collection() {
    struct Undo(RawValue);
    impl Trace for Undo {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn undo_heads(&mut self, visit: &mut dyn FnMut(&mut RawValue)) {
            visit(&mut self.0);
        }
    }
    static LEAF: u64 = 0;
    let mut h = unsafe {
        Heap::for_legacy_owner(HeapConfig {
            external_symbols: true,
            ..HeapConfig::default()
        })
    };
    let apply = Value::word(100_000 << 3);
    let set_buffer_multibyte = Value::word(100_001 << 3);
    let leaf = Value::word(std::ptr::addr_of!(LEAF) as u64 | 5);
    unsafe { h.add_static_range(std::ptr::addr_of!(LEAF) as usize, 8) };
    let args = h.cons(crate::sym::T, h.nil()).unwrap();
    let call = h.cons(set_buffer_multibyte, args).unwrap();
    let entry = h.cons(apply, call).unwrap();
    let leaf_entry = h.cons(leaf, h.fixnum(1).unwrap()).unwrap();
    let tail = h.cons(leaf_entry, h.nil()).unwrap();
    let head = h.cons(entry, tail).unwrap();
    let expected = RawValue::from_value(head);
    let mut undo = Undo(expected);
    for _ in 0..2 {
        h.collect(&mut undo);
        assert_eq!(undo.0, expected);
        let head = h.as_cons(unsafe { undo.0.value(&h) }).unwrap();
        let entry = h.as_cons(head.car()).unwrap();
        assert_eq!(entry.car(), apply);
        let call = h.as_cons(entry.cdr()).unwrap();
        assert_eq!(call.car(), set_buffer_multibyte);
        assert_eq!(h.as_cons(call.cdr()).unwrap().car(), crate::sym::T);
        let tail = h.as_cons(head.cdr()).unwrap();
        assert_eq!(h.as_cons(tail.car()).unwrap().car(), leaf);
        assert!(tail.cdr().is_nil());
    }
}

#[test]
#[should_panic(expected = "Dangling")]
fn stale_undo_head_on_released_chunk_panics() {
    struct Undo {
        head: RawValue,
    }
    impl Trace for Undo {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn undo_heads(&mut self, visit: &mut dyn FnMut(&mut RawValue)) {
            visit(&mut self.head);
        }
    }
    let result = spawn_isolate(HeapConfig::default(), |mut h| {
        let head = RawValue::from_value(h.cons(h.nil(), h.nil()).unwrap());
        h.collect(&mut ());
        h.collect(&mut ());
        assert_eq!(h.allocation_stats().mapped_bytes, 0);
        h.collect(&mut Undo { head });
    })
    .join();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn stale_undo_links_panic_before_dereference() {
    for edge in 0..3 {
        let result = spawn_isolate(HeapConfig::default(), move |mut h| {
            let entry = h.cons(h.nil(), h.nil()).unwrap();
            let head = h.cons(entry, h.nil()).unwrap();
            let root = Root::new(&h, head, "undo head");
            let stale = RawValue::from_value(h.cons(h.nil(), h.nil()).unwrap());
            h.collect(&mut ());
            h.collect(&mut ());
            let cell = h.as_cons(root.get(&h)).unwrap();
            match edge {
                0 => cell.set_cdr(Value::word(stale.0)),
                1 => cell.set_car(Value::word(stale.0)),
                _ => h.as_cons(cell.car()).unwrap().set_car(Value::word(stale.0)),
            }
            let mut head = RawValue::from_value(root.get(&h));
            h.compact_undo(&mut head);
        })
        .join();
        let panic = result.expect_err("stale undo edge must panic");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            message.contains("Dangling") || message.contains("foreign/dead value"),
            "edge {edge}: {message}"
        );
    }
}

struct Legacy {
    policy: Weakness,
    pairs: Rc<RefCell<Vec<(RawValue, RawValue)>>>,
}
// SAFETY: complete weak enumeration; retain preserves order/count; no heap access.
unsafe impl LegacyWeak for Legacy {
    fn weakness(&self) -> Weakness {
        self.policy
    }
    fn entries(&self, f: &mut dyn FnMut(RawValue, RawValue)) {
        for &(k, v) in self.pairs.borrow().iter() {
            f(k, v);
        }
    }
    fn remove_dead(&mut self, keep: &dyn Fn(RawValue, RawValue) -> bool) -> u32 {
        let mut pairs = self.pairs.borrow_mut();
        let before = pairs.len();
        pairs.retain(|&(k, v)| keep(k, v));
        pairs.shrink_to_fit();
        (before - pairs.len()) as u32
    }
}
// SAFETY: no strong entry edges; all weak pairs exposed; no outstanding borrow at GC.
impl Fixture for Legacy {
    fn trace(&self, _: &mut Marker<'_>) {}
    fn weak(&mut self) -> Option<&mut dyn LegacyWeak> {
        Some(self)
    }
    fn bytes(&self) -> usize {
        self.pairs.borrow().capacity() * 16
    }
}
fn legacy(h: &Heap, policy: Weakness, pairs: Rc<RefCell<Vec<(RawValue, RawValue)>>>) -> Value<'_> {
    // SAFETY: fixtures only put this heap's words in pairs and release interior borrows.
    place_fixture(
        h,
        Legacy { policy, pairs },
        crate::raw::ForeignFlags::WEAK_TABLE,
    )
}
#[test]
fn weakness_matrix_native_and_legacy() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        install_fixture::<Legacy>(&mut h);
        for policy in [
            Weakness::Key,
            Weakness::Value,
            Weakness::KeyOrValue,
            Weakness::KeyAndValue,
        ] {
            for mask in 0..4 {
                let k = h.float(1.0).unwrap();
                let v = h.float(2.0).unwrap();
                let wk = WeakRef::new(&h, k);
                let wv = WeakRef::new(&h, v);
                let kr = (mask & 1 != 0).then(|| Root::new(&h, k, "key"));
                let vr = (mask & 2 != 0).then(|| Root::new(&h, v, "value"));
                let n = h.make_weak_pairs(policy, 1).unwrap();
                h.as_weak_pairs(n).unwrap().set(0, k, v);
                let n = Root::new(&h, n, "native");
                let pairs = Rc::new(RefCell::new(vec![(
                    RawValue::from_value(k),
                    RawValue::from_value(v),
                )]));
                let l = Root::new(&h, legacy(&h, policy, pairs.clone()), "legacy");
                let kept = match policy {
                    Weakness::Key => mask & 1 != 0,
                    Weakness::Value => mask & 2 != 0,
                    Weakness::KeyOrValue => mask != 0,
                    Weakness::KeyAndValue => mask == 3,
                };
                h.collect(&mut ());
                assert_eq!(
                    h.as_weak_pairs(n.get(&h)).unwrap().len(),
                    usize::from(kept),
                    "{policy:?}/{mask}"
                );
                assert_eq!(pairs.borrow().len(), usize::from(kept));
                assert_eq!(
                    h.accounted_bytes(),
                    h.allocation_stats().live_bytes + pairs.borrow().capacity() * 16,
                    "weak removal must reaccount released legacy storage"
                );
                assert_eq!(wk.upgrade(&h).is_some(), kept || kr.is_some());
                assert_eq!(wv.upgrade(&h).is_some(), kept || vr.is_some());
                drop((kr, vr, n, l));
                h.collect(&mut ());
                assert!(wk.upgrade(&h).is_none() && wv.upgrade(&h).is_none());
            }
        }
    })
    .join()
    .unwrap();
}
#[test]
fn mixed_ephemeron_fixpoint_discovers_new_weak_container() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        install_fixture::<Legacy>(&mut h);
        let a = h.cons(h.nil(), h.nil()).unwrap();
        let b = h.cons(h.nil(), h.nil()).unwrap();
        let c = h.cons(h.nil(), h.nil()).unwrap();
        let wc = WeakRef::new(&h, c);
        let late = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(late).unwrap().set(0, b, c);
        let pairs = Rc::new(RefCell::new(vec![(
            RawValue::from_value(a),
            RawValue::from_value(b),
        )]));
        let l = Root::new(&h, legacy(&h, Weakness::Key, pairs.clone()), "legacy");
        let n = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(n).unwrap().set(0, a, late);
        let n = Root::new(&h, n, "native");
        let a = Root::new(&h, a, "seed");
        h.collect(&mut ());
        assert!(wc.upgrade(&h).is_some());
        assert_eq!(pairs.borrow().len(), 1);
        drop(a);
        h.collect(&mut ());
        assert!(wc.upgrade(&h).is_none());
        assert!(pairs.borrow().is_empty());
        assert!(h.as_weak_pairs(n.get(&h)).unwrap().is_empty());
        drop(l);
    })
    .join()
    .unwrap();
}
#[test]
fn weak_ids_send_foreign_heap_and_retired_entry() {
    let foreign = spawn_isolate(HeapConfig::default(), |h| WeakRef::new(&h, h.nil()).id())
        .join()
        .unwrap();
    spawn_isolate(HeapConfig::default(), move |h| {
        let weak = WeakRef::new(&h, h.nil());
        let id = weak.id();
        assert!(foreign.resolve(&h).is_none());
        assert_eq!(id.resolve(&h), Some(h.nil()));
        assert!(weak.is(&h, h.nil()));
        drop(weak);
        assert!(id.resolve(&h).is_none());
        let fresh = WeakRef::new(&h, h.nil());
        assert_ne!(id, fresh.id());
        assert!(id.resolve(&h).is_none());
    })
    .join()
    .unwrap();
}
#[test]
fn weak_refs_dead_cell_live_float_neighbor_and_reuse() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        let dead = h.float(1.0).unwrap();
        let dead_word = dead.0;
        let weak = WeakRef::new(&h, dead);
        let live = h.float(2.0).unwrap();
        let neighbor = WeakRef::new(&h, live);
        let live = Root::new(&h, live, "neighbor");
        h.collect(&mut ());
        assert!(weak.upgrade(&h).is_none());
        assert!(neighbor.upgrade(&h).is_some());
        let fresh = h.float(3.0).unwrap();
        assert_eq!(fresh.0, dead_word, "exercise same-slot reuse");
        let fresh = WeakRef::new(&h, fresh);
        assert_ne!(weak.id(), fresh.id());
        assert!(weak.upgrade(&h).is_none());
        assert_eq!(h.as_float(fresh.upgrade(&h).unwrap()), Some(3.0));
        drop(fresh);
        assert_eq!(h.tables.weak_refs.borrow().len(), 1);
        drop(live);
    })
    .join()
    .unwrap();
}
#[test]
fn finalizers_two_pass_once_and_recursive_collection() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        let function = h
            .make_vectorlike(PvecType::NormalVector, &[h.nil()], &[])
            .unwrap();
        let first = h.make_finalizer(function).unwrap();
        let second = h.make_finalizer(first).unwrap();
        // first's function closes over second: neither can prevent the other being doomed.
        h.as_vectorlike(function).unwrap().set(0, second);
        let wf = WeakRef::new(&h, function);
        let wa = WeakRef::new(&h, first);
        let wb = WeakRef::new(&h, second);
        h.collect(&mut ());
        assert_eq!(h.gc.borrow().doomed.len(), 2);
        assert!(wf.upgrade(&h).is_some() && wa.upgrade(&h).is_some() && wb.upgrade(&h).is_some());
        let mut calls = 0;
        assert_eq!(
            h.run_finalizers(|heap, root| {
                calls += 1;
                heap.collect(&mut ());
                assert!(heap.as_vectorlike(root.get(heap)).is_some());
            }),
            2
        );
        assert_eq!(calls, 2);
        h.collect(&mut ());
        assert_eq!(h.run_finalizers(|_, _| panic!("twice")), 0);
        assert!(wf.upgrade(&h).is_none() && wa.upgrade(&h).is_none() && wb.upgrade(&h).is_none());
    })
    .join()
    .unwrap();
}
#[test]
fn finalizer_fifo_and_panic_does_not_repeat_callback() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        for i in 0..3 {
            h.make_finalizer(
                h.make_vectorlike(PvecType::NormalVector, &[h.fixnum(i).unwrap()], &[])
                    .unwrap(),
            )
            .unwrap();
        }
        h.collect(&mut ());
        let mut seen = Vec::new();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            h.run_finalizers(|heap, root| {
                seen.push(
                    heap.as_vectorlike(root.get(heap))
                        .unwrap()
                        .get(0)
                        .unwrap()
                        .as_fixnum()
                        .unwrap(),
                );
                panic!("callback panic");
            })
        }));
        assert!(panic.is_err());
        assert_eq!(seen, vec![0]);
        h.collect(&mut ());
        assert_eq!(
            h.run_finalizers(|heap, root| seen.push(
                heap.as_vectorlike(root.get(heap))
                    .unwrap()
                    .get(0)
                    .unwrap()
                    .as_fixnum()
                    .unwrap()
            )),
            2
        );
        assert_eq!(seen, vec![0, 1, 2]);
        h.collect(&mut ());
        assert_eq!(h.run_finalizers(|_, _| panic!("repeat")), 0);
    })
    .join()
    .unwrap();
}
#[test]
fn panicking_owner_hook_poisoned_before_retry() {
    struct PanicHook;
    impl Trace for PanicHook {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn unchain_markers(&mut self, _: &dyn Fn(RawValue) -> bool) {
            panic!("owner hook panic");
        }
    }
    spawn_isolate(HeapConfig::default(), |mut h| {
        let root = Root::new(&h, h.cons(h.nil(), h.nil()).unwrap(), "root");
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.collect(&mut PanicHook)))
                .is_err()
        );
        let new = h.float(3.0).unwrap();
        h.as_cons(root.get(&h)).unwrap().set_car(new);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.collect(&mut ()))).is_err()
        );
        assert_eq!(
            h.as_float(h.as_cons(root.get(&h)).unwrap().car()),
            Some(3.0)
        );
        assert_eq!(h.epoch(), 0);
    })
    .join()
    .unwrap();
}
#[test]
fn undo_heads_are_marked_in_owner_order() {
    struct Heads([RawValue; 2]);
    impl Trace for Heads {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn undo_heads(&mut self, f: &mut dyn FnMut(&mut RawValue)) {
            for head in &mut self.0 {
                f(head);
            }
        }
    }
    spawn_isolate(HeapConfig::default(), |mut h| {
        let m = h.make_vectorlike(PvecType::Marker, &[], &[]).unwrap();
        let first = h.cons(m, h.nil()).unwrap();
        let entry = h.cons(m, h.fixnum(1).unwrap()).unwrap();
        let second = h.cons(entry, h.nil()).unwrap();
        let mut heads = Heads([RawValue::from_value(first), RawValue::from_value(second)]);
        h.collect(&mut heads);
        assert_eq!(
            h.as_cons(Value::word(heads.0[1].0)).unwrap().car().kind(),
            Kind::Cons
        );
    })
    .join()
    .unwrap();
}
#[test]
fn reachable_finalizer_waits_and_doomed_graph_seeds_weak() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        let key = h.cons(h.nil(), h.nil()).unwrap();
        let value = h.float(4.0).unwrap();
        let weak = WeakRef::new(&h, value);
        let table = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(table).unwrap().set(0, key, value);
        let table = Root::new(&h, table, "table");
        let f = Root::new(&h, h.make_finalizer(key).unwrap(), "finalizer");
        h.collect(&mut ());
        assert_eq!(h.run_finalizers(|_, _| panic!("reachable")), 0);
        drop(f);
        h.collect(&mut ());
        assert!(weak.upgrade(&h).is_some());
        assert_eq!(h.run_finalizers(|_, _| {}), 1);
        h.collect(&mut ());
        assert!(weak.upgrade(&h).is_none());
        assert!(h.as_weak_pairs(table.get(&h)).unwrap().is_empty());
    })
    .join()
    .unwrap();
}
struct World {
    heap: Heap,
    undo: RawValue,
    chain: Vec<RawValue>,
    detached: usize,
}
struct GcRoots<'a> {
    undo: &'a mut RawValue,
    chain: &'a mut Vec<RawValue>,
    detached: &'a mut usize,
}
impl World {
    fn gc_parts(&mut self) -> (&mut Heap, GcRoots<'_>) {
        (
            &mut self.heap,
            GcRoots {
                undo: &mut self.undo,
                chain: &mut self.chain,
                detached: &mut self.detached,
            },
        )
    }
}
impl Trace for GcRoots<'_> {
    fn trace(&self, _: &mut Marker<'_>) {}
    fn undo_heads(&mut self, f: &mut dyn FnMut(&mut RawValue)) {
        f(self.undo);
    }
    fn unchain_markers(&mut self, marked: &dyn Fn(RawValue) -> bool) {
        self.chain.retain(|v| {
            if marked(*v) {
                true
            } else {
                *self.detached += 1;
                false
            }
        });
    }
}
fn undo_case(h: Heap, mask: u8, nonfix: bool, dotted: bool) -> Vec<i64> {
    let mut undo = if dotted {
        h.fixnum(99).unwrap()
    } else {
        h.nil()
    };
    let mut roots = Vec::new();
    let mut chain = Vec::new();
    for i in (0..4).rev() {
        let m = h.make_vectorlike(PvecType::Marker, &[], &[]).unwrap();
        chain.push(RawValue::from_value(m));
        if mask & (1 << i) != 0 {
            roots.push(Root::new(&h, m, "live marker"));
        }
        let data = if nonfix {
            h.cons(h.fixnum(i).unwrap(), h.nil()).unwrap()
        } else {
            h.fixnum(i).unwrap()
        };
        let entry = h.cons(m, data).unwrap();
        undo = h.cons(entry, undo).unwrap();
    }
    let raw = RawValue::from_value(undo);
    let mut world = World {
        heap: h,
        undo: raw,
        chain,
        detached: 0,
    };
    let (heap, mut view) = world.gc_parts();
    heap.collect(&mut view);
    assert_eq!(world.detached, 4 - mask.count_ones() as usize);
    let h = &world.heap;
    let mut tail = Value::word(world.undo.0);
    let mut out = Vec::new();
    while let Some(c) = h.as_cons(tail) {
        let data = h.as_cons(c.car()).unwrap().cdr();
        out.push(
            if nonfix {
                h.as_cons(data).unwrap().car()
            } else {
                data
            }
            .as_fixnum()
            .unwrap(),
        );
        tail = c.cdr();
    }
    assert_eq!(tail.as_fixnum(), dotted.then_some(99));
    assert_eq!(
        out,
        (0..4)
            .filter(|i| mask & (1 << i) != 0)
            .map(i64::from)
            .collect::<Vec<_>>()
    );
    drop(roots);
    out
}
#[test]
fn undo_writeback_head_middle_adjacent_and_dotted_nonfix_data() {
    for mask in [0, 1, 2, 5, 6, 8, 15] {
        for nonfix in [false, true] {
            for dotted in [false, true] {
                spawn_isolate(HeapConfig::default(), move |h| {
                    undo_case(h, mask, nonfix, dotted)
                })
                .join()
                .unwrap();
            }
        }
    }
}
#[test]
fn gnu_undo_and_weakness_differential() {
    let output = std::process::Command::new("emacs")
        .args([
            "-Q",
            "--batch",
            "-l",
            concat!(env!("CARGO_MANIFEST_DIR"), "/bench/special-gnu.el"),
        ])
        .output()
        .expect("GNU oracle must be installed");
    assert!(
        output.status.success(),
        "GNU failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual = String::from_utf8(output.stdout).unwrap();
    let mut expected = String::new();
    for mask in [0, 1, 2, 5, 6, 8, 15] {
        for nonfix in [false, true] {
            for dotted in [false, true] {
                let out = spawn_isolate(HeapConfig::default(), move |h| {
                    undo_case(h, mask, nonfix, dotted)
                })
                .join()
                .unwrap();
                expected.push_str(&format!(
                    "undo {mask} {} {} {}\n",
                    u8::from(nonfix),
                    u8::from(dotted),
                    out.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
                ));
            }
        }
    }
    for (name, policy) in [
        ("key", Weakness::Key),
        ("value", Weakness::Value),
        ("key-or-value", Weakness::KeyOrValue),
        ("key-and-value", Weakness::KeyAndValue),
    ] {
        for mask in 0..4 {
            let len = spawn_isolate(HeapConfig::default(), move |mut h| {
                install_fixture::<Legacy>(&mut h);
                let k = h.cons(h.nil(), h.nil()).unwrap();
                let v = h.cons(h.nil(), h.nil()).unwrap();
                let kr = (mask & 1 != 0).then(|| Root::new(&h, k, "key"));
                let vr = (mask & 2 != 0).then(|| Root::new(&h, v, "value"));
                let n = h.make_weak_pairs(policy, 1).unwrap();
                h.as_weak_pairs(n).unwrap().set(0, k, v);
                let n = Root::new(&h, n, "native");
                let pairs = Rc::new(RefCell::new(vec![(
                    RawValue::from_value(k),
                    RawValue::from_value(v),
                )]));
                let l = Root::new(&h, legacy(&h, policy, pairs.clone()), "legacy");
                h.collect(&mut ());
                let len = h.as_weak_pairs(n.get(&h)).unwrap().len();
                assert_eq!(len, pairs.borrow().len());
                drop((kr, vr, l));
                len
            })
            .join()
            .unwrap();
            expected.push_str(&format!("weak {name} {mask} {len}\n"));
        }
    }
    let special = spawn_isolate(HeapConfig::default(), |mut h| {
        install_fixture::<Legacy>(&mut h);
        let a = h.cons(h.nil(), h.nil()).unwrap();
        let b = h.cons(h.nil(), h.nil()).unwrap();
        let c = h.cons(h.nil(), h.nil()).unwrap();
        let late = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(late).unwrap().set(0, b, c);
        let pairs = Rc::new(RefCell::new(vec![(
            RawValue::from_value(a),
            RawValue::from_value(b),
        )]));
        let l = Root::new(&h, legacy(&h, Weakness::Key, pairs.clone()), "legacy");
        let n = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(n).unwrap().set(0, a, late);
        let n = Root::new(&h, n, "native");
        let a = Root::new(&h, a, "seed");
        h.collect(&mut ());
        let native = h.as_weak_pairs(n.get(&h)).unwrap();
        let mut out = format!(
            "chain live {} {} {}\n",
            native.len(),
            pairs.borrow().len(),
            h.as_weak_pairs(native.get(0).unwrap().1).unwrap().len()
        );
        drop(a);
        h.collect(&mut ());
        out.push_str(&format!(
            "chain dead {} {}\n",
            h.as_weak_pairs(n.get(&h)).unwrap().len(),
            pairs.borrow().len()
        ));
        drop((n, l));
        let function = h
            .make_vectorlike(PvecType::NormalVector, &[h.nil()], &[])
            .unwrap();
        let first = h.make_finalizer(function).unwrap();
        let second = h.make_finalizer(first).unwrap();
        h.as_vectorlike(function).unwrap().set(0, second);
        h.collect(&mut ());
        out.push_str(&format!(
            "finalizers first {}\n",
            h.run_finalizers(|_, _| {})
        ));
        h.collect(&mut ());
        out.push_str(&format!(
            "finalizers again {}\n",
            h.run_finalizers(|_, _| panic!("repeat"))
        ));
        out
    })
    .join()
    .unwrap();
    expected.push_str(&special);
    assert_eq!(
        actual.lines().count(),
        48,
        "28 undo + 16 weak + 2 chain + 2 finalizer rows required"
    );
    assert_eq!(actual, expected);
}
#[cfg(feature = "verify")]
#[test]
fn verifier_doomed_graph_before_any_drop() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        struct Count(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl crate::Payload for Count {
            fn bytes(&self) -> usize {
                0
            }
        }
        impl Drop for Count {
            fn drop(&mut self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p = h
            .make_payload(PvecType::Other, &[], Box::new(Count(drops.clone())))
            .unwrap();
        h.make_finalizer(p).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.collect(&mut ())));
        if let Err(panic) = result {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("");
            assert!(
                message.contains("Lost"),
                "must refuse before sweep: {message}"
            );
            assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert_eq!(h.allocation_stats().released_pages, 0);
            std::panic::resume_unwind(panic);
        }
        assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 0);
        h.run_finalizers(|_, _| {});
        h.collect(&mut ());
        assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 1);
    })
    .join()
    .unwrap();
}
