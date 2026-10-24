use romacs_vm::Vm;

#[test]
fn garbage_collect_is_callable_and_really_collects() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_stats().live_objects, 0);
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i))))")
        .unwrap();
    let before = vm.heap_stats();
    let result = vm.eval_source("(funcall 'garbage-collect)");
    assert!(result.is_ok(), "GC_CALLABLE: {result:?}");
    drop(result);
    let after = vm.heap_stats();
    assert!(after.collections > before.collections, "GC_PERFORMED");
    assert!(after.reclaimed > before.reclaimed, "GC_RECLAIMED");
    assert_eq!(
        vm.eval_source("(interactive-form 'garbage-collect)")
            .unwrap()
            .display()
            .unwrap(),
        "(interactive \"\")"
    );
}

#[test]
fn garbage_collect_rows_and_absent_pool_are_honest() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let result = vm.eval_source("(mapcar 'car (garbage-collect))");
    assert!(result.is_ok(), "GC_ROWS: {result:?}");
    assert_eq!(
        result.unwrap().display().unwrap(),
        "(conses symbols strings string-bytes vectors vector-slots floats buffers)",
        "GC_NO_FABRICATED_INTERVAL_POOL"
    );
    assert_eq!(vm.eval_source("(mapcar (lambda (row) (list (car row) (length row) (mapcar 'integerp (cdr row)))) (garbage-collect))").unwrap().display().unwrap(), "((conses 4 (t t t)) (symbols 4 (t t t)) (strings 4 (t t t)) (string-bytes 3 (t t)) (vectors 3 (t t)) (vector-slots 4 (t t t)) (floats 4 (t t t)) (buffers 3 (t t)))", "GC_EXACT_ROW_SHAPE_AND_INTEGER_TYPES");
    assert_eq!(
        vm.eval_source("(condition-case e (garbage-collect 1) (error e))")
            .unwrap()
            .display()
            .unwrap(),
        "(wrong-number-of-arguments #<subr garbage-collect> 1)",
        "GC_EXACT_ARITY"
    );
}

#[test]
fn garbage_collect_forwards_live_call_state() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i))))")
        .unwrap();
    let result =
        vm.eval_source("(let ((a (list 11 22)) (b (vector 33 44))) (garbage-collect) (list a b))");
    assert!(result.is_ok(), "GC_LIVE_CALL: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "((11 22) [33 44])");
}

#[test]
fn garbage_collect_obeys_existing_inhibition() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX);
    let before = vm.heap_stats().collections;
    assert_eq!(
        vm.eval_source("(garbage-collect)")
            .unwrap()
            .display()
            .unwrap(),
        "nil",
        "GC_INHIBITED_RESULT"
    );
    assert_eq!(vm.heap_stats().collections, before, "GC_INHIBITED_COUNT");
}
