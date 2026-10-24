use crate::Value;
// GNU buffer.c leaves; metadata lines stay stable for the recorded specs.
use crate::prim::leaf::Kind;

defsubr! { name = "overlayp", gnu = "buffer.c:3640", effect = Reads,
fn overlayp(cx, object: Value) {
    Ok(if cx.kind(object) == Kind::Overlay { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "buffer-live-p", gnu = "buffer.c:415", effect = Reads,
fn buffer_live_p(cx, object: Value) {
    Ok(if cx.buffer_state(object).is_some_and(|b| b.live) { Value::TRUE } else { Value::NIL })
}}


defsubr! { name = "buffer-name", gnu = "buffer.c:1314", effect = Reads,
fn buffer_name(cx; optional buffer: Value) {
    let buffer = cx.decode_buffer(buffer)?; let name = cx.buffer_state(buffer).unwrap().name.clone(); Ok(match name { Some(name) => cx.string(&name), None => Value::NIL })
}}

defsubr! { name = "buffer-file-name", gnu = "buffer.c:1334", effect = Reads,
fn buffer_file_name(cx; optional buffer: Value) {
    let buffer = cx.decode_buffer(buffer)?;
    let symbol = cx.intern("buffer-file-name");
    cx.buffer_local_read(symbol, buffer)
}}

defsubr! { name = "char-before", gnu = "editfns.c:1070", effect = Reads,
fn char_before(cx; optional position: Value) {
    cx.character_before(position)
}}
