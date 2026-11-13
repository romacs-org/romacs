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

/// The allocations of a thousand evaluations of BODY, which the loop's
/// last value must show to be EXPECTED.
fn count_calls(vm: &mut Vm, body: &str, expected: &str) -> usize {
    let source = format!("(let ((i 0) (r nil)) (while (< i 1000) (setq r {body}) (setq i (1+ i))) r)");
    let f = vm.compile_source(&source).unwrap();
    vm.execute(f).unwrap();
    ALLOCS.with(|n| n.set(0));
    TRACK.with(|active| active.set(true));
    let result = std::hint::black_box(vm.execute(f)).and_then(romacs_vm::Local::display);
    TRACK.with(|active| active.set(false));
    assert_eq!(result.as_deref(), Ok(expected), "{body}");
    ALLOCS.with(Cell::get)
}

/// A search or a syntax scan that calls no Lisp allocates nothing, as
/// GNU's works in its C frame and one global set of registers: in a
/// multibyte buffer with its own syntax table, `syntax-table` properties
/// honoured and propertized to the end.
#[test]
fn searches_and_scans_allocate_nothing_per_call() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX / 4);
    vm.eval_source(
        r#"(progn
             (set-buffer (get-buffer-create "entry"))
             (let ((i 0)) (while (< i 50) (insert "  (defun exämple (arg)) ;; comment here\n") (setq i (1+ i))))
             (let ((table (make-char-table 'syntax-table)))
               (set-char-table-parent table (standard-syntax-table))
               (modify-syntax-entry ?\; "<" table) (modify-syntax-entry ?\n ">" table)
               (set-syntax-table table))
             (make-local-variable 'parse-sexp-ignore-comments) (setq parse-sexp-ignore-comments t)
             (make-local-variable 'parse-sexp-lookup-properties) (setq parse-sexp-lookup-properties t)
             (put-text-property 3 4 'syntax-table '(4 . 41))
             (setq syntax-propertize--done most-positive-fixnum)
             (goto-char 10))"#,
    )
    .unwrap();
    for (body, expected) in [
        (r#"(progn (goto-char 3) (looking-at "(def\\(un\\)"))"#, "t"),
        (r#"(progn (goto-char 10) (re-search-forward "ex\\(ä\\)mple" nil t))"#, "17"),
        (r#"(progn (goto-char 30) (re-search-backward "ex\\(ä\\)mple" 5 t))"#, "10"),
        (r#"(progn (goto-char 10) (search-forward "mple" nil t))"#, "17"),
        (r#"(string-match "b+\\(ä\\)" "aäbbäcc")"#, "2"),
        (r#"(progn (goto-char 24) (skip-syntax-forward "^w") (point))"#, "28"),
        (r#"(progn (goto-char 24) (skip-syntax-backward "w_") (point))"#, "24"),
        ("(progn (goto-char 24) (forward-comment 1) (point))", "41"),
        // Past no character beyond ASCII: a table allocates to give those.
        ("(scan-sexps 18 1)", "23"),
        ("(scan-lists 18 1 0)", "23"),
        ("(progn (goto-char 12) (backward-prefix-chars) (point))", "12"),
    ] {
        let allocations = count_calls(&mut vm, body, expected);
        // The execution frame may be allocated once; a call allocates nothing.
        assert!(allocations <= 8, "{body}: {allocations} allocations for 1000 calls");
    }
}
