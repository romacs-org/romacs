use romacs_vm::Vm;

#[test]
fn byte_code_function_p_rejects_non_bytecode_objects() {
    let mut vm = Vm::new();
    for form in [
        "(byte-code-function-p nil)",
        "(byte-code-function-p t)",
        "(byte-code-function-p 42)",
        "(byte-code-function-p 1.5)",
        "(byte-code-function-p 'car)",
        "(byte-code-function-p (symbol-function 'car))",
        "(byte-code-function-p \"code\")",
        "(byte-code-function-p [0 \"code\" [] 1])",
        "(byte-code-function-p '(lambda (x) x))",
        "(byte-code-function-p (lambda (x) x))",
        "(byte-code-function-p (make-hash-table))",
        "(byte-code-function-p (current-buffer))",
        "(funcall 'byte-code-function-p nil)",
    ] {
        assert_eq!(
            vm.eval_source(form).and_then(|v| v.display()),
            Ok("nil".into()),
            "non-bytecode object must return nil: {form}"
        );
    }
    assert_eq!(vm.eval_source("(list (fboundp 'byte-code-function-p) (functionp 'byte-code-function-p))").unwrap().display().unwrap(), "(t t)");
}

#[test]
fn gnu_bytecode_representation_remains_unavailable() {
    let mut vm = Vm::new();
    assert!(vm.eval_source("(read-from-string \"#[0 \\\"\\\\300\\\\207\\\" [42] 1]\")").is_err(),
        "GNU bytecode reader support requires replacing the negative-only predicate");
    assert_eq!(vm.eval_source("(fboundp 'make-byte-code)").unwrap().display().unwrap(), "nil",
        "GNU bytecode production requires replacing the negative-only predicate");
}

#[test]
fn byte_code_function_p_arity_is_exactly_one() {
    let mut vm = Vm::new();
    for form in ["(apply 'byte-code-function-p nil)", "(apply 'byte-code-function-p '(nil nil))"] {
        let wrapped = format!("(condition-case e {form} (wrong-number-of-arguments (car e)))");
        assert_eq!(vm.eval_source(&wrapped).unwrap().display().unwrap(), "wrong-number-of-arguments");
    }
}
