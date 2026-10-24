//! Activation boundaries, pending exits, and moving-GC control roots.
//! Characterization cases pass before and after the side-stack refactor.

use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).expect(source);
    value.display().expect("display characterization result")
}

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let result = vm.eval_source(source).and_then(|value| value.display());
    assert_eq!(result, Ok(expected.to_owned()), "{source}");
}

#[test]
fn multiframe_throw_skips_each_callees_own_handlers() {
    check(
        "(defun r113-leaf ()
           (condition-case nil
               (catch 'leaf-only (throw 'destination 42))
             (error 'wrong-leaf-handler)))
         (defun r113-middle ()
           (condition-case nil
               (catch 'middle-only (r113-leaf))
             (error 'wrong-middle-handler)))
         (defun r113-caller () (r113-middle))
         (vector (catch 'destination (r113-caller))
                 (catch 'destination 73))",
        "[42 73]",
    );
}

#[test]
fn inner_condition_handler_receives_signal_from_several_frames() {
    check(
        "(defvar r113-outer-hit nil)
         (defun r113-signal-leaf () (signal 'arith-error '(17)))
         (defun r113-signal-middle () (r113-signal-leaf))
         (defun r113-signal-inner ()
           (condition-case e (r113-signal-middle)
             (arith-error (list 'inner e))))
         (defun r113-signal-caller () (r113-signal-inner))
         (vector
           (condition-case nil (r113-signal-caller)
             (error (setq r113-outer-hit t)))
           r113-outer-hit)",
        "[(inner (arith-error 17)) nil]",
    );
}

#[test]
fn unwind_cleanup_order_crosses_frames_and_calls_cleanup_function() {
    check(
        "(defvar r113-log nil)
         (defun r113-record (item)
           (setq r113-log (cons item r113-log))
           'cleanup-result)
         (defun r113-order-leaf ()
           (unwind-protect (throw 'done 42) (r113-record 'leaf)))
         (defun r113-order-middle ()
           (unwind-protect
               (unwind-protect (r113-order-leaf) (r113-record 'inner))
             (r113-record 'middle)))
         (vector
           (catch 'done
             (unwind-protect (r113-order-middle) (r113-record 'caller)))
           r113-log)",
        "[42 (caller middle inner leaf)]",
    );
}

#[test]
fn caller_handler_survives_same_destination_callee_siblings() {
    // Both zero-argument functions start with condition-case, so the compiler
    // assigns their condition payloads the same local register index. Selecting
    // the callee's first clause must remove its error sibling, not the caller's.
    check(
        "(defun r113-boundary-callee ()
           (condition-case nil (signal 'arith-error nil)
             (arith-error (signal 'file-error '(callee-handler)))
             (error 'wrong-callee-sibling)))
         (defun r113-boundary-caller ()
           (condition-case e (r113-boundary-callee)
             (file-error (list 'caller e))
             (error 'wrong-caller-sibling)))
         (r113-boundary-caller)",
        "(caller (file-error callee-handler))",
    );
}

#[test]
fn resume_normal_preserves_value_across_cleanup_calls() {
    check(
        "(defvar r113-normal-log nil)
         (defun r113-normal-cleanup ()
           (setq r113-normal-log (cons 'called r113-normal-log))
           999)
         (defun r113-normal-body ()
           (unwind-protect
               (unwind-protect (list 'protected 42) (r113-normal-cleanup))
             (r113-normal-cleanup)))
         (vector (r113-normal-body) r113-normal-log)",
        "[(protected 42) (called called)]",
    );
}

#[test]
fn resume_throw_survives_cleanup_with_its_own_catch_and_protect() {
    check(
        "(defvar r113-resume-log nil)
         (defun r113-resume-cleanup ()
           (unwind-protect (catch 'local (throw 'local 99))
             (setq r113-resume-log (cons 'cleanup r113-resume-log))))
         (defun r113-resume-body ()
           (unwind-protect (throw 'done '(original 42))
             (r113-resume-cleanup)))
         (vector (catch 'done (r113-resume-body)) r113-resume-log)",
        "[(original 42) (cleanup)]",
    );
}

#[test]
fn cleanup_throw_overrides_pending_throw_and_normal_completion() {
    check(
        "(defun r113-override-cleanup () (throw 'replacement 23))
         (vector
           (catch 'replacement
             (catch 'original
               (unwind-protect (throw 'original 11)
                 (r113-override-cleanup))))
           (catch 'replacement
             (unwind-protect 11 (r113-override-cleanup))))",
        "[23 23]",
    );
}

#[test]
fn cleanup_signal_overrides_pending_throw_and_original_diagnostic() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            "(defun r113-error-cleanup () (signal 'file-error '(cleanup)))
             (condition-case e
                 (catch 'original
                   (unwind-protect (throw 'original 11)
                     (r113-error-cleanup)))
               (file-error e))",
        ),
        "(file-error cleanup)"
    );
    // A primitive failure carries an engine diagnostic as well as its signal.
    // It must not leak through after cleanup replaces that pending failure.
    let error = vm
        .eval_source("(unwind-protect (car 7) (r113-error-cleanup))")
        .unwrap_err();
    assert!(error.contains("file-error"), "{error}");
    assert_eq!(eval(&mut vm, "(+ 20 22)"), "42");
}

#[test]
fn unhandled_error_runs_cleanup_and_preserves_primitive_diagnostic() {
    let mut vm = Vm::new();
    eval(
        &mut vm,
        "(defvar r113-error-log nil)
         (defun r113-error-leaf ()
           (unwind-protect (car 7)
             (setq r113-error-log (cons 'leaf r113-error-log))))
         (defun r113-error-caller ()
           (unwind-protect (r113-error-leaf)
             (setq r113-error-log (cons 'caller r113-error-log))))",
    );
    let direct = vm.eval_source("(car 7)").unwrap_err();
    let unwound = vm.eval_source("(r113-error-caller)").unwrap_err();
    assert_eq!(unwound, direct);
    assert_eq!(eval(&mut vm, "r113-error-log"), "(caller leaf)");
    assert_eq!(eval(&mut vm, "(+ 20 22)"), "42");
}

#[test]
fn missing_catch_signals_with_payload_runs_cleanup_and_leaves_vm_reusable() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            "(defvar r113-no-catch-log nil)
             (defun r113-no-catch-leaf ()
               (unwind-protect (throw 'missing 19)
                 (setq r113-no-catch-log
                   (cons 'cleanup r113-no-catch-log))))
             (condition-case e (r113-no-catch-leaf) (no-catch e))",
        ),
        "(no-catch missing 19)"
    );
    assert_eq!(eval(&mut vm, "r113-no-catch-log"), "(cleanup)");
    let error = vm.eval_source("(r113-no-catch-leaf)").unwrap_err();
    assert!(error.contains("no-catch"), "{error}");
    assert_eq!(eval(&mut vm, "r113-no-catch-log"), "(cleanup cleanup)");
    assert_eq!(eval(&mut vm, "(catch 'missing 42)"), "42");
}

#[test]
fn forced_collection_during_nested_cleanup_preserves_live_regions() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let function = vm
        .compile_source(
            "(defun r113-gc-work ()
           (let ((i 0))
             (while (< i 30000) (cons i nil) (setq i (1+ i))))
           (condition-case e (signal 'arith-error '(cleanup)) (arith-error e)))
         (defun r113-gc-leaf (tag)
           (unwind-protect (throw tag '(payload 42)) (r113-gc-work)))
         (defun r113-gc-middle (tag) (r113-gc-leaf tag))
         (let ((tag (list 'live-tag)))
           (condition-case nil
               (catch tag (r113-gc-middle tag))
             (error 'wrong-handler)))",
        )
        .unwrap();
    let before = vm.heap_stats();
    let result = vm.execute(function).and_then(|v| v.display());
    assert_eq!(result, Ok("(payload 42)".into()));
    let after = vm.heap_stats();
    assert!(
        after.collections > before.collections,
        "no execution-time collection"
    );
    assert!(
        after.reclaimed > before.reclaimed,
        "no execution-time compaction"
    );
}

#[test]
fn deep_recursion_returns_after_collection_with_caller_handlers_live() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let function = vm
        .compile_source(
            "(defun r113-deep-safe (n tag)
           (catch tag
             (if (< n 1)
                 (let ((i 0))
                   (while (< i 30000) (cons i nil) (setq i (1+ i)))
                   0)
               (1+ (r113-deep-safe (1- n) tag)))))
         (let ((tag (list 'deep-tag)))
           (vector (r113-deep-safe 400 tag) (catch tag (throw tag 42))))",
        )
        .unwrap();
    let before = vm.heap_stats();
    let result = vm.execute(function).and_then(|v| v.display());
    assert_eq!(result, Ok("[400 42]".into()));
    let after = vm.heap_stats();
    assert!(after.collections > before.collections);
    assert!(after.reclaimed > before.reclaimed);
}

#[test]
fn recursion_depth_failure_unwinds_every_entered_protect_and_binding() {
    let mut vm = Vm::new();
    eval(
        &mut vm,
        "(defvar r113-depth-entered 0)
         (defvar r113-depth-cleaned 0)
         (defvar r113-depth-special 'outside)
         (defun r113-depth-loop (r113-depth-special)
           (setq r113-depth-entered (1+ r113-depth-entered))
           (unwind-protect (r113-depth-loop (1+ r113-depth-special))
             (setq r113-depth-cleaned (1+ r113-depth-cleaned))))",
    );
    let error = vm.eval_source("(r113-depth-loop 0)").unwrap_err();
    assert!(error.contains("depth"), "{error}");
    // Do not freeze the implementation's numeric depth limit; characterize its
    // bounded failure, per-activation cleanup, and restoration instead.
    assert_eq!(
        eval(
            &mut vm,
            "(vector (< 1 r113-depth-entered)
                     (= r113-depth-entered r113-depth-cleaned)
                     r113-depth-special
                     (catch 'after-depth 42))",
        ),
        "[t t outside 42]"
    );
}

/// Round-trip region discipline across 100 activations that enter and exit
/// regions both normally and by unwinding.
///
/// WHAT THE MUTANTS ACTUALLY SHOWED, because the first version of this comment
/// claimed more than the evidence supports:
///
/// * `Op::PopHandlers` mutated from `truncate(keep)` to `truncate(keep + 1)`,
///   so a region survives its normal exit: **this test dies**, at the first
///   `eval` -- i.e. through `ret`'s `"invalid image return through control
///   region"` invariant, not through the final `throw` assertion below.
/// * That same mutation WITH `ret`'s invariant removed: **this test survives.**
///
/// So the final `(throw 'r118-stale ...)` assertion is NOT a qualified leak
/// detector; a leaked `Catch` region did not intercept it. It is kept as a
/// backstop and labelled unproven rather than deleted, because deleting it
/// would lose a cheap check, and dressing it up as protection it does not give
/// would be worse than either.
///
/// What this test genuinely protects: a `PopHandlers` regression, caught via the
/// return invariant, together with the characterisation that 100 mixed
/// normal/unwinding activations leave the machine answering correctly. The
/// caller-consumption direction is covered by
/// `malformed_callee_region_ops_cannot_consume_caller_controls` and
/// `normal_escape_cannot_cross_activation_floor`, both of which are qualified.
#[test]
fn returning_callees_leak_no_regions_onto_the_shared_stack() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            "(defvar r118-cleanups 0)
             (defun r118-establishes ()
               (catch 'r118-stale
                 (unwind-protect 42
                   (setq r118-cleanups (1+ r118-cleanups)))))
             (defun r118-unwinds ()
               (catch 'r118-stale
                 (unwind-protect (throw 'r118-stale 7)
                   (setq r118-cleanups (1+ r118-cleanups)))))
             (defun r118-many (n)
               (let ((i 0))
                 (while (< i n)
                   (r118-establishes)
                   (r118-unwinds)
                   (setq i (1+ i)))
                 i))
             (r118-many 50)",
        ),
        "50"
    );
    // Both bodies really ran, so the loop is not vacuous.
    assert_eq!(eval(&mut vm, "r118-cleanups"), "100");
    // Backstop, explicitly NOT qualified by a mutant: a deliberately leaked
    // region did not intercept this throw (see the comment above). Retained
    // because it is nearly free and would catch a coarser regression.
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case nil (throw 'r118-stale 7) (no-catch 'propagated))"
        ),
        "propagated"
    );
    // And the machine still catches what it should, so nothing was over-truncated.
    assert_eq!(eval(&mut vm, "(catch 'r118-stale (throw 'r118-stale 11))"), "11");
}
