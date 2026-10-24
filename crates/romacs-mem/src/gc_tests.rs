use super::*;
use crate::{Payload, PvecType, Root, ValueStack};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct CountDrop(Arc<AtomicUsize>);
impl Payload for CountDrop {
    fn bytes(&self) -> usize {
        1024
    }
}
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn immutable_string_accounting_needs_no_size_tree() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    let baseline = heap.accounted_bytes();
    for _ in 0..3 {
        let kept = Root::new(&heap, heap.make_string(&[b'x'; 16], false).unwrap(), "kept");
        heap.make_string(&[b'y'; 32], false).unwrap();
        heap.make_string(&[], false).unwrap();
        let debt = heap.allocation_debt();
        assert_eq!(heap.accounted_bytes(), baseline + 3 * 48 + 16 + 32);
        heap.account_external(kept.get(&heap));
        assert_eq!(heap.allocation_debt(), debt);
        assert!(
            heap.gc.borrow().external.is_empty(),
            "immutable strings need no size index"
        );
        heap.collect(&mut ());
        assert_eq!(heap.accounted_bytes(), baseline + 48 + 16);
        assert_eq!(heap.allocation_debt(), 0);
        assert_eq!(
            heap.as_string(kept.get(&heap)).unwrap().bytes(),
            &[b'x'; 16]
        );
        drop(kept);
        heap.collect(&mut ());
        assert_eq!(heap.accounted_bytes(), baseline);
        assert_eq!(heap.gc.borrow().external_total, 0);
    }
}

#[test]
fn panicking_payload_is_retired_before_drop() {
    const CHILD: &str = "ROMACS_MEM_DROP_PANIC_CHILD";
    const TEST: &str = "gc::tests::panicking_payload_is_retired_before_drop";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--color", "never", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains(&format!("test {TEST} ... ok")));
        assert!(stdout.contains("test result: ok. 1 passed; 0 failed;"));
        return;
    }
    struct PanicDrop(Arc<AtomicUsize>);
    impl Payload for PanicDrop {
        fn bytes(&self) -> usize {
            1024
        }
    }
    impl Drop for PanicDrop {
        fn drop(&mut self) {
            let count = self.0.fetch_add(1, Ordering::Relaxed) + 1;
            eprintln!("payload drop #{count}");
            if count == 1 {
                panic!("payload destructor panic");
            }
        }
    }
    let mut heap = Heap::create(crate::HeapConfig::default());
    let drops = Arc::new(AtomicUsize::new(0));
    heap.make_payload(
        PvecType::NormalVector,
        &[],
        Box::new(PanicDrop(drops.clone())),
    )
    .unwrap();
    let remaining_drops = Arc::new(AtomicUsize::new(0));
    heap.make_payload(
        PvecType::NormalVector,
        &[],
        Box::new(CountDrop(remaining_drops.clone())),
    )
    .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| heap.collect(&mut ())))
        .expect_err("payload panic must propagate");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"payload destructor panic")
    );
    assert!(heap.resources.borrow().is_empty());
    assert_eq!(heap.gc.borrow().external_total, 0);
    let retry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| heap.collect(&mut ())))
        .expect_err("interrupted sweep must poison collection");
    assert_eq!(
        retry.downcast_ref::<&str>(),
        Some(&"collection poisoned by interrupted sweep")
    );
    assert_eq!(heap.epoch(), 0);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    // Teardown must be safe even after the sweep was interrupted.
    drop(heap);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(remaining_drops.load(Ordering::Relaxed), 0);
}

