//! R110: function-world publication and speculative-entry soundness.
use super::*;

fn text(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap();
    vm.display(value).unwrap()
}

#[test]
fn mid_loop_redefinition_rechecks_at_next_entry() {
    let mut vm = Vm::new();
    let result = text(
        &mut vm,
        "
        (defun epoch-g () 1)
        (let ((i 0) (sum 0))
          (while (< i 4)
            (setq sum (+ sum (epoch-g)))
            (if (= i 1) (defun epoch-g () 10))
            (setq i (1+ i)))
          sum)",
    );
    assert_eq!(result, "22", "next iteration must enter the NEW body");
    assert!(
        vm.speculative_hits > 0,
        "fixture never entered speculative code"
    );
    assert!(vm.speculative_misses > 0, "fixture never deoptimised");
}

#[test]
fn intervening_call_redefinition_rechecks_without_a_backedge() {
    let mut vm = Vm::new();
    let result = text(
        &mut vm,
        "
        (defvar epoch-change nil)
        (defun epoch-g () 1)
        (defun epoch-f () (if epoch-change (defun epoch-g () 42)))
        (defun epoch-run () (epoch-f) (epoch-g))
        (epoch-run)
        (epoch-run)
        (setq epoch-change t)
        (epoch-run)",
    );
    assert_eq!(result, "42", "g must redispatch after f redefines it");
    assert!(
        vm.speculative_hits > 0,
        "fixture never entered speculative code"
    );
    assert!(vm.speculative_misses > 0, "fixture never deoptimised");
}

#[test]
fn writers_invalidate_live_entries() {
    for writer in [
        "(defun epoch-g (x) (+ x 40))",
        "(defalias 'epoch-g (lambda (x) (+ x 40)))",
        "(fset 'epoch-g (lambda (x) (+ x 40)))",
        "(defmacro epoch-g (x) x)",
    ] {
        let mut vm = Vm::new();
        text(
            &mut vm,
            "(defun epoch-g (x) x) (defun epoch-run () (epoch-g 2)) (epoch-run) (epoch-run)",
        );
        assert!(vm.speculative_hits > 0);
        let before = vm.world_version;
        vm.eval_source_raw(writer).unwrap();
        assert!(
            vm.world_version > before,
            "writer missed publication: {writer}"
        );
        let result = vm.eval_source_raw("(epoch-run)").and_then(|v| vm.display(v));
        if writer.contains("defmacro") {
            assert!(
                result
                    .as_ref()
                    .is_err_and(|e| e.contains("invalid function")),
                "stale ordinary body entered after macro definition: {result:?}"
            );
            assert_eq!(text(&mut vm, "(epoch-g 42)"), "42");
            text(&mut vm, "(defmacro epoch-g (x) (list '+ x 1))");
            assert_eq!(text(&mut vm, "(epoch-g 42)"), "43");
        } else {
            assert_eq!(result, Ok("42".into()), "stale body after {writer}");
        }
        // A macro cell is rejected by ResolveHead before Prepared can enter
        // (or deoptimize) the old ordinary call site. The exact error above
        // proves that no stale body ran; ordinary writers still must deopt.
        if !writer.contains("defmacro") {
            assert!(vm.speculative_misses > 0, "no deopt after {writer}");
        }
    }
}

#[test]
fn deopt_preserves_heap_arguments_captures_and_arity() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    text(
        &mut vm,
        "
        (defun epoch-g (x) x)
        (defun epoch-run (x) (epoch-g x))
        (epoch-run '(7 8)) (epoch-run '(7 8))",
    );
    assert!(vm.speculative_hits > 0);
    text(
        &mut vm,
        "(let ((captured (list 40)))
        (defalias 'epoch-g (lambda (x &optional y &rest rest)
           (list captured x y rest))))",
    );
    assert_eq!(text(&mut vm, "(epoch-run '(7 8))"), "((40) (7 8) nil nil)");
    assert!(vm.speculative_misses > 0);
    text(&mut vm, "(defun epoch-g (x y) (+ x y))");
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil (epoch-run '(7 8))
         (wrong-number-of-arguments 'arity-caught))"
        ),
        "arity-caught"
    );
    text(&mut vm, "(defun epoch-g (x) x)");
    assert_eq!(text(&mut vm, "(epoch-run '(9 10))"), "(9 10)");
    let before = vm.heap_stats().collections;
    assert_eq!(
        text(
            &mut vm,
            "(let ((i 0) (v nil))
        (while (< i 20050) (setq v (epoch-run (list i))) (setq i (1+ i))) v)"
        ),
        "(20049)"
    );
    assert!(
        vm.heap_stats().collections > before,
        "moving collector was not exercised"
    );
    assert!(vm.execution.registers.iter().all(|v| *v == Value::NIL));
}

#[test]
fn fset_publishes_and_returns_the_definition() {
    let mut vm = Vm::new();
    let result = vm
        .eval_source_raw("(let ((f (lambda (x) (+ x 7)))) (eq (fset 'epoch-g f) f))")
        .and_then(|v| vm.display(v));
    assert_eq!(
        result,
        Ok("t".into()),
        "fset must publish and return its definition"
    );
}

#[test]
fn every_definition_publishes_an_epoch() {
    let mut vm = Vm::new();
    for source in [
        "(defun epoch-g (x) x)",
        "(defalias 'epoch-g (lambda (x) (+ x 1)))",
        "(fset 'epoch-g (lambda (x) (+ x 2)))",
        "(defmacro epoch-m (x) x)",
        "(defmacro epoch-m (x) (list 'quote x))",
    ] {
        let before = vm.world_version;
        vm.eval_source_raw(source).unwrap();
        assert!(vm.world_version > before, "no publication: {source}");
    }
}
