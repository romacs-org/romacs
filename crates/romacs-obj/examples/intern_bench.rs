use romacs_mem::{Heap, HeapConfig, PvecType, Root, spawn_isolate};
use romacs_obj::Obarray;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn timed_intern(heap: &Heap, names: &Root, table: &Root) -> Duration {
    let names = heap.as_vectorlike(names.get(heap)).unwrap();
    let ob = Obarray::from_value(heap, table.get(heap)).unwrap();
    let start = Instant::now();
    for i in 0..names.len() {
        black_box(ob.intern_string(names.get(i).unwrap()).unwrap());
    }
    let elapsed = start.elapsed();
    assert_eq!(ob.len(), 100000);
    let mut count = 0;
    ob.for_each(&mut |_| count += 1);
    assert_eq!(count, 100000);
    elapsed
}

fn main() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        // Like GNU's diag rows, actual Lisp names are prepared before timing.
        let names = {
            let values: Vec<_> = (0..100000)
                .map(|i| {
                    heap.make_string(format!("core-bench-sym-{i}").as_bytes(), false)
                        .unwrap()
                })
                .collect();
            Root::new(
                &heap,
                heap.make_vectorlike(PvecType::NormalVector, &values, &[])
                    .unwrap(),
                "bench-names",
            )
        };
        let mut fresh = Vec::new();
        let mut hit = Vec::new();
        // One warmup, then five repetitions, full GC outside both timed regions.
        for rep in 0..6 {
            heap.collect(&mut ());
            let table = Root::new(&heap, Obarray::new(&heap).unwrap().value(), "bench-obarray");
            let fresh_time = timed_intern(&heap, &names, &table);
            heap.collect(&mut ());
            let hit_time = timed_intern(&heap, &names, &table);
            if rep != 0 {
                fresh.push(fresh_time);
                hit.push(hit_time);
            }
        }
        fresh.sort();
        hit.sort();
        println!(
            "diagnostic: names=100000 fresh_ms={:.3} hit_ms={:.3}",
            fresh[2].as_secs_f64() * 1000.,
            hit[2].as_secs_f64() * 1000.
        );
    })
    .join()
    .unwrap();
}
