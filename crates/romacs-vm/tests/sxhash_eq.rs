use romacs_vm::Vm;

#[test]
fn identity_hash_survives_mutation_and_moving_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = "(let* ((a (list 1)) (b (list 1)) (alias a) (ha (sxhash-eq a)) (hb (sxhash-eq b))) (setcar a 2) (garbage-collect) (list (= ha (sxhash-eq a)) (= ha (sxhash-eq alias)) (= hb (sxhash-eq b)) (/= ha hb) (integerp ha) (>= ha 0)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t t t t t)");
}

#[test]
fn identity_hash_does_not_retain_dead_objects() {
    let mut vm = Vm::new();
    vm.eval_source("(dotimes (n 1000) (sxhash-eq (list n))) (garbage-collect)").unwrap();
    let source = "(hash-table-count (symbol-value (intern \"#:vm2-sxhash-eq\")))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "0");
}
