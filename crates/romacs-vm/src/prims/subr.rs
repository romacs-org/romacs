use super::*;

defsubr! { name = "subr-arity", gnu = "data.c:1038", effect = Pure,
fn subr_arity(cx, subr: Value) {
    let (minimum, maximum) = crate::prim::arity(subr).ok_or(Fault::Wrong("subrp", subr))?;
    let minimum = Value::integer(minimum as i64).unwrap();
    let maximum = match maximum {
        Some(maximum) => Value::integer(maximum as i64).unwrap(),
        None => cx.intern("many"),
    };
    Ok(cx.cons(minimum, maximum))
}}
