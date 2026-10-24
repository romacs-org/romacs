use super::*;

defsubr! { name = "local-variable-p", gnu = "data.c:2416", effect = Reads,
fn local_variable_p(cx, variable: Value; optional buffer: Value) {
    let buffer = cx.decode_buffer(buffer)?;
    Ok(if cx.has_buffer_local(variable, buffer)? { Value::TRUE } else { Value::NIL })
}}
defsubr! { name = "kill-local-variable", gnu = "data.c:2350", effect = Global,
fn kill_local_variable(cx, variable: Value) { cx.kill_local(variable) }}
defsubr! { name = "buffer-local-value", gnu = "buffer.c:1353", effect = Reads,
fn buffer_local_value(cx, variable: Value, buffer: Value) {
    cx.buffer_local_read(variable, buffer)
}}
defsubr! { name = "buffer-local-variables", gnu = "buffer.c:1485", effect = Reads,
fn buffer_local_variables(cx; optional buffer: Value) {
    let buffer = cx.decode_buffer(buffer)?;
    cx.buffer_local_inventory(buffer)
}}
defsubr! { name = "kill-all-local-variables", gnu = "buffer.c:3031", effect = Global,
fn kill_all_local_variables(cx; optional permanent: Value) -> Step {
    let f = cx.intern("run-hooks");
    let hook = cx.intern("change-major-mode-hook");
    Ok(Step::Call { f, args: vec![hook], then: reset_locals, saved: vec![permanent] })
}}
fn reset_locals(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    cx.reset_buffer_locals(saved[0]).map(Step::Done)
}
