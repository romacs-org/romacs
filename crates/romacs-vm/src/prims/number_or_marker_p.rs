//! GNU data.c Fnumber_or_marker_p: exact object-kind predicate.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "number-or-marker-p", gnu = "data.c:609", effect = Pure,
fn number_or_marker_p(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Integer | Kind::Bignum | Kind::Float | Kind::Marker);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
