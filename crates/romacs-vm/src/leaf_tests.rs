use super::*;
use super::leaf::Kind;

fn ok<T>(result: std::result::Result<T, Fault>) -> T {
    result.unwrap_or_else(|_| panic!("unexpected Leaf fault"))
}
fn int(n: i64) -> Value { Value::integer(n).unwrap() }

#[test]
fn leaf_capability_surface_is_present() {
    let source = include_str!("prim.rs");
    let leaf = source.split("mod leaf {").nth(1).unwrap().split("pub(crate) use leaf::Leaf;").next().unwrap();
    for method in ["string", "byte_string", "string_text", "string_bytes", "vector", "vector_values", "vector_len", "vector_get", "vector_set", "setcar", "setcdr", "float", "float_value", "eq", "eql", "equal", "symbol_name", "default_cell", "default_cell_set", "symbol_function", "set_symbol_function", "intern", "obarray_lookup", "obarray_symbols", "kind"] {
        assert!(leaf.contains(&format!("pub fn {method}(")), "missing Leaf::{method}");
    }
}

#[test]
fn leaf_owned_readers_allow_allocation_and_mutation_after_read() {
    let mut vm = Vm::new();
    let before = vm.heap_stats().collections;
    let mut cx = Leaf::new(&mut vm);
    let text = cx.string("héλ🙂");
    let text_copy: String = ok(cx.string_text(text));
    let bytes_copy: Vec<u8> = ok(cx.string_bytes(text));
    let raw = cx.byte_string(&[0, 0x80, 0xff]);
    assert_eq!(ok(cx.string_bytes(raw)), [0, 0x80, 0xff]);
    // P1a: Unicode-only readers keep rejecting raw bytes, now with the
    // named NotUnicode boundary fault required by the string substrate contract.
    assert!(matches!(cx.string_text(raw), Err(Fault::NotUnicode(v)) if v == raw));
    let vector = cx.vector(&[text, raw]);
    let values: Vec<Value> = ok(cx.vector_values(vector));
    let item: Value = ok(cx.vector_get(vector, int(0)));
    let replacement = cx.string("replacement");
    assert_eq!(ok(cx.vector_set(vector, int(0), replacement)), replacement);
    assert_eq!(ok(cx.vector_get(vector, int(0))), replacement);
    assert_eq!(ok(cx.vector_len(vector)), 2);
    assert_eq!(values, [text, raw]);
    assert_eq!(item, text);
    assert_eq!(text_copy, "héλ🙂");
    assert_eq!(bytes_copy, "héλ🙂".as_bytes());
    for index in [-1, 2, Value::MAX_FIXNUM] {
        assert!(matches!(cx.vector_get(vector, int(index)), Err(Fault::Range(v, i)) if v == vector && i == int(index)));
        assert!(matches!(cx.vector_set(vector, int(index), replacement), Err(Fault::Range(v, i)) if v == vector && i == int(index)));
    }
    assert!(matches!(cx.vector_get(vector, raw), Err(Fault::Wrong("integerp", v)) if v == raw));
    for _ in 0..100 { cx.string("allocation without collection"); }
    assert_eq!(vm.heap_stats().collections, before);
}

#[test]
fn leaf_cons_mutation_and_typed_reader_errors() {
    let mut vm = Vm::new();
    let mut cx = Leaf::new(&mut vm);
    let pair = cx.cons(int(1), int(2));
    assert_eq!(ok(cx.setcar(pair, int(3))), int(3));
    assert_eq!(cx.pair(pair), Some((int(3), int(2))));
    assert_eq!(ok(cx.setcdr(pair, pair)), pair);
    assert_eq!(cx.pair(pair), Some((int(3), pair)));
    assert!(matches!(cx.setcar(Value::NIL, pair), Err(Fault::Wrong("consp", Value::NIL))));
    assert!(matches!(cx.setcdr(int(1), pair), Err(Fault::Wrong("consp", _))));
    assert!(matches!(cx.string_text(pair), Err(Fault::Wrong("stringp", v)) if v == pair));
    assert!(matches!(cx.string_bytes(pair), Err(Fault::Wrong("stringp", v)) if v == pair));
    assert!(matches!(cx.vector_values(pair), Err(Fault::Wrong("vectorp", v)) if v == pair));
    assert!(matches!(cx.float_value(pair), Err(Fault::Wrong("floatp", v)) if v == pair));
}

