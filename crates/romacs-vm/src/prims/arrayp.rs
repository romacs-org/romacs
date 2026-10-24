//! GNU data.c Farrayp: exact represented object kinds.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "arrayp", gnu = "data.c:463", effect = Pure,
fn arrayp(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Vector | Kind::BoolVector | Kind::CharTable | Kind::SyntaxTable | Kind::String | Kind::Bytes);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