#[test]
fn strong_cycle_and_eager_drops() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    let drops = Arc::new(AtomicUsize::new(0));
    let payload = heap
        .make_payload(
            PvecType::NormalVector,
            &[],
            Box::new(CountDrop(drops.clone())),
        )
        .unwrap();
    let vector = heap
        .make_vectorlike(PvecType::NormalVector, &[payload], &[])
        .unwrap();
    let cons = heap.cons(vector, heap.nil()).unwrap();
    heap.as_cons(cons).unwrap().set_cdr(cons);
    let root = Root::new(&heap, cons, "cycle");
    let dead = heap.make_string(b"dead", false).unwrap();
    let _ = dead;
    let r = heap.collect(&mut ());
    assert_eq!(r.freed_objects, 1);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert!(heap.as_cons(root.get(&heap)).is_some());
    drop(root);
    let r = heap.collect(&mut ());
    assert_eq!(r.freed_objects, 3);
    assert_eq!(r.dropped_payloads, 1);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    heap.collect(&mut ());
    assert!(heap.allocation_stats().released_pages > 0);
}
#[test]
fn stack_trace_and_truncate() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    let mut stack = ValueStack::new(&heap);
    stack.push(&heap, heap.make_string(b"stack", false).unwrap());
    assert_eq!(heap.collect(&mut ()).freed_objects, 0);
    stack.truncate(0);
    assert_eq!(heap.collect(&mut ()).freed_objects, 1);
}
struct Buffers {
    undo: Rc<RefCell<RawValue>>,
    markers: Vec<RawValue>,
    strong: RawValue,
}
struct TestWorld {
    heap: Heap,
    roots: Vec<Root>,
    buffers: Buffers,
}
struct GcRoots<'a> {
    roots: &'a [Root],
    buffers: &'a mut Buffers,
}
impl Trace for GcRoots<'_> {
    fn trace(&self, m: &mut Marker<'_>) {
        let _ = (
            self.roots.len(),
            self.buffers.undo.borrow(),
            self.buffers.markers.len(),
        );
        m.mark(self.buffers.strong);
    }
}
impl TestWorld {
    fn gc_parts(&mut self) -> (&mut Heap, GcRoots<'_>) {
        let Self {
            heap,
            roots,
            buffers,
        } = self;
        (heap, GcRoots { roots, buffers })
    }
}
#[test]
fn owner_split_borrow_collects() {
    let heap = Heap::create(crate::HeapConfig::default());
    let strong = RawValue::from_value(heap.make_string(b"owner", false).unwrap());
    let special = RawValue::from_value(heap.cons(heap.nil(), heap.nil()).unwrap());
    let mut world = TestWorld {
        heap,
        roots: vec![],
        buffers: Buffers {
            strong,
            undo: Rc::new(RefCell::new(special)),
            markers: vec![special],
        },
    };
    let (heap, mut roots) = world.gc_parts();
    assert_eq!(heap.collect(&mut roots).freed_objects, 1);
    assert_eq!(
        heap.accounted_bytes(),
        crate::sym::BUILTIN_COUNT as usize * 48 + 48 + 5
    );
}
#[test]
fn symbols_plateau_and_interim_sticky_pin() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    heap.set_symbol_mode(SymbolMode::Interim);
    let s = heap
        .make_symbol(heap.make_string(b"pin", false).unwrap())
        .unwrap();
    heap.as_symbol(s).unwrap().set_value(heap.nil());
    assert_eq!(heap.collect(&mut ()).freed_objects, 0);
    let live = heap.accounted_bytes();
    for _ in 0..100 {
        for _ in 0..30 {
            heap.make_symbol(heap.make_string(b"temp", false).unwrap())
                .unwrap();
        }
        assert_eq!(heap.collect(&mut ()).freed_objects, 60);
        assert_eq!(heap.accounted_bytes(), live);
    }
    heap.set_symbol_mode(SymbolMode::Full);
    assert_eq!(heap.collect(&mut ()).freed_objects, 2);
}
#[test]
fn threshold_no_idle_reclaims_large_graph_and_overshoot() {
    let mut heap = Heap::create(crate::HeapConfig {
        gc_cons_threshold: 1000,
        gc_cons_percentage: 0.1,
        ..Default::default()
    });
    heap.collect(&mut ());
    let l = heap.accounted_bytes();
    heap.make_vectorlike(PvecType::NormalVector, &[], &vec![0; 18000])
        .unwrap();
    assert_eq!(heap.safepoint(), GcEvent::WantCycle);
    let d = heap.allocation_debt();
    let t = heap.threshold();
    let r = heap.collect(&mut ());
    assert_eq!(r.freed_objects, 1);
    assert_eq!(r.record.trigger, Trigger::Threshold);
    assert_eq!(r.record.overshoot, d - t);
    assert!(r.record.overshoot > 0);
    assert_eq!(r.record.accounted_before, l + d);
    assert_eq!(r.record.accounted_after, l);
    heap.make_string(&vec![0; 500], false).unwrap();
    assert_eq!(
        heap.gc_event(false, Duration::from_millis(999)),
        GcEvent::None
    );
    assert_eq!(
        heap.gc_event(false, Duration::from_secs(1)),
        GcEvent::WantCycle
    );
    assert_eq!(heap.collect(&mut ()).record.trigger, Trigger::Idle);
    assert_eq!(heap.gc_event(true, Duration::ZERO), GcEvent::WantCycle);
    assert_eq!(heap.collect(&mut ()).record.trigger, Trigger::Explicit);
}
#[test]
fn model_stress_reclaims_exactly_and_planted_leak_fails() {
    fn run(leak: bool) -> bool {
        let mut heap = Heap::create(crate::HeapConfig {
            gc_cons_threshold: 256,
            gc_cons_percentage: 0.0,
            ..Default::default()
        });
        let baseline = heap.accounted_bytes();
        let mut leaked = Vec::new();
        let mut seed = 17u64;
        for cycle in 0..200 {
            let mut roots = Vec::new();
            let mut dead = 0;
            for _ in 0..40 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let s = heap
                    .make_string(&vec![0; (seed as usize % 100) + 1], false)
                    .unwrap();
                if seed & 4 != 0 {
                    roots.push(Root::new(&heap, s, "model"));
                } else {
                    dead += 1;
                }
                let d = heap.allocation_debt();
                assert!(
                    heap.accounted_bytes()
                        <= heap.gc.borrow().last_live.max(baseline)
                            + heap.threshold()
                            + d.saturating_sub(heap.threshold())
                );
            }
            if leak {
                leaked.push(roots.pop().unwrap());
            }
            let r = heap.collect(&mut ());
            assert_eq!(r.freed_objects, dead);
            assert!(dead > 0);
            drop(roots);
            heap.collect(&mut ());
            if !leak {
                assert_eq!(heap.accounted_bytes(), baseline);
            }
            assert_eq!(heap.gc_records().len(), 2 * (cycle + 1));
        }
        let pass = heap.accounted_bytes() <= baseline + baseline / 10;
        drop(leaked);
        pass
    }
    assert!(run(false));
    assert!(!run(true));
}
#[test]
fn records_never_drop_and_early_pause_fails_gate() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    for _ in 0..10000 {
        heap.collect(&mut ());
    }
    assert_eq!(heap.gc_records().len(), 10000);
    assert_eq!(heap.gc_stats().count, 10000);
    let records = heap.gc_records();
    assert_eq!(
        heap.gc_stats().max_ns,
        records.iter().map(|r| r.gc_elapsed_ns).max().unwrap()
    );
    assert_eq!(
        heap.gc_stats().sum_ns,
        records
            .iter()
            .map(|r| r.gc_elapsed_ns as u128)
            .sum::<u128>()
    );
    // Gate sensitivity uses controlled durations, never host wall-clock bounds.
    let mut planted = records.to_vec();
    for record in &mut planted {
        record.gc_elapsed_ns = 1000;
    }
    let passes =
        |records: &[GcRecord]| records.iter().map(|r| r.gc_elapsed_ns).max().unwrap() <= 10_000_000;
    assert!(passes(&planted));
    planted[2].gc_elapsed_ns = 20_000_000;
    assert!(!passes(&planted));
}

