use romacs_vm::Vm;

#[test]
fn identity_preserves_objects_directly_and_through_aliases() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(condition-case e (let ((x (list 'kept))) (list (eq x (identity x)) (eq x (funcall 'identity x)) (identity nil) (identity 42))) (error e))").unwrap().display().unwrap();
    assert_eq!(value, "(t t nil 42)", "identity must be a callable GNU C primitive");
    vm.eval_source("(defalias 'identity-test-wrapper #'identity)").unwrap();
    assert_eq!(vm.eval_source("(identity-test-wrapper 'kept)").unwrap().display().unwrap(), "kept");
}

#[test]
fn identity_requires_exactly_one_argument() {
    let mut vm = Vm::new();
    for source in ["(identity)", "(identity 1 2)"] {
        let probe = format!("(condition-case e {source} (wrong-number-of-arguments (car e)) (error e))");
        assert_eq!(vm.eval_source(&probe).unwrap().display().unwrap(), "wrong-number-of-arguments", "GNU identity arity: {source}");
    }
}
