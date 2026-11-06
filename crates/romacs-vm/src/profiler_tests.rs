//! profiler.c's contract: start/stop/running-p/log for both profilers,
//! the log's backtrace vectors and special entries, `function-equal' and
//! the hash test profiler.el defines over it.
use crate::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    vm.display(value).unwrap()
}

/// test/src/profiler-tests.el, both profilers.
#[test]
fn lifecycle_matches_gnus_profiler_tests() {
    let mut vm = Vm::new();
    for (start, stop, running, log) in [
        ("(profiler-memory-start)", "(profiler-memory-stop)", "(profiler-memory-running-p)", "(profiler-memory-log)"),
        ("(profiler-cpu-start 1000000)", "(profiler-cpu-stop)", "(profiler-cpu-running-p)", "(profiler-cpu-log)"),
    ] {
        let source = format!(
            "(list {stop} {start} (condition-case e {start} (error (car (cdr e)))) {running} (hash-table-p {log}) {running} {stop} {stop} (progn {log} {log}))"
        );
        let expected = if start.contains("cpu") { "CPU" } else { "Memory" };
        assert_eq!(eval(&mut vm, &source), format!("(nil t \"{expected} profiler is already running\" t t t t nil nil)"));
    }
    assert_eq!(eval(&mut vm, "(condition-case e (profiler-cpu-start 0) (error (car (cdr e))))"), "\"Invalid sampling interval\"");
    assert_eq!(eval(&mut vm, "(profiler-cpu-running-p)"), "nil");
}