struct LegacyGraph {
    edge: RawValue,
    visits: Rc<RefCell<usize>>,
    drops: Arc<AtomicUsize>,
    bytes: Rc<RefCell<usize>>,
}
// SAFETY: reports its sole strong edge; callbacks only access fixture counters.
impl crate::foreign_tests::Fixture for LegacyGraph {
    fn trace(&self, m: &mut Marker<'_>) {
        *self.visits.borrow_mut() += 1;
        m.mark(self.edge);
    }
    fn bytes(&self) -> usize {
        *self.bytes.borrow()
    }
}
impl Drop for LegacyGraph {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
#[test]
fn legacy_trace_once_external_resize_and_drop() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    crate::foreign_tests::install_fixture::<LegacyGraph>(&mut heap);
    let visits = Rc::new(RefCell::new(0));
    let bytes = Rc::new(RefCell::new(100));
    let drops = Arc::new(AtomicUsize::new(0));
    let child = heap.make_string(b"child", false).unwrap();
    let edge = RawValue::from_value(child);
    // SAFETY: the fixture edge is live in this heap, no interior borrow spans GC.
    let old = crate::foreign_tests::place_fixture(
        &heap,
        LegacyGraph {
            edge,
            visits: visits.clone(),
            drops: drops.clone(),
            bytes: bytes.clone(),
        },
        crate::raw::ForeignFlags::NONE,
    );
    let root = Root::new(&heap, old, "legacy");
    let before = heap.accounted_bytes();
    let debt = heap.allocation_debt();
    *bytes.borrow_mut() = 1000;
    heap.account_external(old);
    assert_eq!(heap.accounted_bytes(), before + 900);
    assert_eq!(heap.allocation_debt(), debt + 900);
    *bytes.borrow_mut() = 50;
    heap.account_external(old);
    assert_eq!(heap.accounted_bytes(), before - 50);
    assert_eq!(heap.allocation_debt(), debt + 900);
    assert_eq!(heap.collect(&mut ()).freed_objects, 0);
    assert_eq!(
        *visits.borrow(),
        if cfg!(feature = "verify") { 2 } else { 1 }
    );
    drop(root);
    assert_eq!(heap.collect(&mut ()).freed_objects, 2);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
#[cfg(feature = "verify")]
#[test]
fn verifier_refusal_precedes_any_destruction() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    let drops = Arc::new(AtomicUsize::new(0));
    let child = heap
        .make_payload(PvecType::UserPtr, &[], Box::new(CountDrop(drops.clone())))
        .unwrap();
    let vector = heap
        .make_vectorlike(PvecType::NormalVector, &[child], &[])
        .unwrap();
    let root = Root::new(&heap, vector, "verify");
    let pages = heap.allocation_stats().released_pages;
    // Under the production-slot mutant this catches Lost, proves no drop/page release,
    // then resumes the original panic so the gate is genuinely RED.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| heap.collect(&mut ())));
    if let Err(error) = result {
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(heap.allocation_stats().released_pages, pages);
        drop(root);
        std::panic::resume_unwind(error);
    }
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(root);
}

