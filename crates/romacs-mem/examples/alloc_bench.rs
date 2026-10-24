//! S2 storage-only allocation timings. No object initialization, string bytes,
//! evaluator, or GC cost: GNU constructor timings are an informational reference.
use romacs_mem::{HeapConfig, Kind, spawn_isolate};
use std::{hint::black_box, time::Instant};
fn main() {
    let n = std::env::args()
        .nth(1)
        .map(|s| s.parse::<u64>().expect("iteration count"))
        .unwrap_or(10_000_000);
    for (name, kind, size) in [
        ("cons", Kind::Cons, 16),
        ("string-header", Kind::String, 48),
        ("vector-2", Kind::Vectorlike, 32),
    ] {
        spawn_isolate(HeapConfig::default(), move |heap| {
            let start = Instant::now();
            for _ in 0..n {
                // SAFETY: benchmark never decodes or retains the uninitialized words.
                black_box(unsafe { heap.reserve_storage(kind, size).unwrap() });
            }
            let elapsed = start.elapsed().as_secs_f64();
            let stats = heap.allocation_stats();
            let count = match kind {
                Kind::Cons => stats.counts.conses,
                Kind::String => stats.counts.strings,
                Kind::Vectorlike => stats.counts.vector_cells / 2,
                _ => unreachable!(),
            };
            assert_eq!(count, n);
            println!(
                "rust-storage\t{name}\t{n}\t{elapsed:.6}\t{}",
                stats.live_bytes
            );
        })
        .join()
        .unwrap();
    }
}
