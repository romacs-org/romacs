//! One-test process: exact allocator counters cannot race unrelated tests.
use romacs_vm::{Form, Vm, coordinates::ByteOffset};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
struct Count;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            LIVE.fetch_add(l.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            LIVE.fetch_add(n, Relaxed);
            LIVE.fetch_sub(l.size(), Relaxed);
        }
        q
    }
}
#[global_allocator]
static A: Count = Count;
#[test]
fn native_queries_at_notmuch_scale_allocate_no_lists_or_cons_cells() {
    let mut vm = Vm::new();
    let source = serde_json::json!(["with-current-buffer", ["get-buffer-create", {"string": "overlay-allocation-live"}, "t"],
        ["insert", {"string": "x".repeat(886 * 1024)}],
        ["let", [["i", 0]],
            ["while", ["<", "i", 6673],
                ["make-overlay", ["+", ["*", "i", 100], 1], ["+", ["*", "i", 100], 51]],
                ["setq", "i", ["1+", "i"]]],
            ["current-buffer"]]]);
    let function = vm.compile(&Form::from_json(source).unwrap()).unwrap();
    let buffer = vm.execute(function).unwrap();
    let (allocs, live) = (ALLOCS.load(Relaxed), LIVE.load(Relaxed));
    for i in 0..6673 {
        let p = ByteOffset(i * 100 + 25);
        let mut hits = 0;
        buffer.visit_overlays_at(p, |_, range| {
            assert_eq!(range, ByteOffset(i * 100)..ByteOffset(i * 100 + 50));
            hits += 1;
        })
        .unwrap();
        assert_eq!(hits, 1);
        hits = 0;
        buffer.visit_overlays_in(p..ByteOffset(p.0 + 1), |_, _| hits += 1)
            .unwrap();
        assert_eq!(hits, 1);
        assert_eq!(
            buffer.next_overlay_change_byte(p).unwrap(),
            ByteOffset(i * 100 + 50)
        );
    }
    assert_eq!(ALLOCS.load(Relaxed), allocs, "native query allocated");
    assert_eq!(LIVE.load(Relaxed), live, "native query retained heap");
}
