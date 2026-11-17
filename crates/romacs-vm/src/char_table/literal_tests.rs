use super::*;

fn integer(n: usize) -> Value { Value::integer(n as i64).unwrap() }
fn sub(vm: &mut Vm, depth: usize, min: usize, value: Value) -> Value {
    let mut fields = vec![integer(depth), integer(min)];
    fields.extend(std::iter::repeat_n(value, CharTableNode::SIZES[depth]));
    vm.sub_char_table_from_literal(&fields).unwrap()
}
fn node(vm: &Vm, value: Value) -> CharTableNodeHandle {
    let data::Object::SubCharTable(n) = vm.object(value).unwrap() else { panic!() };
    Arc::clone(n)
}
fn table(vm: &Vm, value: Value) -> &CharTable {
    let data::Object::CharTable(t) = vm.object(value).unwrap() else { panic!() };
    t
}
fn raw(vm: &mut Vm, ascii: Value, first: Value) -> Value {
    let mut fields = vec![Value::NIL; 68];
    fields[3] = ascii;
    fields[4] = first;
    vm.char_table_from_literal(&fields).unwrap()
}

#[test]
fn literal_validation_order_and_exact_messages() {
    let mut vm = Vm::new();
    assert_eq!(vm.char_table_from_literal(&[Value::NIL; 67]).unwrap_err(), "Invalid size char-table");
    assert_eq!(vm.sub_char_table_from_literal(&[]).unwrap_err(), "Invalid size of sub-char-table");
    assert_eq!(vm.sub_char_table_from_literal(&[Value::NIL, Value::NIL]).unwrap_err(), "Invalid depth in sub-char-table");
    assert_eq!(vm.sub_char_table_from_literal(&[integer(3), Value::NIL]).unwrap_err(), "Invalid size in sub-char-table");
    let mut fields = vec![Value::NIL; 130];
    fields[0] = integer(3);
    assert_eq!(vm.sub_char_table_from_literal(&fields).unwrap_err(), "Invalid minimum character in sub-char-table");
    fields[1] = integer(CharTable::LEN);
    assert_eq!(vm.sub_char_table_from_literal(&fields).unwrap_err(), "Invalid minimum character in sub-char-table");
    fields[1] = integer(17); // GNU accepts unaligned minima.
    let value = vm.sub_char_table_from_literal(&fields).unwrap();
    assert_eq!(node(&vm, value).lock().unwrap().min_char, 17);
}

#[test]
fn literal_ascii_is_explicit_and_single_writes_preserve_detached_identity() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_stats().live_objects, 0);
    let leaf = sub(&mut vm, 3, 0, integer(7));
    let value = raw(&mut vm, leaf, integer(9));
    assert_eq!(vm.char_table_get(value, 65).unwrap(), integer(7));
    assert_eq!(vm.char_table_get(value, 128).unwrap(), integer(9));
    let t = vm.objects.entry_mut(value.index(0).unwrap() - 1);
    let data::Object::CharTable(t) = t else { panic!() };
    t.set(65, integer(10)).unwrap();
    assert_eq!(t.get(65).unwrap(), integer(10));
    assert_eq!(t.get(66).unwrap(), integer(7));
    assert_eq!(CharTableNode::get(&node(&vm, leaf), 65), integer(10));
    assert!(matches!(table(&vm, value).root.lock().unwrap().slots[0], CharTableSlot::Value(v) if v == integer(9)));
}

#[test]
fn literal_shared_ascii_and_tree_print_and_copy() {
    let mut vm = Vm::new();
    let leaf = sub(&mut vm, 3, 0, integer(7));
    let two = sub(&mut vm, 2, 0, leaf);
    let one = sub(&mut vm, 1, 0, two);
    let value = raw(&mut vm, leaf, one);
    let CharTableSlot::Node(ascii) = table(&vm, value).ascii.clone() else { panic!() };
    assert!(Arc::ptr_eq(&ascii, &node(&vm, leaf)));
    vm.eval_source_raw("(setq print-circle t)").unwrap();
    let text = vm.print_render(value, true, Value::NIL, false).unwrap();
    assert!(text.starts_with("#^[nil nil nil #1=#^^[3 0 7"), "{text}");
    assert!(text.contains("#1#"));
    let copy = table(&vm, value).clone();
    let CharTableSlot::Node(copy_ascii) = copy.ascii else { panic!() };
    assert!(!Arc::ptr_eq(&copy_ascii, &ascii));
    let root = copy.root.lock().unwrap();
    let CharTableSlot::Node(one) = &root.slots[0] else { panic!() };
    let one = one.lock().unwrap();
    let CharTableSlot::Node(two) = &one.slots[0] else { panic!() };
    let two = two.lock().unwrap();
    let CharTableSlot::Node(three) = &two.slots[0] else { panic!() };
    assert!(Arc::ptr_eq(&copy_ascii, three));
}

