//! GNU process.c. Romacs has no subprocess objects (DIVERGENCES.md
//! VM2-PROC1): no buffer has a process.
use super::*;

defsubr! { name = "get-buffer-process", gnu = "process.c:8422", effect = Reads,
fn get_buffer_process(cx, buffer: Value) {
    // Fget_buffer checks BUFFER before the (empty) process list is searched.
    match cx.kind(buffer) {
        Kind::String | Kind::Bytes | Kind::Buffer | Kind::ByteBuffer => Ok(Value::NIL),
        _ if buffer == Value::NIL => Ok(Value::NIL),
        _ => Err(Fault::Wrong("stringp", buffer)),
    }
}}