#[test]
fn leaf_gnu_float_bits_identity_and_structural_equality() {
    let mut vm = Vm::new();
    let mut cx = Leaf::new(&mut vm);
    for bits in [0u64, 0x8000_0000_0000_0000, 0x7ff8_0000_0000_0042, 0xfff8_0000_0000_0042] {
        let a = cx.float(f64::from_bits(bits));
        let b = cx.float(f64::from_bits(bits));
        assert_eq!(ok(cx.float_value(a)).to_bits(), bits);
        assert!(cx.eq(a, a));
        assert!(!cx.eq(a, b));
        assert!(cx.eql(a, b));
        assert!(matches!(cx.equal(a, b), Ok(true)));
    }
    let zero = cx.float(0.0);
    let negative = cx.float(-0.0);
    assert!(!cx.eql(zero, negative));
    assert!(matches!(cx.equal(zero, negative), Ok(false)));
    assert!(!cx.eql(zero, int(0)));
    let nan1 = cx.float(f64::from_bits(0x7ff8_0000_0000_0042));
    let nan2 = cx.float(f64::from_bits(0x7ff8_0000_0000_0043));
    assert!(!cx.eql(nan1, nan2));
    assert!(matches!(cx.equal(nan1, nan2), Ok(false)));
    let a = cx.string("same");
    let b = cx.byte_string(b"same");
    assert!(!cx.eq(a, b));
    assert!(matches!(cx.equal(a, b), Ok(true)));
    let va = cx.vector(&[a, int(1)]);
    let vb = cx.vector(&[b, int(1)]);
    assert!(matches!(cx.equal(va, vb), Ok(true)));
    ok(cx.vector_set(vb, int(1), int(2)));
    assert!(matches!(cx.equal(va, vb), Ok(false)));
    let ca = cx.cons(a, Value::NIL);
    let cb = cx.cons(b, Value::NIL);
    ok(cx.setcdr(ca, ca));
    ok(cx.setcdr(cb, cb));
    assert!(matches!(cx.equal(ca, cb), Ok(true)));
    ok(cx.setcar(cb, int(9)));
    assert!(matches!(cx.equal(ca, cb), Ok(false)));
}

