use romacs_vm::Vm;

#[test]
fn generated_registration_preserves_calling_printing_and_arity() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, expected) in [
        ("(cdr-safe '(1 2 3))", "(2 3)"),
        ("(funcall #'take 2 '(1 2 3))", "(1 2)"),
        ("(apply #'nthcdr '(1 (2 3)))", "(3)"),
        ("(condition-case e (cdr-safe) (error e))", "(wrong-number-of-arguments #<subr cdr-safe> 0)"),
        ("(condition-case e (nthcdr 'bad nil) (error e))", "(wrong-type-argument integerp bad)"),
    ] {
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual, Ok(expected.to_owned()), "generated registry: {source}");
    }
}
