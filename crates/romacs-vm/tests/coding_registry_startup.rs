use romacs_vm::Vm;

#[test]
fn coding_variables_exist_before_first_primitive() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(boundp 'coding-system-list)", "t"),
        ("(list (car coding-system-list) (car coding-category-list))", "(undecided coding-category-iso-7)"),
        ("(progn (setq coding-category-utf-8 'kept) (coding-system-p 'no-conversion) coding-category-utf-8)", "kept"),
        ("(progn (put 'coding-system-error 'error-message \"kept\") (coding-system-p 'no-conversion) (get 'coding-system-error 'error-message))", "\"kept\""),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}

#[test]
fn first_dynamic_binding_does_not_erase_seed_registry() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source("(list (let ((coding-system-list 'local)) (coding-system-p 'no-conversion) coding-system-list) (car coding-system-list))").unwrap().display().unwrap(), "(local undecided)");
}
