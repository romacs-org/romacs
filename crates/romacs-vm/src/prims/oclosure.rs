use super::*;

defsubr! { name = "romacs--oclosure-type", gnu = "lisp/emacs-lisp/oclosure.el:479", effect = Reads,
fn oclosure_type(cx, object: Value) {
    let document = cx.closure_document(object);
    Ok(if cx.kind(document) == crate::prim::Kind::Symbol { document } else { Value::NIL })
}}
