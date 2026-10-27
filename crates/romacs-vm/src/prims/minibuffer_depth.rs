use super::*;

defsubr! { name = "minibuffer-depth", gnu = "minibuf.c:2241", effect = Reads,
fn minibuffer_depth(cx) {
    let level = cx.intern("romacs--minibuf-level");
    Ok(cx.variable_value(level)?.unwrap_or(Value::integer(0).unwrap()))
}}

// command_loop_level + minibuf_level: VM2 has no recursive edit, so the
// command loop level is 0.
defsubr! { name = "recursion-depth", gnu = "keyboard.c:12350", effect = Reads,
fn recursion_depth(cx) {
    let level = cx.intern("romacs--minibuf-level");
    Ok(cx.variable_value(level)?.unwrap_or(Value::integer(0).unwrap()))
}}
