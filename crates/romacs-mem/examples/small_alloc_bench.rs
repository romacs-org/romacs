//! Initialized constructors, discarded results; allocation never collects implicitly.
use romacs_mem::{GcEvent, Heap, HeapConfig, PvecType, Trace, spawn_isolate};
use std::{hint::black_box, time::Instant};
struct Empty;
impl Trace for Empty {
    fn trace(&self, _: &mut romacs_mem::raw::Marker<'_>) {}
}
fn run(heap: &mut Heap, shape: &str, n: usize, gc: bool, name: &romacs_mem::Root) {
    for base in (0..n).step_by(1000) {
        for _ in base..(base + 1000).min(n) {
            let nil = heap.nil();
            let value = match shape {
                "cons" => heap.cons(nil, nil),
                "string8" => heap.make_string(&[b'x'; 8], false),
                "string16" => heap.make_string(&[b'x'; 16], false),
                "string32" => heap.make_string(&[b'x'; 32], false),
                "vector4" => heap.make_vectorlike(PvecType::NormalVector, &[nil; 4], &[]),
                "symbol" => heap.make_symbol(name.get(heap)),
                _ => panic!("unknown shape"),
            }
            .unwrap();
            black_box(value);
        }
        if gc && heap.safepoint() == GcEvent::WantCycle {
            heap.collect(&mut Empty);
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let shape = args[1].clone();
    let n: usize = args[2].parse().unwrap();
    let gc = args[3] == "gc";
    spawn_isolate(
        HeapConfig {
            gc_cons_threshold: 800_000,
            gc_cons_percentage: 0.1,
            ..Default::default()
        },
        move |mut heap| {
            // A shared prebuilt name matches GNU make-symbol.
            let name =
                romacs_mem::Root::new(&heap, heap.make_string(b"name", false).unwrap(), "bench");
            run(&mut heap, &shape, n, gc, &name);
            heap.collect(&mut Empty);
            let before = heap.allocation_stats().counts.memory_use_counts();
            let cycles = heap.gc_stats().count;
            let start = Instant::now();
            run(&mut heap, &shape, n, gc, &name);
            let elapsed = start.elapsed().as_nanos();
            let after = heap.allocation_stats().counts.memory_use_counts();
            let (counter, units) = match shape.as_str() {
                "cons" => (0, 1),
                "symbol" => (3, 1),
                "vector4" => (2, 4),
                _ => (6, 1),
            };
            assert_eq!(after[counter] - before[counter], n as u64 * units);
            assert_eq!(heap.gc_stats().count > cycles, gc);
            println!("{elapsed}");
        },
    )
    .join()
    .unwrap();
}
