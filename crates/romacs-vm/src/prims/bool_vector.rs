//! Packed GNU bool vectors. All temporary storage is owned; leaves cannot collect.
use super::*;
use crate::data::BoolVector;

fn truth(b: bool) -> Value { if b { Value::TRUE } else { Value::NIL } }
fn wrong_length(cx: &mut Leaf<'_>, a: Value, b: Value, c: Value) -> Fault {
    let size = |v| cx.bool_vector(v).map(|b| Value::integer(b.len as i64).unwrap());
    let a = match size(a) { Ok(n) => n, Err(e) => return e };
    let b = match size(b) { Ok(n) => n, Err(e) => return e };
    let c = if c == Value::NIL { None } else {
        match size(c) { Ok(n) => Some(n), Err(e) => return e }
    };
    let data = c.map(|c| cx.cons(c, Value::NIL)).unwrap_or(Value::NIL);
    let data = cx.cons(b, data);
    let data = cx.cons(a, data);
    let symbol = cx.intern("wrong-length-argument");
    Fault::Signal(symbol, data)
}

defsubr! { name = "make-bool-vector", gnu = "alloc.c:2202", effect = Pure,
fn make_bool_vector(cx, length: Value, init: Value) {
    let n = length.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or(Fault::Wrong("wholenump", length))?;
    let bits = BoolVector::new(n, init != Value::NIL).map_err(Fault::Error)?;
    Ok(cx.bool_vector_from(bits))
}}
defsubr! { name = "bool-vector", gnu = "alloc.c:2215", effect = Pure,
fn bool_vector(cx; rest values) {
    let mut bits = BoolVector::new(values.len(), false).map_err(Fault::Error)?;
    for (i, &v) in values.iter().enumerate() { bits.set(i, v != Value::NIL); }
    Ok(cx.bool_vector_from(bits))
}}
defsubr! { name = "bool-vector-p", gnu = "data.c:454", effect = Pure,
fn bool_vector_p(cx, value: Value) { Ok(truth(cx.bool_vector(value).is_ok())) }}

fn binary(cx: &mut Leaf<'_>, a: Value, b: Value, dest: Value, op: u8) -> Out {
    let a_bits = cx.bool_vector(a)?;
    let b_bits = cx.bool_vector(b)?;
    if a_bits.len != b_bits.len { return Err(wrong_length(cx, a, b, dest)); }
    let old = if dest != Value::NIL {
        let bits = cx.bool_vector(dest)?;
        if bits.len != a_bits.len { return Err(wrong_length(cx, a, b, dest)); }
        Some(bits)
    } else { None };
    let mut result = a_bits;
    for (x, y) in result.bytes.iter_mut().zip(b_bits.bytes) {
        *x = match op { 0 => *x ^ y, 1 => *x | y, 2 => *x & y, _ => *x & !y };
    }
    if old.as_ref() == Some(&result) { return Ok(Value::NIL); }
    if dest == Value::NIL { Ok(cx.bool_vector_from(result)) }
    else { cx.bool_vector_store(dest, result)?; Ok(dest) }
}
defsubr! { name = "bool-vector-exclusive-or", gnu = "data.c:3824", effect = MutatesArgs,
fn exclusive_or(cx, a: Value, b: Value; optional dest: Value) { binary(cx, a, b, dest, 0) }}
defsubr! { name = "bool-vector-union", gnu = "data.c:3835", effect = MutatesArgs,
fn union(cx, a: Value, b: Value; optional dest: Value) { binary(cx, a, b, dest, 1) }}
defsubr! { name = "bool-vector-intersection", gnu = "data.c:3846", effect = MutatesArgs,
fn intersection(cx, a: Value, b: Value; optional dest: Value) { binary(cx, a, b, dest, 2) }}
defsubr! { name = "bool-vector-set-difference", gnu = "data.c:3857", effect = MutatesArgs,
fn difference(cx, a: Value, b: Value; optional dest: Value) { binary(cx, a, b, dest, 3) }}
defsubr! { name = "bool-vector-subsetp", gnu = "data.c:3868", effect = Pure,
fn subsetp(cx, a: Value, b: Value) {
    let x = cx.bool_vector(a)?;
    let y = cx.bool_vector(b)?;
    if x.len != y.len { return Err(wrong_length(cx, a, b, b)); }
    Ok(truth(x.bytes.iter().zip(y.bytes).all(|(x,y)| x & !y == 0)))
}}
defsubr! { name = "bool-vector-not", gnu = "data.c:3877", effect = MutatesArgs,
fn complement(cx, a: Value; optional dest: Value) {
    let mut bits = cx.bool_vector(a)?;
    if dest != Value::NIL && cx.bool_vector(dest)?.len != bits.len {
        return Err(wrong_length(cx, a, dest, Value::NIL));
    }
    for b in &mut bits.bytes { *b = !*b; }
    bits.mask_padding();
    if dest == Value::NIL { Ok(cx.bool_vector_from(bits)) }
    else { cx.bool_vector_store(dest, bits)?; Ok(dest) }
}}
defsubr! { name = "bool-vector-count-population", gnu = "data.c:3918", effect = Pure,
fn population(cx, a: Value) {
    let bits = cx.bool_vector(a)?;
    Ok(Value::integer(bits.bytes.iter().map(|x| i64::from(x.count_ones())).sum()).unwrap())
}}
defsubr! { name = "bool-vector-count-consecutive", gnu = "data.c:3943", effect = Pure,
fn consecutive(cx, a: Value, b: Value, index: Value) {
    let bits = cx.bool_vector(a)?;
    let i = index.integer_value().and_then(|i| usize::try_from(i).ok())
        .ok_or(Fault::Wrong("wholenump", index))?;
    if i > bits.len { return Err(Fault::Range(a, index)); }
    let count = (i..bits.len).take_while(|&i| bits.get(i) == Some(b != Value::NIL)).count();
    Ok(Value::integer(count as i64).unwrap())
}}

