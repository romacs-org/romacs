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

/// A property write costs the runs it touches: it copies no run of its
/// range into a list, compares no key by name and edits the leaf holding
/// its runs in place, so what 1000 writes allocate is bounded, and the same
/// whether the text has a thousand runs or 64 thousand, five other
/// properties or 640.  Each round of writes changes what the last one left:
/// a run's value, nothing, a second property, and a part of a run.
#[test]
fn property_writes_allocate_for_the_runs_they_touch_not_for_the_text() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX / 4);
    vm.eval_source("(defvar ROUND 0)").unwrap();
    let mut counts = Vec::new();
    for (name, runs, others) in [("small", 1_000, 5), ("large", 64_000, 5), ("many-keys", 64_000, 640)] {
        vm.eval_source(&format!(
            "(with-current-buffer (get-buffer-create {name:?}) (setq buffer-undo-list t) (insert (make-string {} ?x)) \
               (dotimes (i {runs}) \
                 (put-text-property (+ 1 (* i 10)) (+ 6 (* i 10)) 'face (if (= 0 (% i 2)) 'bold 'italic)) \
                 (when (= 0 (% i 50)) \
                   (put-text-property (+ 3 (* i 10)) (+ 9 (* i 10)) (intern (format \"other-%d\" (% i {others}))) i))) \
               nil)",
            runs * 10
        ))
        .unwrap();
        let source = format!(
            "(with-current-buffer {name:?} \
               (let ((inhibit-modification-hooks t) (i 0) (even (= 0 (% ROUND 2)))) \
                 (while (< i 1000) \
                   (let ((p (+ 1 (* 10 (% (* i 7919) {runs})))) (k (% i 4))) \
                     (cond ((= k 0) (put-text-property p (+ p 5) 'face (if even 'underline 'shadow))) \
                           ((= k 1) (put-text-property p (+ p 5) 'face (get-text-property p 'face))) \
                           ((= k 2) (put-text-property p (+ p 7) 'written ROUND)) \
                           (t (put-text-property (+ p 1) (+ p 3) 'face (if even 'bold 'italic))))) \
                   (setq i (1+ i))) \
                 (setq ROUND (1+ ROUND)) \
                 i))"
        );
        counts.push((name, count(&mut vm, &source)));
    }
    for (name, allocations) in &counts {
        assert!(*allocations <= 100, "{name}: {allocations} allocations for 1000 property writes: {counts:?}");
    }
}
