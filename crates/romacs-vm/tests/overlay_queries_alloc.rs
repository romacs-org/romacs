//! One-test process: exact allocator counters cannot race unrelated tests.
use romacs_vm::{Form, Vm, coordinates::ByteOffset};
#[path = "support/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::{Counting, measure};
#[global_allocator]
static A: Counting = Counting;
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
    let ((), _, allocs, live) = measure(|| for i in 0..6673 {
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
    });
    assert_eq!(allocs, 0, "native query allocated");
    assert_eq!(live, 0, "native query retained heap");
}