#[test]
fn leaf_symbols_and_default_obarray_preserve_identity_and_cells() {
    let mut vm = Vm::new();
    let generated = vm.generated_symbol(GeneratedName(987654));
    let mut cx = Leaf::new(&mut vm);
    assert_eq!(cx.obarray_lookup("leaf-new-symbol"), None);
    let symbol = cx.intern("leaf-new-symbol");
    assert_eq!(cx.intern("leaf-new-symbol"), symbol);
    assert_eq!(cx.obarray_lookup("leaf-new-symbol"), Some(symbol));
    assert_eq!(cx.obarray_lookup("nil"), Some(Value::NIL));
    assert_eq!(cx.obarray_lookup("t"), Some(Value::TRUE));
    assert_eq!(ok(cx.symbol_name(Value::NIL)), "nil");
    assert_eq!(ok(cx.symbol_name(Value::TRUE)), "t");
    let name: String = ok(cx.symbol_name(symbol));
    assert_eq!(name, "leaf-new-symbol");
    assert!(matches!(cx.default_cell(symbol), Err(Fault::VoidVariable(v)) if v == symbol));
    assert_eq!(ok(cx.default_cell_set(symbol, int(11))), int(11));
    assert_eq!(ok(cx.default_cell(symbol)), int(11));
    assert_eq!(ok(cx.symbol_function(symbol)), Value::NIL);
    let alias = cx.intern("car");
    assert_eq!(ok(cx.set_symbol_function(symbol, alias)), alias);
    assert_eq!(ok(cx.symbol_function(symbol)), alias);
    assert!(matches!(cx.set_symbol_function(symbol, symbol), Err(Fault::CyclicFunctionIndirection(v)) if v == symbol));
    let other = cx.intern("leaf-cycle-other");
    ok(cx.set_symbol_function(other, symbol));
    assert!(matches!(cx.set_symbol_function(symbol, other), Err(Fault::CyclicFunctionIndirection(v)) if v == symbol));
    assert_eq!(ok(cx.symbol_function(symbol)), alias);
    assert_eq!(ok(cx.set_symbol_function(Value::NIL, Value::NIL)), Value::NIL);
    assert!(matches!(cx.set_symbol_function(Value::NIL, alias), Err(Fault::SettingConstant(Value::NIL))));
    assert_eq!(ok(cx.set_symbol_function(Value::TRUE, alias)), alias);
    assert_eq!(ok(cx.symbol_function(Value::TRUE)), alias);
    assert!(matches!(cx.symbol_name(int(1)), Err(Fault::Wrong("symbolp", _))));
    assert!(matches!(cx.symbol_function(Value::UNBOUND), Err(Fault::Wrong("symbolp", _))));
    assert!(matches!(cx.default_cell_set(Value::NIL, int(1)), Err(Fault::SettingConstant(Value::NIL))));
    let keyword = cx.intern(":leaf");
    assert!(matches!(cx.default_cell_set(keyword, int(1)), Err(Fault::SettingConstant(v)) if v == keyword));
    let symbols = cx.obarray_symbols();
    assert_eq!(symbols.iter().filter(|&&v| v == symbol).count(), 1);
    assert_eq!(symbols.iter().filter(|&&v| v == Value::NIL).count(), 1);
    assert_eq!(symbols.iter().filter(|&&v| v == Value::TRUE).count(), 1);
    assert!(!symbols.contains(&generated));
    assert!(!symbols.contains(&Value::UNBOUND));
    drop(cx);
    assert_eq!(vm.eval_source_raw("(leaf-new-symbol '(4 5))").unwrap(), int(4));
    // Warm a call site, replace its cell, and prove cache invalidation.
    vm.eval_source_raw("(defun leaf-caller (x) (leaf-new-symbol x))").unwrap();
    assert_eq!(vm.eval_source_raw("(leaf-caller '(8 9))").unwrap(), int(8));
    let cdr = vm.intern("cdr");
    ok(Leaf::new(&mut vm).set_symbol_function(symbol, cdr));
    let result = vm.eval_source_raw("(leaf-caller '(8 9))").unwrap();
    assert_eq!(vm.display(result).unwrap(), "(9)");
}

#[test]
fn leaf_kind_covers_every_current_storage_variant() {
    let mut vm = Vm::new();
    let mut cases = vec![(Value::NIL, Kind::Symbol), (Value::TRUE, Kind::Symbol), (Value::UNBOUND, Kind::Unbound), (int(-1), Kind::Integer)];
    for (source, kind) in [("1.25", Kind::Float), ("\"x\"", Kind::String), ("[1]", Kind::Vector), ("'(x)", Kind::Cons), ("(lambda (x) x)", Kind::Closure), ("#'car", Kind::Symbol), ("(current-buffer)", Kind::Buffer), ("(make-marker)", Kind::Marker), ("(make-overlay 1 1)", Kind::Overlay), ("(make-hash-table)", Kind::HashTable), ("(selected-window)", Kind::Window)] {
        cases.push((vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}")), kind));
    }
    cases.push((Value::reference(Primitive::Car as usize, 6), Kind::Subr));
    cases.push((vm.allocate_object(data::Object::SyntaxTable(Default::default())), Kind::SyntaxTable));
    cases.push((vm.byte_string(b"raw"), Kind::Bytes));
    vm.cells.push(Value::NIL);
    cases.push((Value::reference(vm.cells.len()-1, 5), Kind::Cell));
    cases.push((vm.allocate_object(data::Object::ByteBuffer(Box::new(bytes::ByteBuffer { state: Default::default(), case_tables: [Value::NIL; 4], bytes: vec![], syntax_table: Value::NIL, category_table: Value::NIL, point: coordinates::ByteOffset(0), restriction: coordinates::ByteOffset(0)..coordinates::ByteOffset(0), saved_restrictions: Vec::new(), saved_points: Vec::new(), properties: Default::default(), had_intervals: false, per_buffer: Default::default(), held: None }))), Kind::ByteBuffer));
    let cx = Leaf::new(&mut vm);
    for (value, kind) in cases { assert_eq!(cx.kind(value), kind, "{value:?}"); }
    assert_eq!(cx.kind(Value::reference(usize::MAX >> 3, 0)), Kind::Invalid);
}