#[test]
fn literal_uniform_print_preserves_ascii_default_parent_purpose_and_extras() {
    let mut vm = Vm::new();
    let mut fields = vec![integer(9); 80];
    fields[0] = integer(1);
    fields[1] = integer(2);
    fields[2] = integer(3);
    fields[3] = integer(4);
    let value = vm.char_table_from_literal(&fields).unwrap();
    let expected = format!("#^[{}]", fields.iter().map(|v| v.integer_value().unwrap().to_string()).collect::<Vec<_>>().join(" "));
    assert_eq!(vm.print_render(value, true, Value::NIL, false).unwrap(), expected);
    assert_eq!(table(&vm, value).get(0).unwrap(), integer(4));
    assert_eq!(table(&vm, value).get(128).unwrap(), integer(9));
    assert_eq!(table(&vm, value).extras.len(), 12);
}

#[test]
fn literal_standalone_print_and_length() {
    let mut vm = Vm::new();
    let value = sub(&mut vm, 3, 17, integer(7));
    let expected = format!("#^^[3 17 {}]", vec!["7"; 128].join(" "));
    assert_eq!(vm.print_render(value, true, Value::NIL, false).unwrap(), expected);
    vm.eval_source_raw("(setq print-length 3)").unwrap();
    assert_eq!(vm.print_render(value, true, Value::NIL, false).unwrap(), "#^^[3 17 7 7 ...]");
}

#[test]
fn literal_gc_forwards_standalone_and_shared_payload_once() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    vm.eval_source_raw("(make-vector 100 nil)").unwrap();
    let payload = vm.eval_source_raw("(list (copy-sequence \"payload\"))").unwrap();
    let leaf = sub(&mut vm, 3, 0, payload);
    let handle = node(&vm, leaf);
    let value = raw(&mut vm, leaf, Value::NIL);
    let mut roots = [leaf, value];
    vm.collect(&mut roots);
    assert!(Arc::ptr_eq(&handle, &node(&vm, roots[0])));
    let after = CharTableNode::get(&handle, 0);
    assert_ne!(payload, after);
    assert_eq!(vm.display(after).unwrap(), "(\"payload\")");
    assert_eq!(vm.char_table_get(roots[1], 0).unwrap(), after);
    // Only the table now roots the ASCII node, not its Lisp wrapper.
    let mut roots = [roots[1]];
    vm.collect(&mut roots);
    assert_eq!(vm.display(vm.char_table_get(roots[0], 0).unwrap()).unwrap(), "(\"payload\")");
}

#[test]
fn literal_malformed_geometry_is_retained_but_lookup_and_write_refuse() {
    let mut vm = Vm::new();
    let leaf = sub(&mut vm, 3, 17, integer(7));
    let value = raw(&mut vm, Value::NIL, leaf);
    assert!(vm.char_table_get(value, 65535).is_err());
    let data::Object::CharTable(t) = vm.objects.entry_mut(value.index(0).unwrap() - 1) else { panic!() };
    assert_eq!(t.set(128, integer(9)).unwrap_err(), "invalid sub-char-table geometry");
    assert!(vm.print_render(value, true, Value::NIL, false).is_ok());
}

/// A write refused for a malformed node below it writes nothing: not the
/// slots of its range before that node, nor a split of them.
#[test]
fn a_refused_write_changes_nothing() {
    let mut vm = Vm::new();
    // Characters 4096..8191 sit under a depth-two node claiming to start at 17.
    let bad = sub(&mut vm, 2, 17, integer(5));
    let mut fields = vec![integer(1), integer(0)];
    fields.extend((0..CharTableNode::SIZES[1]).map(|i| if i == 1 { bad } else { integer(7) }));
    let middle = vm.sub_char_table_from_literal(&fields).unwrap();
    let value = raw(&mut vm, Value::NIL, middle);
    let writes = super::tests::WROTE.with(std::cell::Cell::get);
    let data::Object::CharTable(t) = vm.objects.entry_mut(value.index(0).unwrap() - 1) else { panic!() };
    assert_eq!(t.set_range(100, 4200, integer(9)).unwrap_err(), "invalid sub-char-table geometry");
    assert_eq!(t.set(4200, integer(9)).unwrap_err(), "invalid sub-char-table geometry");
    assert_eq!(super::tests::WROTE.with(std::cell::Cell::get), writes);
    assert_eq!(t.get(200).unwrap(), integer(7));
    assert!(matches!(node(&vm, middle).lock().unwrap().slots[0], CharTableSlot::Value(v) if v == integer(7)));
}

