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

fn churn(vm: &mut Vm, conses: u32) -> u64 {
    let before = vm.heap_stats().collections;
    vm.eval_source(&format!("(let ((i 0)) (while (< i {conses}) (cons i nil) (setq i (1+ i))))")).unwrap();
    vm.heap_stats().collections - before
}

/// GNU's pacing variables (alloc.c:5740-5809) decide when allocation
/// collects, read live: a binding of either takes effect at once.
#[test]
fn gc_cons_threshold_and_percentage_pace_collection() {
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    let eval = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    assert_eq!(eval(&mut vm, "(list gc-cons-threshold gc-cons-percentage)"), "(800000 0.1)", "GNU's defaults");
    // 160000 bytes is 10000 objects; nil drops the proportional term.
    vm.eval_source("(setq gc-cons-threshold 160000 gc-cons-percentage nil)").unwrap();
    let plain = churn(&mut vm, 200_000);
    assert!(plain >= 10, "a 10000-object threshold collects: {plain}");
    let before = vm.heap_stats().collections;
    let restored = eval(&mut vm, "(progn (let ((gc-cons-threshold most-positive-fixnum)) (let ((i 0)) (while (< i 200000) (cons i nil) (setq i (1+ i))))) gc-cons-threshold)");
    assert_eq!(restored, "160000", "the binding is undone");
    // Only the request boundary after the binding ends may collect, where
    // the same churn collected ten times above.
    assert!(vm.heap_stats().collections <= before + 1, "a raised threshold defers collection");
    // A proportional term above the threshold paces instead: 0.12, scaled
    // to 1.8, waits 18 traced heaps.
    vm.eval_source("(setq gc-cons-percentage 0.12 gr-live (make-list 20000 nil))").unwrap();
    let proportional = churn(&mut vm, 200_000);
    assert!(proportional * 3 <= plain, "a raised percentage defers collection: {proportional} vs {plain}");
    // Larger settings (gcmh-style 0.2 to 1.0, or absurd ones) are held at
    // that ceiling: they defer collection, and it still happens. 18 traced
    // heaps against the default's 11.3 is about 0.63 of its collections.
    let mut deferred = vec![];
    for p in ["0.2", "1.0", "1e9"] {
        vm.eval_source(&format!("(setq gc-cons-percentage {p})")).unwrap();
        deferred.push(churn(&mut vm, 2_000_000));
    }
    vm.eval_source("(setq gc-cons-percentage 0.1)").unwrap();
    let default = churn(&mut vm, 2_000_000);
    assert!(deferred.iter().all(|&n| n >= 1 && n * 5 <= default * 4), "clamped deferral: {deferred:?} vs default {default}");
    // The same ceiling, give or take where each churn starts in a cycle.
    assert!(deferred.iter().max().unwrap() - deferred.iter().min().unwrap() <= 1, "the ceiling holds: {deferred:?}");
    // Non-integer thresholds are refused, as GNU's DEFVAR_INT is.
    assert_eq!(eval(&mut vm, "(condition-case e (setq gc-cons-threshold 1.5) (error (car e)))"), "wrong-type-argument");
    // An embedder's floor replaces both variables.
    vm.set_collection_floor(1);
    assert!(churn(&mut vm, 200_000) >= 2, "a floor of 1 collects as the heap doubles");
}
