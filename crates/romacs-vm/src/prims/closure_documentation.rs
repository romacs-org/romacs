use super::*;

defsubr! { name = "romacs--closure-document", gnu = "simple.el:2739", effect = Reads,
fn closure_document(cx, function: Value) {
    if cx.kind(function) != crate::prim::Kind::Closure {
        return Err(Fault::Wrong("closurep", function));
    }
    Ok(cx.closure_document(function))
}}

defsubr! { name = "romacs--closure-arity", gnu = "eval.c:3537", effect = Reads,
fn closure_arity(cx, function: Value) {
    let (required, positional, rest) = cx.closure_arity(function).ok_or(Fault::Wrong("closurep", function))?;
    let maximum = if rest { cx.intern("many") } else { Value::integer(positional as i64).unwrap() };
    Ok(cx.cons(Value::integer(required as i64).unwrap(), maximum))
}}
