use super::*;

defsubr! { name = "subr-arity", gnu = "data.c:1038", effect = Pure,
fn subr_arity(cx, subr: Value) {
    let (minimum, maximum) = crate::prim::arity(subr).ok_or(Fault::Wrong("subrp", subr))?;
    // A special form's arity is (MIN . unevalled).
    let special = crate::prim::name(subr).and_then(super::special_forms::special_form_min);
    let minimum = Value::integer(special.unwrap_or(minimum) as i64).unwrap();
    let maximum = match maximum {
        _ if special.is_some() => cx.intern("unevalled"),
        Some(maximum) => Value::integer(maximum as i64).unwrap(),
        None => cx.intern("many"),
    };
    Ok(cx.cons(minimum, maximum))
}}

defsubr! { name = "subr-name", gnu = "data.c:1056", effect = Pure,
fn subr_name(cx, subr: Value) {
    let name = crate::prim::name(subr).ok_or(Fault::Wrong("subrp", subr))?;
    Ok(cx.byte_string(name.as_bytes()))
}}