/// A CPU-bound loop under a named function: its samples carry it, in
/// vectors of `profiler-max-stack-depth' innermost-first frames. The loop
/// runs for CPU time: a sample counts only on a CPU, so wall time on a
/// loaded host would take too few. A second of it outlasts a spell of the
/// sampler sharing the thread's CPU, when it sees nothing.
#[test]
fn cpu_log_records_the_running_backtrace() {
    let mut vm = Vm::new();
    let source = r#"
      (progn
        (setq profiler-max-stack-depth 4)
        (defalias 'pt-leaf (lambda (n) (let ((s 0)) (while (> n 0) (setq s (+ s n) n (1- n))) s)))
        (defalias 'pt-spin (lambda () (let ((end (+ (float-time (get-internal-run-time)) 1.0)))
                                        (while (< (float-time (get-internal-run-time)) end) (pt-leaf 2000)))))
        (profiler-cpu-start 1000000)
        (pt-spin)
        (let ((log (profiler-cpu-log)) (total 0) (leaf 0) (depths nil))
          (profiler-cpu-stop)
          (maphash (lambda (k v)
                     (setq total (+ total v))
                     (unless (memq (length k) depths) (push (length k) depths))
                     ;; In pt-leaf or a primitive it calls, under pt-spin.
                     (let ((at (if (eq (aref k 0) 'pt-leaf) 0 1)))
                       (when (eq (aref k at) 'pt-leaf)
                         (setq leaf (+ leaf v))
                         (or (eq (aref k (1+ at)) 'pt-spin) (error "caller %S" k)))))
                   log)
          (list depths (> total 100) (> (* 2 leaf) total))))"#;
    assert_eq!(eval(&mut vm, source), "((4) t t)");
}

/// Samples a collection takes are GNU's [Automatic\ GC nil] entry; a full
/// log evicts its least counted half into [Discarded\ Samples nil].
#[test]
fn special_entries_for_collection_and_eviction() {
    let mut vm = Vm::new();
    let source = r#"
      (progn
        (defalias 'pt-gc (lambda () (let ((end (+ (float-time) 0.3))) (while (< (float-time) end) (make-list 20000 nil) (garbage-collect)))))
        (profiler-cpu-start 1000000)
        (pt-gc)
        (profiler-cpu-stop)
        (let ((gc nil))
          (maphash (lambda (k v) (when (equal k (vector (intern "Automatic GC") nil)) (setq gc v))) (profiler-cpu-log))
          (natnump gc)))"#;
    assert_eq!(eval(&mut vm, source), "t");
    let source = r#"
      (progn
        (setq profiler-log-size 2)
        (profiler-memory-start)
        (funcall (lambda () (make-list 10 nil)))
        (list (make-list 10 nil))
        (vector (make-list 10 nil))
        (cons (make-list 10 nil) nil)
        (profiler-memory-stop)
        (let ((n 0) (discarded nil))
          (maphash (lambda (k v) (setq n (1+ n))
                     (when (equal k (vector (intern "Discarded Samples") nil)) (setq discarded v)))
                   (profiler-memory-log))
          (list (<= n 3) (natnump discarded))))"#;
    assert_eq!(eval(&mut vm, source), "(t t)");
}

/// Ffunction_equal: one function, or closures of one code; profiler.el's
/// `define-hash-table-test' over it makes a working table.
#[test]
fn function_equal_and_its_hash_test() {
    let mut vm = Vm::new();
    let source = r#"
      (progn
        (defalias 'pt-make (lambda (n) (lambda () n)))
        (let ((a (pt-make 1)) (b (pt-make 2)) (c (lambda () 3)))
          (define-hash-table-test 'pt-fe #'function-equal (lambda (f) (if (closurep f) (aref f 1) f)))
          (let ((h (make-hash-table :test 'pt-fe)))
            (puthash a 'a h)
            (puthash 'car 'car h)
            (garbage-collect)
            (list (function-equal a b) (function-equal a c) (function-equal 'car 'car) (function-equal 'car 'cdr)
                  (gethash b h) (gethash c h) (gethash 'car h) (hash-table-count h) (hash-table-test h)))))"#;
    assert_eq!(eval(&mut vm, source), "(t nil t nil a nil car 2 pt-fe)");
}

/// A burst of CPU followed by a wait is charged to the burst: a sample
/// counts only while the thread runs, so a wait never takes the samples
/// of the work before it. 400 bursts of 0.2 ms at 1 ms are about 80
/// samples (fewer when a loaded host delays the sampler); a sampler that
/// charged late lost about half of them to the wait.
#[test]
fn a_burst_before_a_wait_is_charged_to_the_burst() {
    let mut vm = Vm::new();
    let source = r#"
      (progn
        (defalias 'pt-burst (lambda () (let ((end (+ (float-time) 0.0002)) (n 0)) (while (< (float-time) end) (setq n (1+ n))) n)))
        (defalias 'pt-wait (lambda () (sleep-for 0.001)))
        (profiler-cpu-start 1000000)
        (dotimes (_ 400) (pt-burst) (pt-wait))
        (profiler-cpu-stop)
        (let ((burst 0) (wait 0))
          (maphash (lambda (k v) (cond ((memq 'pt-burst (append k nil)) (setq burst (+ burst v)))
                                       ((memq 'pt-wait (append k nil)) (setq wait (+ wait v)))))
                   (profiler-cpu-log))
          (list (>= burst 20) (< (* 3 wait) burst) burst wait)))"#;
    let result = eval(&mut vm, source);
    assert!(result.starts_with("(t t "), "{result}");
}

/// A log taken while the memory profiler runs is exported before it
/// restarts, so the next log does not hold the export's allocations.
#[test]
fn a_memory_log_does_not_count_its_own_export() {
    let mut vm = Vm::new();
    let source = "(progn (profiler-memory-start) (make-list 100 nil) (profiler-memory-log) (profiler-memory-stop) (hash-table-count (profiler-memory-log)))";
    assert_eq!(eval(&mut vm, source), "0");
}

/// Allocation into the entries a collection freed is allocation: after a
/// sweep leaves a dead list's conses free below a live one, a function
/// that conses into them is charged (gc.rs `FreeEntries::allocated').
#[test]
fn allocation_into_freed_entries_is_charged() {
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    let source = "(progn (setq pt-junk (make-list 5000 nil) pt-keep (list 1)) (setq pt-junk nil) (garbage-collect)
                   (defalias 'pt-cons (lambda () (make-list 2000 nil)))
                   (profiler-memory-start) (pt-cons) (profiler-memory-stop)
                   (let ((n 0))
                     (maphash (lambda (k v) (when (memq 'pt-cons (append k nil)) (setq n (+ n v)))) (profiler-memory-log))
                     (>= n 32000)))";
    assert_eq!(eval(&mut vm, source), "t");
}

/// The profiler charges an object's storage, not only its entry: copying
/// a populated hash table or a string with properties is charged for the
/// copy's table and property runs.
#[test]
fn copies_are_charged_their_storage() {
    let mut vm = Vm::new();
    let source = "(progn (setq pt-h (make-hash-table) pt-s (propertize (make-string 64 ?x) 'face 'bold))
                   (dotimes (i 1000) (puthash i i pt-h))
                   (defalias 'pt-copy-table (lambda () (copy-hash-table pt-h)))
                   (defalias 'pt-copy-string (lambda () (copy-sequence pt-s)))
                   (profiler-memory-start) (pt-copy-table) (pt-copy-string) (profiler-memory-stop)
                   (let ((table 0) (string 0))
                     (maphash (lambda (k v) (cond ((memq 'pt-copy-table (append k nil)) (setq table (+ table v)))
                                                  ((memq 'pt-copy-string (append k nil)) (setq string (+ string v)))))
                              (profiler-memory-log))
                     (list (>= table 24000) (> string 64) table string)))";
    let result = eval(&mut vm, source);
    assert!(result.starts_with("(t t "), "{result}");
}

/// An explicit closure keyed in a function-equal table is found after a
/// collection moves its body and not the closure.
#[test]
fn a_function_equal_key_survives_its_body_moving() {
    let mut vm = Vm::new();
    let source = r#"
      (progn
        (define-hash-table-test 'pt-fe2 #'function-equal #'ignore)
        (let* ((junk (make-list 1000 nil))
               (f (make-interpreted-closure '(x) (list (list 'car 'x)) '(t) nil nil))
               (h (make-hash-table :test 'pt-fe2)))
          (puthash f 99 h)
          (setq junk nil)
          (garbage-collect)
          (list junk (gethash f h 'missing))))"#;
    assert_eq!(eval(&mut vm, source), "(nil 99)");
}

/// `romacs-instructions' counts the evaluating thread's work: a longer
/// loop retires more instructions than a shorter one, and a repeat of the
/// same loop costs about the same. Where the kernel refuses counters it is
/// nil, never a made-up number.
#[test]
fn romacs_instructions_counts_the_threads_work() {
    let mut vm = Vm::new();
    let source = "(let ((cost (lambda (n) (let ((a (romacs-instructions)) (i 0)) (while (< i n) (setq i (1+ i))) (- (romacs-instructions) a)))))
                    (and (romacs-instructions) (list (funcall cost 1000) (funcall cost 10000) (funcall cost 10000))))";
    let value = vm.eval_source_raw(source).unwrap();
    let text = vm.display(value).unwrap();
    if text == "nil" {
        assert!(crate::host::thread_instructions().is_none(), "nil only without a counter");
        return;
    }
    let counts: Vec<f64> = text.trim_matches(['(', ')']).split(' ').map(|n| n.parse().unwrap()).collect();
    assert!(counts[1] > 5.0 * counts[0], "10x the loop is more work: {counts:?}");
    assert!((counts[2] / counts[1] - 1.0).abs() < 0.05, "the same loop costs the same: {counts:?}");
}

/// `romacs-native-stack-room' is the evaluating thread's remaining native
/// stack: less inside a nested Lisp recursion than at its top.
#[test]
fn romacs_native_stack_room_shrinks_inside_recursion() {
    let mut vm = Vm::new();
    let source = "(progn (defun room-at (n) (if (= n 0) (romacs-native-stack-room) (eval (list 'room-at (1- n)))))
                    (let ((top (romacs-native-stack-room)) (deep (room-at 40))) (list (integerp top) (< deep top))))";
    assert_eq!(eval(&mut vm, source), "(t t)");
}
