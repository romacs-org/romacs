//! Semantic guards for the inherited opaque-closure observation pins.
//! These are bounded VM2 contracts, not closure-slot/GNU parity: VM2-CLOSEQ1
//! and VM2-FR5-1 remain open. No function body is retained for printing.
use romacs_vm::Vm;

fn check(source: &str, expected: &str, contract: &str) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    let result = vm.eval_source(source).unwrap().display().unwrap();
    assert_eq!(result, expected, "{contract}: {source}");
}

#[test]
fn closure_conditions_preserve_kind_predicate_and_offending_identity() {
    // reverse agrees on the condition/predicate, but not the printed datum.
    check(
        "(let ((f (lambda (x) x))) (condition-case e (reverse f) (error (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) f) (cdr (cdr (cdr e)))))))",
        "(wrong-type-argument sequencep t nil)",
        "CLOSURE_REVERSE_CONDITION",
    );
    check(
        "(let ((f (lambda () 1))) (condition-case e (call-interactively f) (error (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) f) (cdr (cdr (cdr e)))))))",
        "(wrong-type-argument commandp t nil)",
        "CLOSURE_COMMAND_CONDITION",
    );
}

#[test]
fn closure_sequences_keep_the_exact_early_refusal_without_callbacks() {
    // GNU walks closure slots here. VM2 instead refuses before invoking any
    // callback, even a missing/invalid/wrong-arity one. Do not call this printing.
    for operation in [
        "(vconcat f)",
        "(mapc (lambda (x) (setq calls (1+ calls))) f)",
        "(mapcan (lambda (x) (setq calls (1+ calls)) x) f)",
        "(mapcan (lambda (x) (setq calls (1+ calls)) (list x)) f)",
        "(mapcan #'list f)", "(mapcan 'car f)", "(mapcan #'1+ f)",
        "(mapcan nil f)", "(mapcan 'primh-map-no-such-fn f)",
        "(mapcan (lambda () 1) f)", "(mapcan 5 f)",
    ] {
        check(
            &format!("(let ((f (lambda (x) x)) (calls 0)) (condition-case e {operation} (error (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) f) (cdr (cdr (cdr e))) calls))))"),
            "(wrong-type-argument sequencep t nil 0)",
            "CLOSURE_SEQUENCE_EARLY_REFUSAL",
        );
    }
}

#[test]
fn call_interactively_preserves_callback_function_and_argument_identity() {
    check(
        "(let ((f (lambda (&rest a) (interactive \"p\") a)) (calls 0)) (defalias 'funcall-interactively (lambda (g &rest a) (setq calls (1+ calls)) (list (eq g f) a))) (list (call-interactively f) calls))",
        "((t (1)) 1)",
        "CLOSURE_INTERACTIVE_PREFIX_CALLBACK",
    );
    check(
        "(let* ((arg (cons 'owned nil)) (f (lambda (x) (interactive (list arg)) x)) (calls 0)) (defalias 'funcall-interactively (lambda (g &rest a) (setq calls (1+ calls)) (list (eq g f) (eq (car a) arg) (cdr a)))) (list (call-interactively f) calls))",
        "((t t nil) 1)",
        "CLOSURE_INTERACTIVE_ARGUMENT_IDENTITY",
    );
}

#[test]
fn closure_printers_preserve_return_identity_and_exact_newline_framing() {
    for (printer, text) in [
        ("prin1", "#<interpreted-function>"),
        ("princ", "#<interpreted-function>"),
        ("print", "\n#<interpreted-function>\n"),
    ] {
        let chars: Vec<_> = text.bytes().map(|c| c.to_string()).collect();
        check(
            &format!("(let ((f (lambda (x) x)) (out nil)) (let ((r ({printer} f (lambda (c) (setq out (cons c out)))))) (list (eq r f) (nreverse out))))"),
            &format!("(t ({}))", chars.join(" ")),
            "CLOSURE_PRINT_RETURN_AND_FRAMING",
        );
    }
    let chars: Vec<_> = b"#<interpreted-function>".iter().map(|c| c.to_string()).collect();
    check(
        "(append (prin1-to-string (lambda (x) x)) nil)",
        &format!("({})", chars.join(" ")),
        "CLOSURE_PRIN1_STRING",
    );
}
