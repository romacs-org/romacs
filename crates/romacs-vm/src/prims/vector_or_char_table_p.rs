//! GNU data.c Fvector_or_char_table_p: exact represented object kinds.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "vector-or-char-table-p", gnu = "data.c:444", effect = Pure,
fn vector_or_char_table_p(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::Vector | Kind::CharTable | Kind::SyntaxTable);
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
