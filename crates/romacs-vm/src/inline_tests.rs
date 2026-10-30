//! Speculative inlining must be observable only as eliminated call machinery.
use super::*;

fn text(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap();
    vm.display(value).unwrap()
}

fn rejects_definition(vm: &Vm) {
    let target = vm.slots[vm.names["in-g"]].function;
    let function = vm.closures[target.payload() as usize].function;
    assert!(
        inline_body(vm, function).is_none(),
        "ineligible body admitted"
    );
}

#[test]
fn trivial_calls_eliminate_frames() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(defun in-g (x) (1+ x)) (defun in-run (x) (in-g x)) (in-run 1) (in-run 2)"
        ),
        "3"
    );
    assert!(vm.inline_hits > 0, "qualifying body did not run inline");
}

#[test]
fn inline_mid_loop_redefinition() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(defun in-g () 1)
      (let ((i 0) (sum 0)) (while (< i 6)
        (setq sum (+ sum (in-g)))
        (if (= i 2) (fset 'in-g (lambda () 10)))
        (setq i (1+ i))) sum)"
        ),
        "33",
        "next iteration must enter new body"
    );
    assert!(vm.inline_hits > 0, "fixture did not inline");
    assert!(vm.speculative_misses > 0);
}

#[test]
fn inline_intervening_call_redefinition() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(defvar in-change nil) (defun in-g () 1)
      (defun in-f () (if in-change (fset 'in-g (lambda () 42))))
      (defun in-run () (progn (in-f) (in-g)))
      (in-run) (in-run) (setq in-change t) (in-run)"
        ),
        "42",
        "g must redispatch after f"
    );
    assert!(vm.inline_hits > 0, "fixture did not inline");
    assert!(vm.speculative_misses > 0);
}

#[test]
fn captures_are_not_inlined() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(let ((x 40)) (fset 'in-g (lambda (y) (+ x y))))
      (in-g 1) (in-g 2)"
        ),
        "42"
    );
    rejects_definition(&vm);
    assert_eq!(vm.inline_hits, 0, "captured callee was inlined");
}

#[test]
fn inline_arity_mismatch_falls_back() {
    let mut vm = Vm::new();
    text(
        &mut vm,
        "(defun in-g (x) (1+ x)) (defun in-run () (in-g 1)) (in-run) (in-run)",
    );
    assert!(vm.inline_hits > 0, "fixture did not inline");
    let before = vm.inline_hits;
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil (in-g) (wrong-number-of-arguments 'caught))"
        ),
        "caught"
    );
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil (in-g 1 2) (wrong-number-of-arguments 'caught))"
        ),
        "caught"
    );
    assert_eq!(vm.inline_hits, before, "wrong arity entered inline body");
    text(&mut vm, "(defun in-g (x y) x)");
    // Same warmed call site: first miss refreshes the descriptor, second call
    // exercises cached arity admission (not just the cold arity check).
    for _ in 0..2 {
        assert_eq!(
            text(
                &mut vm,
                "(condition-case nil (in-run) (wrong-number-of-arguments 'caught))"
            ),
            "caught",
            "cached arity mismatch must fall back"
        );
    }
    assert_eq!(vm.inline_hits, before);
}

#[test]
fn control_regions_are_not_inlined() {
    for body in [
        "(catch 'tag 7)",
        "(condition-case nil 7 (error 9))",
        "(unwind-protect 7 (setq in-clean t))",
    ] {
        let mut vm = Vm::new();
        assert_eq!(
            text(
                &mut vm,
                &format!("(defvar in-clean nil) (defun in-g () {body}) (in-g) (in-g)")
            ),
            "7"
        );
        rejects_definition(&vm);
        assert_eq!(vm.inline_hits, 0, "control body was inlined: {body}");
        if body.contains("unwind-protect") {
            assert_eq!(text(&mut vm, "in-clean"), "t");
        }
    }
}

#[test]
fn dynamic_binding_is_not_inlined() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(defvar in-special 1) (defun in-g () (let ((in-special 7)) in-special)) (in-g) (in-g)"
        ),
        "7"
    );
    rejects_definition(&vm);
    assert_eq!(vm.inline_hits, 0, "dynamic binding was inlined");
    assert_eq!(text(&mut vm, "in-special"), "1");
}

