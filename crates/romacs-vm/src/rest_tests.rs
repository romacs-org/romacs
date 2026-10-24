//! Opaque rest bindings: allocation, observation, and moving-root regression.
use super::*;

#[test]
fn direct_tier_reads_current_buffer_local_value() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defvar dz 1) (set-buffer (get-buffer-create \"direct-local\")) (make-local-variable 'dz) (setq dz 5)").unwrap();
    let before = vm.source_cache_stats().direct;
    let answer = vm.eval_source_raw("(+ dz 0)").unwrap();
    assert_eq!(
        answer,
        Value::integer(5).unwrap(),
        "direct tier must read the current buffer-local binding"
    );
    assert_eq!(
        vm.source_cache_stats().direct,
        before + 1,
        "fixture must exercise the direct tier"
    );
}

#[test]
fn opaque_rest_allocation_observation_and_compaction() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    vm.eval_source_raw(
        "(defvar rest1-n 0)
        (defun rest1-ignore (&rest r) 42)
        (defun rest1-run ()
          (let ((i 0))
            (while (< i rest1-n)
              (rest1-ignore 11 22 33)
              (setq i (1+ i))) i))",
    )
    .unwrap();
    let slot = vm.slot("rest1-n");
    let mut counts = Vec::new();
    let mut capacities = Vec::new();
    for n in [100, 1000, 10000] {
        vm.slots[slot].value = Value::integer(n).unwrap();
        let before = vm.heap_stats().allocations;
        let answer = vm.eval_source_raw("(rest1-run)").unwrap();
        assert_eq!(answer, Value::integer(n).unwrap());
        let count = vm.heap_stats().allocations - before;
        let capacity = vm.execution.rest_stack_capacity();
        eprintln!("rest1 N={n} allocations={count} stack_capacity={capacity:?}");
        capacities.push(capacity);
        counts.push(count);
    }
    assert!(
        counts[2] <= counts[0] + 16,
        "unused rest tails must allocate O(1), not per call: {counts:?}"
    );

    assert!(
        capacities.iter().all(|&capacity| capacity == capacities[0]),
        "rest argument-stack backing must be reused, not grow per call: {capacities:?}"
    );

    threaded::assert_opaque_rest_compaction();

    vm.set_collection_floor(1);
    let before_gc = vm.heap_stats().collections;
    let live = vm
        .eval_source_raw(
            "(defun rest1-wait (&rest r)
        (let ((i 0)) (while (< i 20010) (cons i nil) (setq i (1+ i)))) r)
        (rest1-wait (list 17) (list 42))",
        )
        .unwrap();
    assert_eq!(
        vm.display(live).unwrap(),
        "((17) (42))",
        "opaque rest must survive real dispatch safepoints"
    );
    assert!(
        vm.heap_stats().collections > before_gc,
        "live opaque tail fixture must actually collect"
    );
    let nested = vm
        .eval_source_raw(
            "(list
        (apply (lambda (&rest r) (apply 'rest1-ignore '(1 2 3 4 5)) r)
               '((7) (8)))
        (funcall (lambda (&rest r)
                   (catch 'done (funcall (lambda (&rest inner) (throw 'done 9)) 4 5))
                   r) 1 2)
        (funcall (lambda (&rest r)
                   (condition-case nil
                       (funcall (lambda (&rest inner) (car 1)) 4 5)
                     (error nil)) r) 3 4))",
        )
        .unwrap();
    assert_eq!(
        vm.display(nested).unwrap(),
        "(((7) (8)) (1 2) (3 4))",
        "nested apply scratch reuse and nonlocal exits must preserve caller tails"
    );

    let source = "(let* ((r (funcall (lambda (&rest r) r) 1 2 3))
                         (alias r)
                         (fresh (funcall (lambda (&rest r) r) 1 2 3)))
        (list (length r) (car r) (cdr r) (nth 2 r)
              (equal r fresh) (eq r alias) (eq r fresh)
              (append r '(4))
              (progn (setcar r 9) (car alias))
              (funcall (lambda (&rest r) (list (eq r r) r r)) 7 8)
              (funcall (lambda (&rest r) (setq rest1-saved r) r) 5 6)
              rest1-saved
              (funcall (lambda (&rest r) (funcall (lambda (x) x) r)) 3 4)
              (funcall (lambda (&rest r) (funcall (lambda () r))) 8 9)
              (funcall (lambda (x &optional y &rest r) (list x y r)) 1)
              (funcall (lambda (&rest r) (setq r 17) r) 1 2)))";
    let result = vm.eval_source_raw(source).unwrap();
    assert_eq!(
        vm.display(result).unwrap(),
        "(3 1 (2 3) 3 t t nil (1 2 3 4) 9 (t (7 8) (7 8)) (5 6) (5 6) (3 4) (8 9) (1 nil nil) 17)",
        "observed rest bindings must be ordinary, shared mutable lists"
    );
}
