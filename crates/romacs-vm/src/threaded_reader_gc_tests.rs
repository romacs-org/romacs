use super::*;

#[test]
fn callback_collects_and_forwards_suspended_stack_in_place() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
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
    let conses = vm.conses.len() - vm.free.of(0);
    let closures = vm.closures.len() - vm.free.of(1);
    for _ in 0..128 {
        let callback = vm.execute_raw(maker).unwrap();
        let result = vm.reader_call(callback, &[]).unwrap();
        assert_eq!(vm.display(result).unwrap(), "(1 . 2)");
    }
    vm.collect(&mut []);
    assert_eq!(vm.conses.len() - vm.free.of(0), conses, "callback captures were retained");
    assert_eq!(vm.closures.len() - vm.free.of(1), closures, "callbacks were retained");
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

/// A native call into Lisp calls as `funcall' does: optional and rest
/// parameters, a captured closure, primitives, more arguments than the
/// trampolines pass in registers, an arity error and an autoload, with a
/// moving collection at every Lisp entry.
#[test]
fn native_calls_keep_funcall_semantics_under_stress() {
    let dir = std::env::temp_dir().join(format!("romacs-native-call-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("nc-auto.el"), "(defun nc-auto (x) (list 'loaded (* x 2)))\n").unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    vm.eval_source_raw("(defun nc-opt (a &optional b) (list a b)) (defun nc-rest (a &rest r) (cons a r)) \
                        (setq nc-cap (let ((x (list 40))) (lambda (y) (+ (car x) y)))) (autoload 'nc-auto \"nc-auto\")").unwrap();
    vm.set_gc_stress(1);
    let ints = |ns: std::ops::Range<i64>| ns.map(|n| Value::integer(n).unwrap()).collect::<Vec<_>>();
    let call = |vm: &mut Vm, function: &str, args: Vec<Value>| {
        let function = if function == "nc-cap" { vm.eval_source_raw("nc-cap").unwrap() } else { vm.intern(function) };
        vm.reader_call(function, &args).map(|value| vm.display(value).unwrap())
    };
    assert_eq!(call(&mut vm, "nc-opt", ints(1..2)).unwrap(), "(1 nil)");
    assert_eq!(call(&mut vm, "nc-opt", ints(1..3)).unwrap(), "(1 2)");
    assert!(call(&mut vm, "nc-opt", ints(1..4)).is_err(), "three arguments to a two-parameter function");
    assert_eq!(call(&mut vm, "nc-rest", ints(1..13)).unwrap(), "(1 2 3 4 5 6 7 8 9 10 11 12)");
    assert_eq!(call(&mut vm, "nc-cap", ints(2..3)).unwrap(), "42");
    assert_eq!(call(&mut vm, "+", ints(0..10)).unwrap(), "45");
    assert_eq!(call(&mut vm, "list", vec![]).unwrap(), "nil");
    let strings: Vec<Value> = (0..10).map(|n| vm.make_lisp_string(format!("s{n}").into_bytes(), true)).collect();
    let concat = vm.intern("concat");
    let joined = vm.reader_call(concat, &strings).unwrap();
    assert_eq!(vm.bytes_value(joined).unwrap(), b"s0s1s2s3s4s5s6s7s8s9");
    assert_eq!(call(&mut vm, "nc-auto", ints(21..22)).unwrap(), "(loaded 42)");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn callback_preserves_an_open_builders_collection_hold() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun inhibited-reader () (garbage-collect))")
        .unwrap();
    let callback = vm.intern("inhibited-reader");
    // A compiler builder expanding a macro holds collection (`Vm::function').
    vm.gc_holds += 1;
    let before = vm.heap_stats().collections;
    let result = vm.reader_call(callback, &[]).unwrap();
    assert_eq!(result, Value::NIL, "garbage-collect returns nil while inhibited");
    assert_eq!(vm.heap_stats().collections, before);
    assert_eq!(vm.gc_holds, 1);
}

/// Big payloads under a collection hold leave their request pending
/// without making every native return poll; once the hold ends, the next
/// native return polls and collects.
#[test]
fn a_held_collection_keeps_its_request_without_polling_at_every_return() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(defvar held-big nil)").unwrap();
    vm.gc_holds += 1;
    POLLS.with(|polls| polls.set(0));
    vm.eval_source("(progn (setq held-big (make-string 4000000 ?x)) (dotimes (_ 5000) (make-string 4 ?y)))").unwrap();
    let polls = POLLS.with(|polls| polls.get());
    assert!(vm.collect_poll, "the request waits for the hold");
    assert!(polls < 50, "{polls} polls over 5000 native returns under a hold");
    vm.gc_holds -= 1;
    let before = vm.heap_stats().collections;
    vm.eval_source("(dotimes (_ 3) (make-string 4 ?y))").unwrap();
    assert!(vm.heap_stats().collections > before, "collected once the hold ended");
    assert!(!vm.collect_poll);
}

#[test]
fn a_collection_drops_idle_programs_code_and_nested_runs_relower_it() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun spare-callee (x) (list x (cons x 'tail)))").unwrap();
    let callee = vm.intern("spare-callee");
    let first = vm.reader_call(callee, &[Value::integer(1).unwrap()]).unwrap();
    assert_eq!(vm.display(first).unwrap(), "(1 (1 . tail))");
    assert!(vm.spare_programs.iter().any(|p| p.code_len() > 0), "a nested run leaves its program idle");
    vm.collect(&mut []);
    assert!(vm.spare_programs.iter().all(|p| p.code_len() == 0), "a collection drops idle lowered code");
    let again = vm.reader_call(callee, &[Value::integer(2).unwrap()]).unwrap();
    assert_eq!(vm.display(again).unwrap(), "(2 (2 . tail))");
}

struct PanickingHost;
impl crate::HostInput for PanickingHost {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: crate::HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<crate::HostEvent, String> {
        panic!("host failure while Lisp waits")
    }
}

struct PanickingPoll;
impl crate::HostInput for PanickingPoll {
    fn input_pending(&mut self) -> bool { panic!("host failure while a leaf polls") }
    fn redisplay(&mut self, _: crate::HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<crate::HostEvent, String> {
        Err("no events".into())
    }
}

/// A panic out of a leaf a site runs leaves no address of that site behind,
/// for the next call into Lisp to read as a live leaf.
#[test]
fn a_panic_out_of_a_site_leaf_forgets_the_site() {
    let mut vm = Vm::new();
    vm.eval_source_raw("(defun spare-poll () (input-pending-p))").unwrap();
    vm.set_host_input(Some(Box::new(PanickingPoll)));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.eval_source_raw("(spare-poll)")));
    assert!(caught.is_err(), "the host panic propagates");
    assert_eq!(vm.site_leaf, (0, 0));
    vm.set_host_input(None);
    let poll = vm.intern("spare-poll");
    assert_eq!(vm.reader_call(poll, &[]).unwrap(), Value::NIL);
}

/// A world cannot hold the address of a leaf in flight.
#[test]
fn a_world_refuses_a_site_leaf_in_flight() {
    let mut vm = Vm::new();
    vm.world_normalize();
    assert_eq!(vm.world_quiescent(), Ok(()));
    vm.site_leaf = (1, 1);
    assert!(vm.world_quiescent().unwrap_err().contains("site_leaf"));
    vm.site_leaf = (0, 0);
}

/// A panic out of a host wait, with engines suspended by `eval' and the
/// prompt, leaves no suspension state behind once caught: a collection
/// then traces only live engines, and Lisp runs and collects again.
#[test]
fn a_panic_inside_suspended_calls_unwinds_the_suspension_state() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let probe = vm.eval_source_raw("(condition-case e (read-string \"\") (error (car e)))").unwrap();
    assert_eq!(vm.display(probe).unwrap(), "end-of-file");
    vm.set_host_input(Some(Box::new(PanickingHost)));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vm.eval_source_raw("(let ((keep (list 1 2))) (eval '(read-from-minibuffer \"P: \")) keep)")
    }));
    assert!(caught.is_err(), "the host panic propagates");
    assert!(vm.suspended_code.is_empty(), "no suspended engine outlives the panic");
    assert!(vm.suspended_stacks.is_empty() && vm.stack.is_empty());
    assert!(vm.external_roots.is_empty());
    assert_eq!((vm.running_engines, vm.gc_holds, vm.rooted_write), (0, 0, false));
    assert!(vm.signal_handlers.is_empty());
    vm.set_host_input(None);
    vm.collect(&mut []);
    vm.set_gc_stress(1);
    let before = vm.heap_stats().collections;
    let value = vm.eval_source_raw("(let ((x (list (quote after) (copy-sequence \"aa\")))) (garbage-collect) x)").unwrap();
    assert_eq!(vm.display(value).unwrap(), "(after \"aa\")");
    assert!(vm.heap_stats().collections > before);
}
