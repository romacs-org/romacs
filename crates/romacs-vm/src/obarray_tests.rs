use super::*;
fn run(vm: &mut Vm, text: &str) -> Value { vm.eval_source_raw(text).unwrap() }
#[test]
fn obarray_compiled_call_retains_symbol_slot() {
    for kind in 0..4 {
        let mut vm = Vm::new();
        let table = vm.allocate_object(Object::Obarray(Box::new(Obarray::new(0, false))));
        let name = vm.make_lisp_string(b"code-owned".to_vec(), false);
        let symbol = vm.obarray_intern(table, name).ok().unwrap();
        let slot = symbol.payload() as usize - 1;
        vm.slots[slot].value = Value::integer(42).unwrap();
        let op = match kind {
            0 => Op::Define(slot, 0), 1 => Op::DefineMacro(slot, 0),
            2 => Op::Direct(0, slot, 0, 0, 0), _ => Op::Sealed(0, slot, 0, 0),
        };
        // Direct also carries a cache index, independently traced.
        let id = vm.compile(&Form::Int(0)).unwrap();
        let function = Arc::get_mut(&mut vm.functions[id]).unwrap();
        function.code = vec![op];
        if kind == 2 { vm.caches.push(Cache { version: 0, target: Value::NIL }); }
        let index = vm.closures.len();
        vm.closures.push(Closure { function: id, captures: vec![], metadata: vec![], document: Value::NIL });
        let mut roots = [Value::reference(index, 4)];
        vm.collect(&mut roots);
        assert!(vm.obarray_symbol_names.contains_key(&slot), "compiled custom-symbol operand retains its original slot: {kind}");
        assert_eq!(vm.slots[slot].value, Value::integer(42).unwrap());
    }
}
#[test]
fn obarray_condition_properties_do_not_root_dead_symbols() {
    let mut vm = Vm::new();
    let source = "(let ((s (intern \"e\" (obarray-make)))) (put s 'error-conditions (list s 'error))) nil";
    run(&mut vm, source); vm.collect(&mut []);
    let slots = vm.slots.len();
    for _ in 0..32 { run(&mut vm, source); vm.collect(&mut []); }
    assert!(vm.obarray_symbol_names.is_empty(), "a custom condition property cannot keep its dead owner alive");
    assert_eq!(vm.slots.len(), slots, "condition slots must be reused");
}
#[test]
fn obarray_dead_symbol_reclaimed_with_fully_live_heap() {
    let mut vm = Vm::new();
    let table = vm.allocate_object(Object::Obarray(Box::new(Obarray::new(0, false))));
    let name = vm.make_lisp_string(b"fully-live".to_vec(), false);
    let symbol = vm.obarray_intern(table, name).ok().unwrap();
    assert!(vm.obarray_unintern(table, symbol));
    let mut roots = [table, name];
    vm.collect(&mut roots);
    assert!(vm.obarray_symbol_names.is_empty(), "fully live object arena still sweeps dead symbols");
}

