use super::*;

defsubr! { name = "minibuffer-depth", gnu = "minibuf.c:2241", effect = Reads,
fn minibuffer_depth(cx) {
    let level = cx.intern("romacs--minibuf-level");
    Ok(cx.variable_value(level)?.unwrap_or(Value::integer(0).unwrap()))
}}
