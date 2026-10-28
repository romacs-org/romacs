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

/// GNU's inhibited `garbage-collect' returns nil (alloc.c:6044). Romacs
/// inhibits where a native frame holds untraced Values (DIVERGENCES
/// VM2-GC-INHIBITED): a watcher a primitive's write runs, and macro
/// expansion inside an open compiler builder. A pacing floor does not
/// inhibit an explicit collection.
#[test]
fn garbage_collect_obeys_existing_inhibition() {
    let mut vm = Vm::new();
    // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
    vm.set_gc_stress(0);
    vm.set_collection_floor(usize::MAX);
    let eval = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    assert_ne!(eval(&mut vm, "(garbage-collect)"), "nil", "GC_PACING_IS_NOT_INHIBITION");
    eval(&mut vm, "(defvar gi-v nil)");
    eval(&mut vm, "(add-variable-watcher 'gi-v (lambda (&rest _) (setq gi-r (garbage-collect))))");
    eval(&mut vm, "(defmacro gi-m () (list 'quote (garbage-collect)))");
    let before = vm.heap_stats().collections;
    assert_eq!(eval(&mut vm, "(progn (set 'gi-v 1) gi-r)"), "nil", "GC_INHIBITED_RESULT");
    eval(&mut vm, "(defun gi-f () (gi-m))");
    assert_eq!(eval(&mut vm, "(gi-f)"), "nil", "GC_INHIBITED_RESULT");
    assert_eq!(vm.heap_stats().collections, before, "GC_INHIBITED_COUNT");
    // The same watcher collects from a compiled `setq', whose engine suspends.
    assert_ne!(eval(&mut vm, "(progn (setq gi-v 2) gi-r)"), "nil", "GC_WATCHED_SETQ");
}
