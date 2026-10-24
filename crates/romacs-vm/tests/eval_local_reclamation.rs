use romacs_vm::Vm;

#[test]
fn dead_buffer_local_storage_is_bounded_in_bytes_and_objects() {
    let mut samples = Vec::new();
    for iterations in [1000, 2000, 4000] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.eval_source(&format!("(dotimes (i {iterations}) (with-temp-buffer (make-local-variable 'x) (set 'x (current-buffer))))")).unwrap();
        let before = vm.heap_stats().collections;
        vm.collect_garbage();
        let stats = vm.heap_stats();
        assert!(stats.collections > before, "must count a completed collection");
        samples.push((iterations, stats.live_objects, stats.live_bytes));
    }
    eprintln!("dead-buffer locals (N, live_objects, live_bytes): {samples:?}");
    let (_, objects, bytes) = samples[0];
    for &(iterations, retained_objects, retained_bytes) in &samples[1..] {
        assert!(retained_objects <= objects + 2,
            "dead-buffer local objects grow at N={iterations}: {samples:?}");
        assert!(retained_bytes <= bytes + 1024,
            "dead-buffer local bytes grow at N={iterations}: {samples:?}");
    }
}
