//! One-test process: exact allocator counters cannot race unrelated tests.
use romacs_vm::anchors::{AnchorTree, Bias};
#[path = "support/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{Counting, measure};
#[global_allocator]
static A: Counting = Counting;
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
    let ((), _, allocs, live) = measure(|| for i in 0..1024 {
        assert!(t.delete(0, 1000));
        assert!(t.insert(0, 1000, i & 1 == 0));
        assert!(t.set(handles[i % handles.len()], 500, Bias::Right));
        std::hint::black_box(t.resolve(handles[(i + 1) % handles.len()]));
    });
    assert_eq!(allocs, 0, "edit allocated");
    assert_eq!(live, 0, "edit retained heap");
}
