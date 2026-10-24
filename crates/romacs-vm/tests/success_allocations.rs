//! Count only allocations on this test thread, after lowering and storage warmup.
use romacs_vm::Vm;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        TRACK.with(|active| {
            if active.get() {
                ALLOCS.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        TRACK.with(|active| {
            if active.get() {
                ALLOCS.with(|n| n.set(n.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn count(vm: &mut Vm, source: &str) -> usize {
    let f = vm.compile_source(source).unwrap();
    vm.execute(f).unwrap();
    ALLOCS.with(|n| n.set(0));
    TRACK.with(|active| active.set(true));
    let result = std::hint::black_box(vm.execute(f));
    TRACK.with(|active| active.set(false));
    assert_eq!(result.unwrap().as_integer().unwrap(), 1000);
    ALLOCS.with(Cell::get)
}
#[test]
fn successful_accesses_do_not_allocate_errors() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX / 4);
    vm.eval_source("(defvar CELL (cons 1 2)) (defvar VEC (vector 1 2))")
        .unwrap();
    for body in ["(car CELL)", "(aref VEC 0)"] {
        let source = format!("(let ((i 0)) (while (< i 1000) {body} (setq i (1+ i))) i)");
        let allocations = count(&mut vm, &source);
        // The execution frame allocation is retained; accesses need none.
        assert!(
            allocations <= 2,
            "{body}: {allocations} allocations for 1000 successful accesses"
        );
    }
}
#[test]
fn closures_use_one_environment_allocation_not_one_per_capture() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX / 4);
    vm.eval_source("(defvar G nil)").unwrap();
    let allocations = count(
        &mut vm,
        "(let ((i 0) (a 1) (b 2) (c 3)) (while (< i 1000) (setq G (lambda () (+ i a b c))) (setq i (1+ i))) i)",
    );
    // One contiguous capture allocation per closure, plus bounded arena/frame
    // growth. No intermediate capture Vec followed by a second heap object.
    assert!(
        allocations <= 1005,
        "{allocations} allocations for 1000 closures"
    );
}

#[test]
fn numeric_type_tests_do_not_allocate_errors() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX / 4);
    let allocations = count(
        &mut vm,
        "(let ((i 0)) (while (< i 1000) (+ i i i) (setq i (1+ i))) i)",
    );
    // Cached fixnum addition must neither allocate argument vectors nor format
    // errors while inspecting the argument tags.
    assert!(
        allocations <= 2,
        "{allocations} allocations for 1000 n-ary additions"
    );
}
