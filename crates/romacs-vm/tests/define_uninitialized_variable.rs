use romacs_vm::Vm;

fn answer(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn malformed_symbol_arguments_are_rejected_before_side_effects() {
    // GNU eval.c uses unchecked XSYMBOL here; its native process crashes for
    // malformed callers. VM2 deliberately rejects them without memory access.
    for arg in ["1", "1.5", "\"text\"", "'(a)", "[1]"] {
        let mut vm = Vm::new();
        let source = format!("(let ((current-load-list '(untouched))) (list (condition-case e (internal--define-uninitialized-variable {arg} 7) (error (list (car e) (car (cdr e)) (equal (car (cdr (cdr e))) {arg})))) current-load-list))");
        assert_eq!(answer(&mut vm, &source), "((wrong-type-argument symbolp t) (untouched))", "unsafe argument {arg}");
    }
}

#[test]
fn declaration_is_object_owned_and_preserves_value_cells() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(answer(&mut vm, "(progn (internal--define-uninitialized-variable 'uv-interned) (special-variable-p (make-symbol \"uv-interned\")))"), "nil", "interned declaration must not mark an uninterned twin");
    assert_eq!(answer(&mut vm, "(progn (setq uv-original '(3 4)) (setq uv-held uv-original) (internal--define-uninitialized-variable 'uv-original) (eq uv-held uv-original))"), "t", "declaration must preserve value identity");
    assert_eq!(answer(&mut vm, "(progn (internal--define-uninitialized-variable 'uv-dynamic) (setq uv-dynamic 3) (eval '(let ((uv-dynamic 9)) (symbol-value 'uv-dynamic)) t))"), "9", "declaration must govern later dynamic binding");
}
