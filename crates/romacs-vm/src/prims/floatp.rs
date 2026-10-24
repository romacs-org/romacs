//! GNU data.c Ffloatp: exact object-kind predicate.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "floatp", gnu = "data.c:619", effect = Pure,
fn floatp(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Float);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
