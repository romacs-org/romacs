//! Preliminary strong-only core-stop benchmark, not complete-collector H1 evidence.
use romacs_mem::{HeapConfig, PvecType, Root, spawn_isolate};
fn main() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let mut head = heap.nil();
        for _ in 0..4096 {
            let s = heap.make_string(&[b'x'; 1024], false).unwrap();
            let v = heap
                .make_vectorlike(PvecType::NormalVector, &[s, heap.fixnum(17).unwrap()], &[])
                .unwrap();
            head = heap.cons(v, head).unwrap();
        }
        let root = Root::new(&heap, head, "bench");
        let live = heap.accounted_bytes();
        let mut times = Vec::new();
        for _ in 0..7 {
            times.push(heap.collect(&mut ()).record.core_stop_ns);
        }
        times.sort_unstable();
        println!(
            "preliminary strong-only: live={} median_ns={} ns_per_live_mib={:.0}",
            live,
            times[3],
            times[3] as f64 / (live as f64 / 1048576.0)
        );
        drop(root);
        assert!(heap.collect(&mut ()).freed_objects > 0);
    })
    .join()
    .unwrap();
}