#[test]
fn literal_image_publication_explicitly_refuses_native_graph() {
    let mut vm = Vm::new();
    let entry = vm.compile(&Form::Int(0)).unwrap();
    sub(&mut vm, 3, 0, Value::NIL);
    let path = std::path::Path::new("/unused-native-table-image");
    assert_eq!(vm.publish_code_image(path, entry, [0; 32]).unwrap_err(),
        "code image cannot publish character tables; native graph serialization is not supported");
}

fn bind(vm: &mut Vm, name: &str, value: Value) {
    let slot = vm.slot(name);
    vm.slots[slot].value = value;
}

#[test]
fn review_mapping_rejects_unsafe_geometry_without_panicking() {
    let mut vm = Vm::new();
    let leaf = sub(&mut vm, 3, 17, integer(7));
    let value = raw(&mut vm, Value::NIL, leaf);
    bind(&mut vm, "review-table", value);
    assert!(vm.eval_source_raw("(map-keymap (lambda (k v) nil) (list 'keymap review-table))").is_err());
    assert!(vm.print_render(value, true, Value::NIL, false).is_ok());
}

#[test]
fn review_non_table_parent_is_ignored_by_lookup_and_mapping() {
    let mut vm = Vm::new();
    let mut fields = vec![Value::NIL; 68];
    fields[1] = integer(42);
    let value = vm.char_table_from_literal(&fields).unwrap();
    assert_eq!(vm.char_table_get(value, 128).unwrap(), Value::NIL);
    bind(&mut vm, "review-table", value);
    assert!(vm.eval_source_raw("(map-keymap (lambda (k v) nil) (list 'keymap review-table))").is_ok());
}

#[test]
fn review_subtable_type_and_copy_sequence_contract() {
    let mut vm = Vm::new();
    let value = sub(&mut vm, 3, 0, integer(7));
    bind(&mut vm, "review-sub", value);
    let answer = vm.eval_source_raw("(list (type-of review-sub) (sequencep review-sub) (condition-case e (copy-sequence review-sub) (wrong-type-argument (car (cdr e)))))").unwrap();
    assert_eq!(vm.display(answer).unwrap(), "(sub-char-table nil sequencep)");
}

#[test]
fn review_subtable_writes_use_native_nodes() {
    let mut vm = Vm::new();
    let leaf = sub(&mut vm, 3, 0, integer(7));
    let two = sub(&mut vm, 2, 0, Value::NIL);
    node(&vm, two).lock().unwrap().slots[0] = CharTableSlot::Node(node(&vm, leaf));
    let one = sub(&mut vm, 1, 0, Value::NIL);
    node(&vm, one).lock().unwrap().slots[0] = CharTableSlot::Node(node(&vm, two));
    bind(&mut vm, "review-sub", one);
    let value = vm.eval_source_raw("(setq review-table (make-char-table nil)) (set-char-table-range review-table '(0 . 65535) review-sub) review-table").unwrap();
    assert!(matches!(table(&vm, value).root.lock().unwrap().slots[0], CharTableSlot::Node(_)));
    assert_eq!(vm.char_table_get(value, 65).unwrap(), integer(7));
    // Defaults are Lisp values, not contents-tree edges.
    let answer = vm.eval_source_raw("(set-char-table-range review-table nil review-sub) (eq (aref review-table 65536) review-sub)").unwrap();
    assert_eq!(answer, Value::TRUE);
    let stored_leaf = sub(&mut vm, 3, 0, integer(11));
    bind(&mut vm, "review-leaf", stored_leaf);
    vm.eval_source_raw("(aset review-table 128 review-leaf)").unwrap();
    assert!(table(&vm, value).get(128).is_err(), "an unsafe depth-three edge must not become a scalar wrapper");
    let initialized = vm.eval_source_raw("(make-char-table nil review-leaf)").unwrap();
    assert!(matches!(table(&vm, initialized).root.lock().unwrap().slots[0], CharTableSlot::Node(_)));
    let filled = vm.eval_source_raw("(let ((ct (make-char-table nil))) (set-char-table-range ct t review-leaf) ct)").unwrap();
    assert!(matches!(table(&vm, filled).root.lock().unwrap().slots[63], CharTableSlot::Node(_)));
    vm.eval_source_raw("(aset review-table 65 review-leaf)").unwrap();
    let CharTableSlot::Node(ascii) = &table(&vm, value).ascii else { panic!() };
    assert!(matches!(ascii.lock().unwrap().slots[65], CharTableSlot::Node(_)));
}

#[test]
fn review_independent_ascii_is_preserved_in_copy_and_rejected_by_mapping() {
    let mut vm = Vm::new();
    let leaf = sub(&mut vm, 3, 0, integer(7));
    let value = raw(&mut vm, leaf, integer(9));
    assert_eq!(table(&vm, value).clone().get(65).unwrap(), integer(9));
    bind(&mut vm, "review-table", value);
    assert!(vm.eval_source_raw("(map-keymap (lambda (k v) nil) (list 'keymap review-table))").is_err());
}
