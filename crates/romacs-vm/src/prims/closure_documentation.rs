use super::*;

defsubr! { name = "romacs--closure-document", gnu = "simple.el:2739", effect = Reads,
fn closure_document(cx, function: Value) {
    if cx.kind(function) != crate::prim::Kind::Closure {
        return Err(Fault::Wrong("closurep", function));
    }
    Ok(cx.closure_document(function))
}}
