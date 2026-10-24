//! GNU src/data.c `Fkeywordp`.
use super::*;

defsubr! { name = "keywordp", gnu = "data.c:383", effect = Pure,
fn keywordp(cx, object: Value) {
    Ok(if is_keyword(cx, object) { Value::TRUE } else { Value::NIL })
}}

// GNU data.c:388: SYMBOLP, then `SREF (SYMBOL_NAME (object), 0) == ':'` (the
// first BYTE; an empty name reads its NUL terminator), then
// SYMBOL_INTERNED_IN_INITIAL_OBARRAY_P. VM2 has one obarray, and a symbol is
// interned in it exactly when that obarray maps its name back to the same
// object, so a `make-symbol' ":x" fails the last test as in GNU.
fn is_keyword(cx: &Leaf<'_>, object: Value) -> bool {
    let Ok(name) = cx.symbol_name(object) else { return false };
    name.as_bytes().first() == Some(&b':') && cx.obarray_lookup(&name) == Some(object)
}