#[cfg(test)]
mod tests {
    use super::*;
    fn replay(name: &str) {
        let oracle: serde_json::Value = serde_json::from_str(include_str!("../../evidence/bool-vector/oracle.json")).unwrap();
        let rows = oracle["rows"].as_array().unwrap();
        let mut count = 0;
        for row in rows.iter().filter(|r| r["primitive"] == name) {
            let mut vm = crate::Vm::new();
            vm.set_collection_floor(1);
            let value = vm.eval_source_raw(row["source"].as_str().unwrap()).unwrap_or_else(|e| panic!("{}: {e}", row["id"]));
            let actual = Leaf::new(&mut vm).string_text(value).unwrap_or_else(|_| panic!("expected string"));
            if row["replay"] == false {
                assert!(actual.starts_with("(sig (error "), "{}: memory-full class: {actual}", row["id"]);
            } else {
                assert_eq!(actual, row["expected"].as_str().unwrap(), "{}", row["id"]);
            }
            count += 1;
        }
        assert!(count >= 3, "empty or truncated oracle: {name}");
    }
    macro_rules! case {
        ($test:ident, $name:literal) => { #[test] fn $test() { replay($name); } };
    }
    case!(make_bool_vector_oracle, "make-bool-vector");
    case!(bool_vector_oracle, "bool-vector");
    case!(predicate_oracle, "bool-vector-p");
    case!(xor_oracle, "bool-vector-exclusive-or");
    case!(union_oracle, "bool-vector-union");
    case!(intersection_oracle, "bool-vector-intersection");
    case!(difference_oracle, "bool-vector-set-difference");
    case!(subset_oracle, "bool-vector-subsetp");
    case!(not_oracle, "bool-vector-not");
    case!(population_oracle, "bool-vector-count-population");
    case!(consecutive_oracle, "bool-vector-count-consecutive");

    fn printed(source: &str) -> String {
        let mut vm = crate::Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source_raw(source).unwrap();
        vm.display(value).unwrap()
    }

    #[test]
    fn integration_append_copies_bits_and_preserves_final_tail() {
        assert_eq!(printed("(let* ((b (bool-vector t nil t)) (tail (list 'end)) (out (append b tail))) (aset b 0 nil) (list out (eq (cdr (cdr (cdr out))) tail)))"), "((t nil t end) t)");
        assert_eq!(printed("(let ((b (bool-vector t nil))) (list (eq (append b) b) (eq (cdr (append '(x) b)) b) (eq (append (bool-vector) b) b) (append (bool-vector) nil)))"), "(t t t nil)");
        for len in [0, 1, 7, 8, 9, 63, 64, 65] {
            let source = format!("(let ((b (make-bool-vector {len} nil))) (dotimes (i {len}) (aset b i (= (% i 3) 1))) (append b nil))");
            let expected = if len == 0 { "nil".to_owned() } else {
                format!("({})", (0..len).map(|i| if i % 3 == 1 { "t" } else { "nil" }).collect::<Vec<_>>().join(" "))
            };
            assert_eq!(printed(&source), expected, "length {len}");
        }
    }

    #[test]
    fn integration_nreverse_mutates_alias_and_masks_boundaries() {
        for len in [0, 1, 7, 8, 9, 63, 64, 65] {
            let mut vm = crate::Vm::new();
            vm.set_collection_floor(1);
            let source = format!("(let* ((b (make-bool-vector {len} nil)) (alias b)) (dotimes (i {len}) (aset b i (= (% i 3) 1))) (if (eq (nreverse b) alias) alias nil))");
            let value = vm.eval_source_raw(&source).unwrap();
            let bits = Leaf::new(&mut vm).bool_vector(value).unwrap_or_else(|_| panic!("expected bool-vector"));
            assert_eq!(bits.len, len);
            for i in 0..len { assert_eq!(bits.get(i), Some((len - 1 - i) % 3 == 1), "length {len}, bit {i}"); }
            let mut masked = bits.clone();
            masked.mask_padding();
            assert_eq!(bits, masked, "length {len}: padding");
        }
    }

    #[test]
    fn integration_fillarray_mutates_alias_with_truthiness_and_masked_tail() {
        for len in [0, 1, 7, 8, 9, 63, 64, 65] {
            for (item, truth) in [("nil", false), ("t", true), ("0", true), ("'symbol", true), ("'(nil)", true)] {
                let mut vm = crate::Vm::new();
                vm.set_collection_floor(1);
                let source = format!("(let* ((b (make-bool-vector {len} {})) (alias b)) (if (eq (fillarray b {item}) alias) alias nil))", if truth { "nil" } else { "t" });
                let value = vm.eval_source_raw(&source).unwrap();
                let bits = Leaf::new(&mut vm).bool_vector(value).unwrap_or_else(|_| panic!("expected bool-vector"));
                assert_eq!(bits, BoolVector::new(len, truth).unwrap(), "length {len}, item {item}");
            }
        }
    }

    #[test]
    fn bool_vector_moving_gc() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(setq bv (bool-vector t nil t))").unwrap();
        for _ in 0..3 {
            vm.collect(&mut []);
            let result = vm.eval_source_raw("(list (aref bv 0) (aref bv 1) (aref bv 2) (bool-vector-count-population bv))").unwrap();
            assert_eq!(vm.display(result).unwrap(), "(t nil t 2)");
        }
    }
}
