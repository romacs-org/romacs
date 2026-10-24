use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("ERROR: {error}"),
    }
}

#[test]
fn charset_plist_replacement_returns_and_stores_the_same_object() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let ((p (list :probe 7))) (list (eq (set-charset-plist 'ascii p) p) (eq (charset-plist 'ascii) p)))"), "(t t)", "charset replacement must return and store the input object");
    assert_eq!(eval(&mut vm, "(let ((p (list :probe 7))) (set-charset-plist 'ascii p) (setcar p :changed) (charset-plist 'ascii))"), "(:changed 7)", "stored plist must observe input mutation");
}

#[test]
fn charset_plist_bad_charset_has_exact_condition_data() {
    let mut vm = Vm::new();
    for (arg, expected) in [("'missing-charset", "(wrong-type-argument charsetp missing-charset)"), ("42", "(wrong-type-argument charsetp 42)"), ("nil", "(wrong-type-argument charsetp nil)")] {
        assert_eq!(eval(&mut vm, &format!("(condition-case e (set-charset-plist {arg} nil) (error e))")), expected, "invalid charset must signal charsetp with original datum");
    }
}

#[test]
fn charset_plist_non_plists_are_stored_without_validation() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (set-charset-plist 'ascii 42) (charset-plist 'ascii) (set-charset-plist 'ascii '(odd)) (charset-plist 'ascii))"), "(42 42 (odd) (odd))", "GNU setter accepts non-plists verbatim");
}

#[test]
fn charset_plist_lazy_roots_survive_moving_gc() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_stats().live_objects, 0, "charset registry must allocate nothing at startup");
    eval(&mut vm, "(make-vector 100 nil) (set-charset-plist 'ascii (list (copy-sequence \"retained\")))");
    vm.collect_garbage();
    assert_eq!(eval(&mut vm, "(charset-plist 'ascii)"), "(\"retained\")", "charset plist must be traced and forwarded");
}
