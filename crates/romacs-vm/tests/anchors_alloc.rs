//! One-test process: exact allocator counters cannot race unrelated tests.
use romacs_vm::anchors::{AnchorTree, Bias};
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
fn edits_collapse_and_movement_allocate_nothing() {
    let positions: Vec<_> = (0..13346)
        .map(|i| (i % 1000, if i & 1 == 0 { Bias::Left } else { Bias::Right }))
        .collect();
    let mut t = AnchorTree::new(1000);
    let handles: Vec<_> = positions
        .iter()
        .map(|&(p, b)| t.create(p, b).unwrap())
        .collect();
    let (allocs, live) = (ALLOCS.load(Relaxed), LIVE.load(Relaxed));
    for i in 0..1024 {
        assert!(t.delete(0, 1000));
        assert!(t.insert(0, 1000, i & 1 == 0));
        assert!(t.set(handles[i % handles.len()], 500, Bias::Right));
        std::hint::black_box(t.resolve(handles[(i + 1) % handles.len()]));
    }
    assert_eq!(ALLOCS.load(Relaxed), allocs, "edit allocated");
    assert_eq!(LIVE.load(Relaxed), live, "edit retained heap");
}
