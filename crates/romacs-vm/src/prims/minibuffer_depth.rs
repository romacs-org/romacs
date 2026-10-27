use super::*;

defsubr! { name = "minibuffer-depth", gnu = "minibuf.c:2241", effect = Reads,
fn minibuffer_depth(cx) {
    let level = cx.intern("romacs--minibuf-level");
    Ok(cx.variable_value(level)?.unwrap_or(Value::integer(0).unwrap()))
}}

// command_loop_level + minibuf_level, kept by the Lisp halves of the
// command loop (rlisp/vm/keyboard.el) and the minibuffer.
defsubr! { name = "recursion-depth", gnu = "keyboard.c:12350", effect = Reads,
fn recursion_depth(cx) {
    let mut depth = 0;
    for name in ["romacs--command-loop-level", "romacs--minibuf-level"] {
        let level = cx.intern(name);
        depth += cx.variable_value(level)?.and_then(|v| v.integer_value()).unwrap_or(0);
    }
    Ok(Value::integer(depth).unwrap())
}}
