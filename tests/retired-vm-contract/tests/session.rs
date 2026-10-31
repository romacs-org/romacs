use romacs_vm::Vm;

fn evaluate(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).expect(source);
    value.display().expect("display request result")
}

#[test]
fn one_vm_owns_successive_evaluation_requests() {
    let mut vm = Vm::new();
    assert_eq!(evaluate(&mut vm, "(setq wire1 2)"), "2");
    assert_eq!(evaluate(&mut vm, "(+ wire1 1)"), "3");
    assert_eq!(evaluate(&mut vm, "(selected-frame)"), "#<frame initial>");
    assert_eq!(evaluate(&mut vm, "(fboundp 'recursive-edit)"), "t");
}

#[test]
fn message_formats_session_values_and_frame_identity() {
    let mut vm = Vm::new();
    evaluate(&mut vm, "(setq wire1 2)");
    assert_eq!(evaluate(&mut vm, "(message \"%s\" (+ wire1 1))"), "\"3\"");
    // print.c PVEC_FRAME: its name and address, as prin1 prints it.
    let frame = evaluate(&mut vm, "(message \"%s\" (selected-frame))");
    assert!(frame.starts_with("\"#<frame F1 0x") && frame.ends_with(">\""), "{frame}");
    assert_eq!(frame, evaluate(&mut vm, "(prin1-to-string (selected-frame))"));
    assert_eq!(
        evaluate(&mut vm, "(message \"%s\" (fboundp 'recursive-edit))"),
        "\"t\""
    );
}

#[test]
fn lisp_error_stops_remaining_source_forms_and_preserves_prior_state() {
    let mut vm = Vm::new();
    evaluate(&mut vm, "(setq reached 'before)");
    assert_eq!(
        vm.eval_source("(car 1) (setq reached 'after)").unwrap_err(),
        "wrong value type (expected tag 3)"
    );
    assert_eq!(evaluate(&mut vm, "reached"), "before");
    assert_eq!(evaluate(&mut vm, "(+ 2 1)"), "3");
}

#[test]
fn heap_backed_session_state_survives_request_collections() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    evaluate(&mut vm, "(setq retained (list \"session\" 2))");
    let before = vm.heap_stats().collections;
    for _ in 0..16 {
        evaluate(
            &mut vm,
            "(let ((n 0)) (while (< n 256) (cons n n) (setq n (1+ n))))",
        );
    }
    assert!(vm.heap_stats().collections > before);
    assert_eq!(evaluate(&mut vm, "(car retained)"), "\"session\"");
    assert_eq!(evaluate(&mut vm, "(+ (car (cdr retained)) 1)"), "3");
}
