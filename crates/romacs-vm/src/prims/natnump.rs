//! GNU data.c: integer sign, without converting bignums through floating point.
use super::*;

defsubr! { name = "natnump", gnu = "data.c:588", effect = Pure,
fn natnump(cx, object: Value) {
    let nonnegative = cx.bigint_value(object)
        .is_some_and(|n| n.sign() != num_bigint::Sign::Minus);
    Ok(if nonnegative { Value::TRUE } else { Value::NIL })
}}
