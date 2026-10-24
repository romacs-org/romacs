use romacs_vm::Vm;

fn answer(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn toplevel_saved_binding_contract() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(condition-case e (progn (defvar tv 1) (let ((tv 2)) (let ((tv 3)) (list (default-toplevel-value 'tv) (set-default-toplevel-value 'tv 4) tv (default-value 'tv))))) (error e))"), "(1 nil 3 3)", "toplevel reads and writes the outermost saved default, preserving live bindings");
    assert_eq!(answer(&mut vm, "tv"), "4", "toplevel write survives unwinding");
}

#[test]
fn toplevel_saved_values_survive_counted_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    assert_eq!(answer(&mut vm, "(progn (defvar tv-gc 1) (let ((tv-gc 2)) (set-default-toplevel-value 'tv-gc (list 4 5)) (let ((i 0)) (while (< i 3000) (list i i) (setq i (1+ i))))) tv-gc)"), "(4 5)", "saved toplevel value must remain rooted through moving collection and unwind");
    assert!(vm.heap_stats().collections > before, "saved toplevel value probe must collect");
}
