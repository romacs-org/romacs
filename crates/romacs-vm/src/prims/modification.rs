//! GNU buffer.c:1518-1687. Counter state is integer-only and moves with buffers.
use super::*;
defsubr! { name = "buffer-modified-p", gnu = "buffer.c:1518", effect = Reads,
fn buffer_modified_p(cx; optional buffer: Value) {
    let state = cx.buffer_modification(buffer)?;
    if state.save_modiff >= state.modiff { return Ok(Value::NIL); }
    Ok(if state.autosave_modiff == state.modiff { cx.intern("autosaved") } else { Value::TRUE })
}}
defsubr! { name = "buffer-modified-tick", gnu = "buffer.c:1651", effect = Reads,
fn buffer_modified_tick(cx; optional buffer: Value) {
    Value::integer(cx.buffer_modification(buffer)?.modiff as i64).map_err(Fault::Error)
}}
defsubr! { name = "buffer-chars-modified-tick", gnu = "buffer.c:1674", effect = Reads,
fn buffer_chars_modified_tick(cx; optional buffer: Value) {
    Value::integer(cx.buffer_modification(buffer)?.chars_modiff as i64).map_err(Fault::Error)
}}
defsubr! { name = "set-buffer-modified-p", gnu = "buffer.c:1561", effect = Global,
fn set_buffer_modified_p(cx, flag: Value) {
    cx.set_buffer_modified(flag, true)?;
    Ok(Value::NIL)
}}
defsubr! { name = "restore-buffer-modified-p", gnu = "buffer.c:1588", effect = Global,
fn restore_buffer_modified_p(cx, flag: Value) {
    cx.set_buffer_modified(flag, false)?;
    Ok(flag)
}}
