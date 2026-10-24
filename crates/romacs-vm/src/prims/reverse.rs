//! GNU Freverse at the held GNU pin.
use super::*;

defsubr! { name = "reverse", gnu = "fns.c:2322", effect = Pure,
fn reverse(cx, seq: Value) {
    if seq == Value::NIL {
        return Ok(Value::NIL);
    }
    if cx.pair(seq).is_some() {
        let (mut tail, mut tortoise, mut steps) = (seq, seq, 0u64);
        let mut result = Value::NIL;
        // FOR_EACH_TAIL's precise cycle witness (lisp.h), not a length cap.
        // Leaf allocation cannot collect or call Lisp: all copied Values stay
        // valid until the returned result/fault is published by the engine.
        while let Some((car, cdr)) = cx.pair(tail) {
            result = cx.cons(car, result);
            tail = cdr;
            if tail == tortoise {
                return Err(Fault::Circular(tail));
            }
            steps += 1;
            if steps & 4095 == 0 && steps.is_power_of_two() {
                tortoise = tail;
            }
        }
        return if tail == Value::NIL { Ok(result) }
            else { Err(Fault::Wrong("listp", tail)) };
    }
    if let Ok(bits) = cx.bool_vector(seq) {
        let mut result = crate::data::BoolVector::new(bits.len, false).map_err(Fault::Error)?;
        for i in 0..bits.len {
            result.set(i, bits.get(bits.len - 1 - i).unwrap());
        }
        return Ok(cx.bool_vector_from(result));
    }
    if let Ok(mut values) = cx.vector_values(seq) {
        // GNU make_uninit_vector(0) returns its canonical empty vector.
        if values.is_empty() { return Ok(seq); }
        values.reverse();
        return Ok(cx.vector(&values));
    }
    if let Ok(text) = cx.string_view(seq) {
        // GNU uses byte count == character count, not the source's flag:
        // explicitly multibyte ASCII becomes unibyte. Properties are dropped.
        let one_byte = text.chars == text.bytes.len();
        let multibyte = !one_byte && text.multibyte;
        let bytes = if one_byte || !text.multibyte {
            text.bytes.iter().rev().copied().collect()
        } else {
            let mut bytes = Vec::with_capacity(text.bytes.len());
            let chars: Vec<_> = text.chars().collect();
            let mut encoded = [0; 5];
            for code in chars.into_iter().rev() {
                let width = crate::lstring::char_string(code, &mut encoded);
                bytes.extend_from_slice(&encoded[..width]);
            }
            bytes
        };
        return Ok(cx.make_string(bytes, multibyte));
    }
    Err(Fault::Wrong("sequencep", seq))
}}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn reverse_result_actually_moves() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_collection_floor(1_000_000);
        vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i)))) (setq reverse-moving-result (reverse (list (list 11) (list 22))))").unwrap();
        let slot = vm.names["reverse-moving-result"];
        // Keep only the numeric old location for the movement assertion, never
        // dereference an unrooted Value after collection.
        let old = vm.slots[slot].value.0;
        vm.collect_garbage();
        assert_ne!(old, vm.slots[slot].value.0, "REVERSE_GC_MOVED");
        assert_eq!(vm.eval_source("reverse-moving-result").unwrap().display().unwrap(), "((22) (11))", "REVERSE_MOVED_CONTENTS");
    }
}
