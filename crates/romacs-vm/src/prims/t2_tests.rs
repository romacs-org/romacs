use crate::Vm;

#[test]
fn generated_descriptors_point_to_human_declarations() {
    for descriptor in crate::prim::GENERATED {
        let file = std::path::Path::new(descriptor.file);
        let path = if file.is_absolute() {
            file.to_owned()
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file)
        };
        let source = std::fs::read_to_string(&path).expect("human declaration file exists");
        let line = source
            .lines()
            .nth(descriptor.line as usize - 1)
            .expect("declaration line exists");
        assert!(
            line.contains("defsubr!"),
            "{}:{}: {line}",
            descriptor.file,
            descriptor.line
        );
        assert!(
            line.contains(descriptor.name),
            "declaration names {}: {line}",
            descriptor.name
        );
        assert!(!descriptor.file.contains("generated_prims"));
    }
}

fn eval(vm: &mut Vm, source: &str) -> Result<String, String> {
    vm.eval_source_raw(source).and_then(|v| vm.display(v))
}

fn collecting_vm() -> Vm {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm
}

// Each callback runs past the 10,000-step budget, so the engine reaches a wait
// point inside it and, at collection floor 1, collects and compacts. The
// leading garbage puts every list built afterwards in a region that moves.
const CHURN: &str = "(let ((i 0)) (while (< i 12000) (setq i (1+ i)) (cons i i)))";
const GARBAGE: &str = "(let ((i 0)) (while (< i 3000) (setq i (1+ i)) (cons i i)))";