#[test]
fn dropped_large_graph_dies_at_next_natural_full() {
    let mut heap = Heap::create(crate::HeapConfig {
        gc_cons_threshold: 1024,
        gc_cons_percentage: 0.1,
        ..Default::default()
    });
    let children: Vec<_> = (0..200)
        .map(|_| heap.make_string(&[7; 256], false).unwrap())
        .collect();
    let graph = heap
        .make_vectorlike(PvecType::NormalVector, &children, &[0; 9000])
        .unwrap();
    let root = Root::new(&heap, graph, "graph");
    drop(children);
    assert_eq!(heap.collect(&mut ()).freed_objects, 0);
    assert_eq!(heap.allocation_stats().large_objects, 1);
    drop(root);
    let mut churn = 0;
    while heap.safepoint() == GcEvent::None {
        heap.cons(heap.nil(), heap.nil()).unwrap();
        churn += 1;
    }
    assert!(churn > 0);
    let r = heap.collect(&mut ());
    assert_eq!(r.record.trigger, Trigger::Threshold);
    assert_eq!(r.freed_objects, 201 + churn);
    assert_eq!(heap.allocation_stats().large_objects, 0);
    assert_eq!(
        heap.accounted_bytes(),
        crate::sym::BUILTIN_COUNT as usize * 48
    );
}
#[test]
fn owner_timing_update_keeps_online_totals_exact() {
    let mut heap = Heap::create(crate::HeapConfig::default());
    for _ in 0..3 {
        let r = heap.collect(&mut ());
        heap.finish_gc_timings(
            r.record.seq,
            Duration::from_nanos(r.record.core_stop_ns + 1000),
            Duration::from_nanos(r.record.core_stop_ns + 2000),
        );
    }
    let records = heap.gc_records();
    assert_eq!(
        heap.gc_stats().sum_ns,
        records
            .iter()
            .map(|r| r.gc_elapsed_ns as u128)
            .sum::<u128>()
    );
    assert_eq!(
        heap.gc_stats().max_ns,
        records.iter().map(|r| r.gc_elapsed_ns).max().unwrap()
    );
    assert!(
        records
            .iter()
            .all(|r| r.input_blocking_ns > r.gc_elapsed_ns && r.gc_elapsed_ns > r.core_stop_ns)
    );
}
