//! GNU fns.c sequence access.
use super::*;
use crate::prim::leaf::Kind;

// GNU Felt accepts ARRAYP, which excludes records and byte-code objects even
// though Faref itself accepts them.
defsubr! { name = "elt", gnu = "fns.c:1853", effect = Pure,
fn elt(cx, sequence: Value, index: Value) {
    if sequence == Value::NIL || cx.pair(sequence).is_some() {
        let tail = if let Some(n) = index.integer_value() {
            super::fns::nthcdr(cx, Integer(n), sequence)?
        } else {
            let n = cx.bigint_value(index).ok_or(Fault::Wrong("integerp", index))?;
            bignum_nthcdr(cx, n, sequence)?
        };
        return match cx.pair(tail) {
            Some((car, _)) => Ok(car),
            None if tail == Value::NIL => Ok(Value::NIL),
            None => Err(Fault::Wrong("listp", tail)),
        };
    }
    let kind = cx.kind(sequence);
    if !matches!(kind, Kind::String | Kind::Bytes | Kind::Vector | Kind::BoolVector | Kind::CharTable) {
        return Err(Fault::Wrong("sequencep", sequence));
    }
    let n = index.integer_value().ok_or(Fault::Wrong("fixnump", index))?;
    if kind == Kind::CharTable {
        let n = usize::try_from(n).ok().filter(|&n| n <= crate::lstring::MAX_CHAR as usize)
            .ok_or(Fault::Wrong("characterp", index))?;
        return cx.char_table_element(sequence, n);
    }
    let n = usize::try_from(n).map_err(|_| Fault::Range(sequence, index))?;
    if kind == Kind::BoolVector {
        return cx.bool_vector(sequence)?.get(n)
            .map(|b| if b { Value::TRUE } else { Value::NIL })
            .ok_or(Fault::Range(sequence, index));
    }
    if kind == Kind::Vector {
        return cx.vector_get(sequence, index);
    }
    let string = cx.string_view(sequence)?;
    let code = string.chars().nth(n).ok_or(Fault::Range(sequence, index))?;
    Ok(Value::integer(i64::from(code)).unwrap())
}}

// Fnthcdr's bignum correction (fns.c:1813): reduce the original integer
// modulo the real cycle, never truncate it to a machine word. Values remain
// leaf-local and allocation cannot collect while the visited map is live.
fn bignum_nthcdr(cx: &Leaf<'_>, mut n: num_bigint::BigInt, list: Value) -> Out {
    if n.sign() == num_bigint::Sign::Minus { return Ok(list); }
    let mut tail = list;
    let mut seen = std::collections::HashMap::new();
    while n != num_bigint::BigInt::from(0) {
        let Some((_, cdr)) = cx.pair(tail) else {
            return if tail == Value::NIL { Ok(tail) } else { Err(Fault::Wrong("listp", list)) };
        };
        if let Some(&first) = seen.get(&tail.0) {
            let period = seen.len() - first;
            let steps = usize::try_from(n % num_bigint::BigInt::from(period))
                .expect("cycle remainder fits its usize period");
            for _ in 0..steps { tail = cx.pair(tail).unwrap().1; }
            return Ok(tail);
        }
        seen.insert(tail.0, seen.len());
        tail = cdr;
        n -= 1;
    }
    Ok(tail)
}
