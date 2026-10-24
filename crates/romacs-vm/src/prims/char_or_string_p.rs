//! GNU data.c Fchar_or_string_p: exact object-kind predicate.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "char-or-string-p", gnu = "data.c:559", effect = Pure,
fn char_or_string_p(cx, object: Value) {
    let answer = matches!(cx.kind(object), Kind::String | Kind::Bytes) || object.integer_value().is_some_and(|n| (0..=0x3fffff).contains(&n));
    Ok(if answer { Value::TRUE } else { Value::NIL })
}}
