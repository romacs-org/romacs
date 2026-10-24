use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "documentation-stringp", gnu = "doc.c:319", effect = Pure,
fn documentation_stringp(cx, object: Value) {
    let valid = matches!(cx.kind(object), Kind::String | Kind::Bytes) || object.as_integer().is_ok()
        || cx.pair(object).is_some_and(|(car, cdr)| matches!(cx.kind(car), Kind::String | Kind::Bytes) && cdr.as_integer().is_ok());
    Ok(if valid { Value::TRUE } else { Value::NIL })
}}
