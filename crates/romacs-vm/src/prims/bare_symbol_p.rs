//! GNU data.c:352; VM2 symbols are bare (positioned objects: VM2-SYMPOS1).
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "bare-symbol-p", gnu = "data.c:352", effect = Pure,
fn bare_symbol_p(cx, object: Value) {
    Ok(if matches!(cx.kind(object), Kind::Symbol) { Value::TRUE } else { Value::NIL })
}}
