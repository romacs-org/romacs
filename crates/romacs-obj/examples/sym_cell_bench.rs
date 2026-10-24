use romacs_mem::{Heap, HeapConfig, Root, Value, spawn_isolate};
use romacs_obj::sym_cell;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const N: i64 = 10_000_000;

#[inline(never)]
fn hot_read<'h>(heap: &'h Heap, symbol: Value<'h>) -> Value<'h> {
    sym_cell(heap, symbol).unwrap().get()
}

fn timed(heap: &Heap, owner: &Root, fast: bool) -> Duration {
    let symbol = owner.get(heap);
    sym_cell(heap, symbol).unwrap().set(heap.fixnum(0).unwrap());
    let start = Instant::now();
    for i in 0..N {
        let symbol = black_box(symbol);
        if fast {
            let cell = sym_cell(heap, symbol).unwrap();
            black_box(cell.get());
            cell.set(black_box(heap.fixnum(i).unwrap()));
        } else {
            let cell = heap.as_symbol(symbol).unwrap();
            black_box(cell.value());
            cell.set_value(black_box(heap.fixnum(i).unwrap()));
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(hot_read(heap, symbol).as_fixnum(), Some(N - 1));
    elapsed
}

fn main() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let symbol = heap.make_symbol(heap.make_string(b"bench-dynamic", false).unwrap()).unwrap();
        let owner = Root::new(&heap, symbol, "bench-symbol");
        let mut old = Vec::new();
        let mut fast = Vec::new();
        let stats = heap.allocation_stats();
        for rep in 0..12 {
            let (a, b) = if rep % 2 == 0 {
                (timed(&heap, &owner, false), timed(&heap, &owner, true))
            } else {
                let b = timed(&heap, &owner, true);
                (timed(&heap, &owner, false), b)
            };
            if rep != 0 {
                println!("rep={rep} old_ms={:.3} fast_ms={:.3}", a.as_secs_f64()*1000., b.as_secs_f64()*1000.);
                old.push(a);
                fast.push(b);
            }
        }
        assert_eq!(heap.allocation_stats().counts, stats.counts);
        assert_eq!(heap.epoch(), 0);
        old.sort();
        fast.sort();
        println!("reads+writes={N} old_median_ms={:.3} fast_median_ms={:.3} old_range_ms={:.3}..{:.3} fast_range_ms={:.3}..{:.3} gc=0 allocs=0",
            old[5].as_secs_f64()*1000., fast[5].as_secs_f64()*1000.,
            old[0].as_secs_f64()*1000., old[10].as_secs_f64()*1000.,
            fast[0].as_secs_f64()*1000., fast[10].as_secs_f64()*1000.);
        heap.collect(&mut ());
    }).join().unwrap();
}