#[test]
fn obarray_identity_moves_and_retention_is_flat() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    assert!(!vm.objects.iter().any(|o| matches!(o, Object::Obarray(_))), "no startup obarray allocation");
    run(&mut vm, "(make-vector 100 nil)");
    run(&mut vm, "(setq ob-root (obarray-make) ob-sym (intern \"custom-name\" ob-root)) (set ob-sym (list (copy-sequence \"payload\")))");
    let before = vm.slots[vm.names["ob-root"]].value;
    vm.collect(&mut []);
    assert_ne!(before, vm.slots[vm.names["ob-root"]].value, "obarray must move");
    let result = run(&mut vm, "(list (eq ob-sym (intern \"custom-name\" ob-root)) (symbol-value ob-sym))");
    assert_eq!(vm.display(result).unwrap(), "(t (\"payload\"))", "symbol identity and sole-owned value survive moving collection");
    run(&mut vm, "(setq ob-keep ob-root ob-keep-sym ob-sym)");
    let source = "(setq ob-root (obarray-make) ob-sym (intern \"ephemeral\" ob-root)) (set ob-sym (list (make-vector 17 nil) (copy-sequence \"payload\"))) (setq ob-root nil ob-sym nil)";
    run(&mut vm, source); vm.collect(&mut []);
    let baseline = vm.heap_stats();
    let slots = vm.slots.len();
    for _ in 0..32 {
        run(&mut vm, source); vm.collect(&mut []);
        let now = vm.heap_stats();
        assert_eq!((now.live_objects, now.live_bytes), (baseline.live_objects, baseline.live_bytes), "obarray heap retention must be flat");
        assert_eq!(vm.slots.len(), slots, "custom-symbol slot retention must be flat");
    }
    eprintln!("obarray retention objects={} bytes={} slots={} cycles=32", baseline.live_objects, baseline.live_bytes, slots);
}
#[test]
fn obarray_unintern_storage_is_reclaimed() {
    let mut vm = Vm::new();
    run(&mut vm, "(setq ob-root (obarray-make))");
    let source = "(unintern (intern \"discarded-name\" ob-root) ob-root)";
    run(&mut vm, source); vm.collect(&mut []);
    let baseline = vm.heap_stats();
    for _ in 0..32 { run(&mut vm, source); vm.collect(&mut []); }
    assert_eq!(vm.heap_stats().live_bytes, baseline.live_bytes, "unintern must release dead entry storage");
}
#[test]
fn obarray_symbol_is_kept_by_variable_alias() {
    let mut vm = Vm::new();
    run(&mut vm, "(setq ob-root (obarray-make) ob-sym (intern \"aliased\" ob-root)) (set ob-sym 42) (defvaralias 'ob-held ob-sym) (setq ob-root nil ob-sym nil)");
    vm.collect(&mut []);
    let result = vm.eval_source_raw("(symbol-value 'ob-held)").and_then(|v| vm.display(v));
    assert_eq!(result, Ok("42".into()), "variable alias must retain custom-symbol target");
}
#[test]
fn obarray_symbol_survives_as_weak_table_value() {
    let mut vm = Vm::new();
    run(&mut vm, "(setq ob-root (obarray-make) ob-sym (intern \"held\" ob-root) ob-hash (make-hash-table :weakness 'key)) (set ob-sym 42) (puthash 'permanent ob-sym ob-hash) (setq ob-root nil ob-sym nil)");
    vm.collect(&mut []);
    let result = run(&mut vm, "(symbol-value (gethash 'permanent ob-hash))");
    assert_eq!(vm.display(result).unwrap(), "42", "weak-key table must strongly retain custom-symbol value");
}
/// A text property's key keeps its symbol: a symbol of a dead obarray that
/// only a property names survives collections, in a unibyte buffer, a
/// multibyte buffer and a string, and its slot is not given to the
/// symbols made next.  The buffers keep no undo, whose records would
/// name the key too.  The dead obarray's other symbol is reclaimed.
#[test]
fn a_text_property_key_keeps_its_obarray_symbol() {
    let mut vm = Vm::new();
    run(&mut vm, "(setq tpk-unibyte (get-buffer-create \"tpk-unibyte\") tpk-multibyte (get-buffer-create \"tpk-multibyte\") \
                        tpk-string (copy-sequence \"x\")) \
                  (let ((o (obarray-make))) \
                    (intern \"dropped\" o) \
                    (with-current-buffer tpk-unibyte (setq buffer-undo-list t) (set-buffer-multibyte nil) (insert \"x\") (put-text-property 1 2 (intern \"kept-unibyte\" o) t)) \
                    (with-current-buffer tpk-multibyte (setq buffer-undo-list t) (insert \"x\") (put-text-property 1 2 (intern \"kept-multibyte\" o) t)) \
                    (put-text-property 0 1 (intern \"kept-string\" o) t tpk-string)) \
                  nil");
    vm.collect(&mut []);
    vm.collect(&mut []);
    let names: std::collections::BTreeSet<&str> = vm.obarray_symbol_names.keys().map(|&slot| vm.symbol_names[slot].as_str()).collect();
    assert!(!names.contains("dropped"), "the dead obarray's unnamed symbol is reclaimed: {names:?}");
    run(&mut vm, "(let ((reuse (obarray-make))) (dotimes (i 100) (intern (format \"reused-%d\" i) reuse)))");
    let keys = run(&mut vm, "(list (symbol-name (car (with-current-buffer tpk-unibyte (text-properties-at 1)))) \
                                   (symbol-name (car (with-current-buffer tpk-multibyte (text-properties-at 1)))) \
                                   (symbol-name (car (text-properties-at 0 tpk-string))))");
    assert_eq!(vm.display(keys).unwrap(), "(\"kept-unibyte\" \"kept-multibyte\" \"kept-string\")");
}
