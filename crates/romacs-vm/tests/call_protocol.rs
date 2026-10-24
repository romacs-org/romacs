use romacs_vm::Vm;

#[test]
fn primitive_cache_rechecks_each_changed_alias_target() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(defalias 'sum-alias '+) (defun via-alias () (sum-alias 1 2 3)) (via-alias) (defalias '+ (lambda (a b c) 42)) (via-alias)").unwrap();
    assert_eq!(value.as_integer().unwrap(), 42);
    let value = vm
        .eval_source("(defalias 'sum-alias (lambda (a b c) 43)) (via-alias)")
        .unwrap();
    assert_eq!(value.as_integer().unwrap(), 43);
}

#[test]
fn cached_functions_copy_the_current_environment_and_check_each_arity() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = vm.eval_source("(defun mk (a b c d) (lambda (x y z &optional o &rest r) (list a b c d x y z o r))) (defvar f1 (mk 1 2 3 4)) (defvar f2 (mk 5 6 7 8)) (defun invoke (f args) (apply f args)) (invoke f1 '(9 10 11)) (invoke f2 '(12 13 14 15 16 17))").unwrap();
    assert_eq!(value.display().unwrap(), "(5 6 7 8 12 13 14 15 (16 17))");
    let value = vm.eval_source("(invoke f1 '(9 10 11))").unwrap();
    assert_eq!(value.display().unwrap(), "(1 2 3 4 9 10 11 nil nil)");
    let value = vm
        .eval_source("(condition-case nil (invoke f1 '(1 2)) (wrong-number-of-arguments 42))")
        .unwrap();
    assert_eq!(value.as_integer().unwrap(), 42);
    // The same cached fixed-arity function sees varying apply counts too.
    let value = vm.eval_source("(defun fixed (a b c d) d) (invoke 'fixed '(1 2 3 4)) (condition-case nil (invoke 'fixed '(1 2 3 4 5)) (wrong-number-of-arguments 43))").unwrap();
    assert_eq!(value.as_integer().unwrap(), 43);
}
