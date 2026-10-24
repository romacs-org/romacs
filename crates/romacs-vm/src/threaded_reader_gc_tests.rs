use super::*;

#[test]
fn callback_collects_and_forwards_suspended_stack_in_place() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun collecting-reader (x) (garbage-collect) (setcar x 42) x)")
        .unwrap();
    for _ in 0..32 {
        vm.list_value(&[Value::NIL]);
    }
    let function = vm.intern("collecting-reader");
    let value = vm.list_value(&[Value::NIL]);
    vm.external_roots.push(value);
    vm.stack = vec![value, function];
    let ptr = vm.stack.as_ptr();
    let capacity = vm.stack.capacity();
    let before = vm.heap_stats().collections;
    let result = vm.reader_call(function, &[value]).unwrap();
    assert!(
        vm.heap_stats().collections > before,
        "callback collection was inhibited"
    );
    assert_ne!(result, value, "fixture must force forwarding");
    assert_eq!(vm.external_roots, vec![result]);
    assert_eq!(vm.stack, vec![result, function]);
    assert_eq!(vm.stack.as_ptr(), ptr);
    assert_eq!(vm.stack.capacity(), capacity);
    assert_eq!(vm.cons_pair(result).unwrap().0.as_integer().unwrap(), 42);
}

#[test]
fn ordinary_read_callback_collects_with_outer_cons_and_closure_arguments() {
    let mut vm = Vm::new();
    let code = vm.compile_source("(let ((chars '(55 32))) (funcall (lambda (x f) (read (lambda (&optional unread) (garbage-collect) (if unread (progn (setq chars (cons unread chars)) nil) (prog1 (car chars) (setq chars (cdr chars)))))) (+ (car x) (funcall f))) (cons 40 nil) (let ((payload (cons 2 nil))) (lambda () (car payload)))))").unwrap();
    let before = vm.heap_stats().collections;
    let value = vm.execute_raw(code).unwrap();
    assert_eq!(value.as_integer().unwrap(), 42);
    assert!(
        vm.heap_stats().collections > before,
        "read callback collection was inhibited"
    );
}

#[test]
fn callback_wrappers_reuse_request_slots_without_retaining_dead_callbacks() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun read-constant () 32)").unwrap();
    let callback = vm.intern("read-constant");
    let functions = vm.functions.len();
    let caches = vm.caches.len();
    for _ in 0..128 {
        assert_eq!(
            vm.reader_call(callback, &[]).unwrap().as_integer().unwrap(),
            32
        );
    }
    assert!(
        vm.functions.len() <= functions + 1,
        "per-character wrapper slots leaked"
    );
    assert!(
        vm.caches.len() <= caches + 1,
        "per-character cache slots leaked"
    );
    let maker = vm
        .compile_source("(let ((payload (cons 1 2))) (lambda () payload))")
        .unwrap();
    vm.collect(&mut []);
    let conses = vm.conses.len();
    let closures = vm.closures.len();
    for _ in 0..128 {
        let callback = vm.execute_raw(maker).unwrap();
        let result = vm.reader_call(callback, &[]).unwrap();
        assert_eq!(vm.display(result).unwrap(), "(1 . 2)");
    }
    vm.collect(&mut []);
    assert_eq!(vm.conses.len(), conses, "callback captures were retained");
    assert_eq!(vm.closures.len(), closures, "callbacks were retained");
}

#[test]
fn collecting_read_callback_propagates_throw_and_signal() {
    let mut vm = Vm::new();
    for source in [
        "(catch 'reader-tag (read (lambda (&optional unread) (garbage-collect) (throw 'reader-tag (cons 42 nil)))))",
        "(let ((tag (cons 1 nil))) (catch tag (read (lambda (&optional unread) (garbage-collect) (throw tag (cons 42 nil))))))",
        "(condition-case err (read (lambda (&optional unread) (garbage-collect) (signal 'arith-error (cons 42 nil)))) (arith-error (cdr err)))",
    ] {
        let code = vm.compile_source(source).unwrap();
        let before = vm.heap_stats().collections;
        let value = vm.execute_raw(code).unwrap();
        assert_eq!(vm.display(value).unwrap(), "(42)");
        assert!(vm.heap_stats().collections > before);
    }
}

#[test]
fn callback_preserves_existing_compiler_collection_inhibition() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun inhibited-reader () (garbage-collect) 32)")
        .unwrap();
    let callback = vm.intern("inhibited-reader");
    vm.gc_threshold = usize::MAX;
    let before = vm.heap_stats().collections;
    vm.reader_call(callback, &[]).unwrap();
    assert_eq!(vm.heap_stats().collections, before);
    assert_eq!(vm.gc_threshold, usize::MAX);
}
