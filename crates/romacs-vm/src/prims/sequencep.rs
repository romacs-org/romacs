//! GNU data.c Fsequencep: exact represented object kinds.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "sequencep", gnu = "data.c:472", effect = Pure,
fn sequencep(cx, object: Value) {
    let answer = object == Value::NIL || matches!(cx.kind(object), Kind::Cons | Kind::Vector | Kind::BoolVector | Kind::CharTable | Kind::SyntaxTable | Kind::String | Kind::Bytes);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