#[test]
fn t2_saved_values_are_forwarded_by_collections_inside_the_callback() {
    let mut vm = collecting_vm();
    let before = vm.heap_stats().collections;
    let source = format!(
        "{GARBAGE}
         (let* ((needle (list 'needle))
                (calls 0)
                (alist (list (cons (list 'a) 1) (cons (list 'b) 2)
                             (cons needle 3) (cons (list 'd) 4))))
           (list (romacs--t2-assoc needle alist
                                   (lambda (x k) (setq calls (1+ calls)) {CHURN} (eq x k)))
                 calls))"
    );
    assert_eq!(eval(&mut vm, &source), Ok("(((needle) . 3) 3)".into()));
    let collections = vm.heap_stats().collections - before;
    assert!(
        collections >= 3,
        "each callback must collect; saw {collections}"
    );
}

#[test]
fn t2_nil_testfn_stays_in_rust_and_misses_return_nil() {
    let mut vm = collecting_vm();
    for (source, expected) in [
        ("(romacs--t2-assoc 'b '((a . 1) x (b . 2)) nil)", "(b . 2)"),
        ("(romacs--t2-assoc 'z '((a . 1) (b . 2)) nil)", "nil"),
        (
            "(romacs--t2-assoc 3 '((1 . a) (2 . b)) (lambda (x k) (= x k)))",
            "nil",
        ),
        // GNU's TESTFN order is (car ELT) then KEY: the first car below 2.
        (
            "(romacs--t2-assoc 2 '((3 . c) (1 . a) (0 . z)) #'<)",
            "(1 . a)",
        ),
    ] {
        assert_eq!(eval(&mut vm, source), Ok(expected.into()), "{source}");
    }
}

#[test]
fn t2_callbacks_may_be_any_function_and_may_nest() {
    let mut vm = collecting_vm();
    for (source, expected) in [
        // A legacy primitive through its symbol.
        ("(romacs--t2-assoc 'b '((a . 1) (b . 2)) 'eq)", "(b . 2)"),
        // A closure with captures and a &rest closure.
        (
            "(let ((want 2)) (romacs--t2-assoc 0 '((1 . a) (2 . b)) (lambda (x _k) (= x want))))",
            "(2 . b)",
        ),
        (
            "(romacs--t2-assoc 2 '((1 . a) (2 . b)) (lambda (&rest xs) (apply #'= xs)))",
            "(2 . b)",
        ),
        // Through apply, from either side of the boundary.
        (
            "(apply #'romacs--t2-assoc 2 '(((1 . a) (2 . b)) eq))",
            "(2 . b)",
        ),
        (
            "(romacs--t2-assoc 2 '((1 . a) (2 . b)) (lambda (x k) (apply #'eq (list x k))))",
            "(2 . b)",
        ),
        // A T2 primitive called from inside another one's callback.
        (
            "(romacs--t2-assoc 2 '((1 . a) (2 . b))
               (lambda (x k) (romacs--t2-assoc x '((1 . p) (2 . q)) (lambda (_c x2) (= x2 k)))))",
            "(2 . b)",
        ),
        // A generated leaf as the callback reports its own arity.
        (
            "(condition-case e (romacs--t2-assoc 1 '((1 . a)) #'cdr-safe) (error e))",
            "(wrong-number-of-arguments #<subr cdr-safe> 2)",
        ),
    ] {
        assert_eq!(eval(&mut vm, source), Ok(expected.into()), "{source}");
    }
}

#[test]
fn t2_escapes_unwind_the_continuation_frame() {
    let mut vm = collecting_vm();
    // 600 iterations exceed the explicitly bound depth budget if a frame leaks.
    let source = "(let ((max-lisp-eval-depth 100) (n 0) (caught 0))
                    (while (< n 600)
                      (setq n (1+ n))
                      (condition-case nil
                          (romacs--t2-assoc 1 '((2 . a)) (lambda (_x k) (car k)))
                        (wrong-type-argument (setq caught (1+ caught))))
                      (catch 'out (romacs--t2-assoc 1 '((2 . a)) (lambda (_x _k) (throw 'out nil)))))
                    caught)";
    assert_eq!(eval(&mut vm, source), Ok("600".into()));
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (romacs--t2-assoc 1 '((2 . a)) (lambda (_x k) (car k))) (error e))"
        ),
        Ok("(wrong-type-argument listp 1)".into())
    );
}

// Optimized builds dispatch handlers by sibling calls, so a continuation that
// kept an owned Vec live across its dispatch would retain native frames per
// callback until the request ended. Debug builds use a driver loop instead.
#[test]
#[cfg(not(debug_assertions))]
fn t2_normal_returns_do_not_grow_the_native_stack() {
    let worker = std::thread::Builder::new().stack_size(512 * 1024).spawn(|| {
        let mut vm = Vm::new();
        eval(
            &mut vm,
            "(let ((i 0)) (while (< i 1000000) (romacs--t2-assoc 2 '((1 . a) (2 . b)) #'eq) (setq i (1+ i))) i)",
        )
    });
    assert_eq!(worker.unwrap().join().unwrap(), Ok("1000000".into()));
}

// The same bound for each callee kind t2_dispatch can enter: a symbol alias
// of a primitive, a named closure, and `apply` spreading onto a primitive.
// Optimized builds without LTO (the gate profile) are where a stray
// address-taken local in the dispatcher costs a native frame per callback.
#[test]
#[cfg(not(debug_assertions))]
fn t2_callback_kinds_do_not_grow_the_native_stack() {
    let worker = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut vm = Vm::new();
            let setup = "(defalias 't2-eq-alias #'eq) (defun t2-never (_x _k) nil)";
            let mut results = vec![eval(&mut vm, setup)];
            for call in [
                "(romacs--t2-assoc 2 '((1 . a) (2 . b)) #'t2-eq-alias)",
                "(romacs--t2-assoc 2 '((1 . a) (2 . b)) #'t2-never)",
                "(romacs--t2-assoc '(2 2) (list (cons #'eq 'a)) #'apply)",
            ] {
                let source = format!("(let ((i 0)) (while (< i 200000) {call} (setq i (1+ i))) i)");
                results.push(eval(&mut vm, &source));
            }
            results
        });
    let expected: Vec<Result<String, String>> = vec![
        Ok("t2-never".into()),
        Ok("200000".into()),
        Ok("200000".into()),
        Ok("200000".into()),
    ];
    assert_eq!(worker.unwrap().join().unwrap(), expected);
}

#[test]
fn t2_body_faults_and_arity_are_gnu_conditions() {
    let mut vm = collecting_vm();
    for (source, expected) in [
        (
            "(condition-case e (romacs--t2-assoc 1 '((2 . a) . 5) #'eq) (error e))",
            "(wrong-type-argument listp ((2 . a) . 5))",
        ),
        (
            "(condition-case e (romacs--t2-assoc 1) (error e))",
            "(wrong-number-of-arguments #<subr romacs--t2-assoc> 1)",
        ),
    ] {
        assert_eq!(eval(&mut vm, source), Ok(expected.into()), "{source}");
    }
}

#[test]
fn t2_unbounded_callback_recursion_fails_cleanly() {
    let mut vm = collecting_vm();
    let source = "(defun t2-deep (n) (romacs--t2-assoc n (list (cons n n)) (lambda (_x _k) (t2-deep (1+ n)))))
                  (condition-case nil (t2-deep 0) (error 'overflow))";
    assert_eq!(eval(&mut vm, source), Ok("overflow".into()));
    // Recursion only through continuation activations and the closures they
    // enter, so no ordinary call-site depth check intervenes.
    let source = "(setq t2-self (lambda (_x k) (romacs--t2-assoc k (list (cons k k)) t2-self)))
                  (condition-case nil (romacs--t2-assoc 1 '((1 . 1)) t2-self) (error 'overflow))";
    assert_eq!(eval(&mut vm, source), Ok("overflow".into()));
    assert_eq!(
        eval(&mut vm, "(romacs--t2-assoc 1 '((1 . ok)) #'eq)"),
        Ok("(1 . ok)".into())
    );
}

// `run-hooks' (prims/eval_hooks.rs) holds the hook symbol, the local tail, the
// global tail and the hook arguments not yet run in the continuation's
// registers. Every hook function below runs past the step budget, so each
// callback collects and compacts; the lists were built after the leading
// garbage, so they move. The walk must still visit every element in GNU's
// order (eval.c run_hook_with_args) and finish with the same objects. Live
// junk placed ahead of `rh-next' and dropped by the run request makes the
// first collection inside a hook move `rh-next' itself.
#[test]
fn run_hooks_walk_state_is_forwarded_by_collections_inside_hooks() {
    let mut vm = collecting_vm();
    let setup = format!(
        "{GARBAGE}
         (progn
           (set 'rh-log nil)
           (set 'rh-junk (let ((i 0) (j nil)) (while (< i 3000) (setq i (1+ i)) (setq j (cons i j))) j))
           (defalias 'rh-rec (lambda (tag) {CHURN} (set 'rh-log (cons tag (symbol-value 'rh-log)))))
           (set 'rh-h (list (lambda () (rh-rec 'g1)) t (lambda () (rh-rec 'g2))))
           (set 'rh-next (list (lambda () (rh-rec 'n1)) (lambda () (rh-rec 'n2))))
           (set 'rh-parts (list (symbol-value 'rh-next) (car (symbol-value 'rh-next))))
           nil)"
    );
    assert_eq!(eval(&mut vm, &setup), Ok("nil".into()));
    let next = format!("{:?}", vm.eval_source("rh-next").unwrap());
    let before = vm.heap_stats().collections;
    let out = eval(
        &mut vm,
        "(with-temp-buffer
           (set 'rh-junk nil)
           (make-local-variable 'rh-h)
           (set 'rh-h (list (lambda () (rh-rec 'l1)) t (lambda () (rh-rec 'l2))))
           (list (run-hooks 'rh-h 'rh-next 'rh-next) (nreverse (symbol-value 'rh-log))))",
    );
    assert_eq!(out, Ok("(nil (l1 g1 g2 l2 n1 n2 n1 n2))".into()));
    let collections = vm.heap_stats().collections - before;
    assert!(collections >= 8, "each hook function must collect; saw {collections}");
    let moved = format!("{:?}", vm.eval_source("rh-next").unwrap());
    eprintln!("rh-next: {next:?} -> {moved:?}");
    assert_ne!(next, moved, "the fixture must move the hook list");
    // The moved list is the same object, still holding the same closures.
    let same = "(let ((p (symbol-value 'rh-parts))) (list (eq (car p) rh-next) (eq (car (cdr p)) (car rh-next)) (length rh-next)))";
    assert_eq!(eval(&mut vm, same), Ok("(t t 2)".into()));
}

// GNU run_hook_with_args calls a FUNCTIONP value whole, calls a
// lambda-headed global value whole, and walks any other list, calling each
// element (here a lambda list) whole.  Walking a lambda list instead would
// call the symbol `lambda' and signal; each lambda records that it ran.
// Answers are pinned GNU 32.0.50's.
#[test]
fn run_hooks_calls_lambda_lists_whole() {
    let mut vm = Vm::new();
    vm.eval_source("(defvar rh-ran nil)").unwrap();
    for (source, expected) in [
        ("(progn (setq rh-ran nil) (set 'rh-h '(lambda () (setq rh-ran (cons 1 rh-ran))))
                 (list (condition-case e (run-hooks 'rh-h) (error e)) rh-ran))",
         "(nil (1))"),
        ("(progn (setq rh-ran nil) (set 'rh-g '(lambda () (setq rh-ran (cons 2 rh-ran))))
                 (with-temp-buffer (make-local-variable 'rh-g) (set 'rh-g (list t))
                   (list (condition-case e (run-hooks 'rh-g) (error e)) rh-ran)))",
         "(nil (2))"),
        ("(progn (setq rh-ran nil) (set 'rh-l (list '(lambda () (setq rh-ran (cons 3 rh-ran)))))
                 (list (condition-case e (run-hooks 'rh-l) (error e)) rh-ran))",
         "(nil (3))"),
    ] {
        assert_eq!(eval(&mut vm, source), Ok(expected.into()), "{source}");
    }
}
