//! GNU data.c Finteger_or_marker_p: exact object-kind predicate.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "integer-or-marker-p", gnu = "data.c:579", effect = Pure,
fn integer_or_marker_p(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Integer | Kind::Bignum | Kind::Marker);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