#[test]
fn inline_and_generic_agree() {
    for (args, body, call) in [
        ("x", "x", "'(a b)"),
        ("", "17", ""),
        ("x", "(1+ x)", "41"),
        ("x", "(1- x)", "43"),
        ("x y", "(+ x y)", "20 22"),
        ("x y", "(- x y)", "50 8"),
        ("x y", "(* x y)", "6 7"),
        ("x y", "(< x y)", "6 7"),
        ("x y", "(= x y)", "7 7"),
        ("x", "(null x)", "nil"),
    ] {
        let source = format!(
            "(defun in-g ({args}) {body}) (defun in-run () (in-g {call})) (in-run) (in-run)"
        );
        let mut inline = Vm::new();
        let mut generic = Vm::new();
        generic.disable_inline = true;
        assert_eq!(
            text(&mut inline, &source),
            text(&mut generic, &source),
            "{source}"
        );
        assert!(inline.inline_hits > 0, "fixture did not inline: {source}");
        assert_eq!(generic.inline_hits, 0);
    }
}

#[test]
fn inline_survives_forced_moving_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    text(
        &mut vm,
        "(defun in-g (x) x) (defun in-run (x) (in-g x)) (in-run '(7)) (in-run '(8))",
    );
    assert!(vm.inline_hits > 0, "fixture did not inline");
    // Force a dead prefix and prove the surviving argument moved, rather than
    // merely assuming a low collection threshold entails compaction.
    for _ in 0..100 {
        vm.conses.push((Value::NIL, Value::NIL));
    }
    let old = vm.list_value(&[Value::integer(42).unwrap()]);
    let slot = vm.slot("in-root");
    vm.slots[slot].value = old;
    let before = vm.heap_stats().collections;
    assert_eq!(
        text(
            &mut vm,
            "(let ((i 0) (v nil)) (while (< i 20050)
      (setq v (in-run in-root))
      (if (eq v in-root) nil (setq i 20050 v 'stale))
      (setq i (1+ i))) v)"
        ),
        "(42)"
    );
    assert!(
        vm.heap_stats().collections > before,
        "collector did not run"
    );
    assert_ne!(vm.slots[slot].value, old, "live argument did not move");
    assert!(
        vm.inline_hits > 10000,
        "inlining was not active across collection"
    );
    assert!(vm.execution.registers.iter().all(|v| *v == Value::NIL));
}

#[test]
fn inline_sealed_primitive_ignores_redefinition() {
    // (1+ x) is GNU's Badd1: the inlined body stays valid across the
    // redefinition of `1+' (VM2-SEALED1), as the compiled caller does in GNU.
    let mut vm = Vm::new();
    text(
        &mut vm,
        "(defun in-g (x) (1+ x)) (defun in-run () (in-g 2)) (in-run) (in-run)",
    );
    assert!(vm.inline_hits > 0);
    let hits = vm.inline_hits;
    assert_eq!(
        text(&mut vm, "(fset '1+ (lambda (x) 99)) (in-run) (in-run)"),
        "3"
    );
    assert!(vm.inline_hits > hits, "the sealed body must still be inlined");
}

#[test]
fn inline_type_and_overflow_bailouts_preserve_generic_results() {
    for call in ["2.5", "'bad", "most-positive-fixnum"] {
        let source = format!(
            "(defun in-g (x) (1+ x)) (defun in-run (x) (in-g x)) (in-run 1) (in-run 2)
          (condition-case e (in-run {call}) (error (car e)))"
        );
        let mut inline = Vm::new();
        let mut generic = Vm::new();
        generic.disable_inline = true;
        assert_eq!(
            text(&mut inline, &source),
            text(&mut generic, &source),
            "{call}"
        );
        assert!(inline.inline_hits > 0);
    }
}

#[test]
fn inline_rejects_heap_literals_optional_rest_and_nested_calls() {
    for definition in [
        "(defun in-g () '(7 8))",
        "(defun in-g () \"literal\")",
        "(defun in-g (&optional x) x)",
        "(defun in-g (&rest x) x)",
        "(defun in-g (x) (in-other x))",
        "(defun in-g (x) (1+ (1+ (1+ x))))",
    ] {
        let mut vm = Vm::new();
        text(&mut vm, definition);
        rejects_definition(&vm);
    }
}
