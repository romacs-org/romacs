//! The heap boundary is a compile-time property (tests/compile_boundaries.rs).
//!
//! The runtime tests below are the net under the type: a `Rooted` handle is
//! the only way to hold a value across a collection, so it must resolve after
//! a compacting collection and must stop retaining anything once dropped.

use romacs_vm::Vm;

/// GC.md's "existing wait/budget points": the executor polls at backward
/// branches and calls and may collect mid-request, with live registers, not
/// only at the request boundary. `finish_request` collects at most once per
/// request, so two or more collections inside one `eval_source` can only come
/// from `poll_resume`.
#[test]
fn collection_runs_inside_one_request_not_only_at_its_boundary() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.collect_garbage();
    let before = vm.heap_stats().collections;
    let shown = vm
        .eval_source("(let ((keep (list 1 2)) (i 0)) (while (< i 100000) (cons i nil) (setq i (1+ i))) keep)")
        .unwrap()
        .display()
        .unwrap();
    let during = vm.heap_stats().collections - before;
    assert_eq!(
        shown, "(1 2)",
        "live register value survives mid-request collection"
    );
    assert!(
        during >= 2,
        "mid-request collection: {during} collections in one request"
    );
}

#[test]
fn rooted_handle_resolves_after_compaction_and_releases_on_drop() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX);
    vm.set_gc_moving(true);
    let source = "(progn (let ((i 0)) (while (< i 64) (cons i nil) (setq i (1+ i)))) (list (copy-sequence \"rooted\") (vector 1 2)))";
    // Warm the source cache so the measured request adds only its own data.
    vm.eval_source(source).unwrap();
    vm.collect_garbage();
    let base = vm.heap_stats();
    let value = vm.eval_source(source).unwrap();
    let bits = format!("{value:?}");
    let shown = value.display().unwrap();
    let rooted = value.root();
    vm.collect_garbage();
    let live = rooted.get(&vm).unwrap();
    assert_eq!(
        live.display(),
        Ok(shown),
        "rooted handle must resolve to the same value after compaction"
    );
    assert_ne!(
        format!("{live:?}"),
        bits,
        "fixture must actually move the value"
    );
    let held = vm.heap_stats();
    assert!(
        held.live_objects > base.live_objects && held.live_bytes > base.live_bytes,
        "a live handle retains its value: {held:?} vs {base:?}"
    );
    drop(rooted);
    vm.collect_garbage();
    let released = vm.heap_stats();
    let delta = (
        released.live_objects as i64 - base.live_objects as i64,
        released.live_bytes as i64 - base.live_bytes as i64,
    );
    assert_eq!(
        delta,
        (0, 0),
        "dropping the handle must release it: live_objects/live_bytes delta"
    );
}

#[test]
fn rooted_handle_is_bound_to_its_vm() {
    let mut a = Vm::new();
    let b = Vm::new();
    let rooted = a.eval_source("(list 1)").unwrap().root();
    let foreign = rooted.get(&b);
    assert!(
        matches!(&foreign, Err(e) if e.contains("another VM")),
        "a handle must not resolve in another VM: {foreign:?}"
    );
    assert_eq!(rooted.get(&a).unwrap().display().unwrap(), "(1)");
}
