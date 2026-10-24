//! GNU data.c Fmarkerp: exact represented object kinds.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "markerp", gnu = "data.c:490", effect = Pure,
fn markerp(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Marker);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
