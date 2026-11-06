//! GNU fns.c sequence access.
use super::*;
use crate::prim::leaf::Kind;

// GNU Felt accepts ARRAYP, which excludes records and byte-code objects even
// though Faref itself accepts them.
defsubr! { name = "elt", gnu = "fns.c:1853", effect = Pure,
fn elt(cx, sequence: Value, index: Value) {
    if sequence == Value::NIL || cx.pair(sequence).is_some() {
        let tail = super::fns::nthcdr(cx, index, sequence)?;
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
