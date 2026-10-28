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

// VM2 has no indirect buffers (no make-indirect-buffer), so no buffer has
// a base buffer.
defsubr! { name = "buffer-base-buffer", gnu = "buffer.c:1342", effect = Reads,
fn buffer_base_buffer(cx; optional buffer: Value) {
    cx.decode_buffer(buffer)?;
    Ok(Value::NIL)
}}

/// GNU Fbarf_if_buffer_read_only (buffer.c:2465): signal `buffer-read-only'
/// in a read-only current buffer, unless `inhibit-read-only' is set or the
/// text at POSITION (point when nil) has an `inhibit-read-only' property.
pub(crate) fn barf_if_read_only(cx: &mut crate::prim::leaf::Leaf<'_>, position: Value) -> std::result::Result<(), crate::prim::Fault> {
    use crate::prim::Fault;
    let position = if position == Value::NIL {
        cx.point()?
    } else {
        position.integer_value().ok_or(Fault::Wrong("fixnump", position))?
    };
    let read_only = super::fileio::var(cx, "buffer-read-only")?;
    if read_only != Value::NIL
        && super::fileio::var(cx, "inhibit-read-only")? == Value::NIL
        && {
            let buffer = cx.decode_buffer(Value::NIL)?;
            let property = cx.intern("inhibit-read-only");
            cx.text_property_at(buffer, position, property)? == Value::NIL
        }
    {
        let buffer = cx.decode_buffer(Value::NIL)?;
        let condition = cx.intern("buffer-read-only");
        let data = cx.cons(buffer, Value::NIL);
        return Err(Fault::Signal(condition, data));
    }
    Ok(())
}

defsubr! { name = "barf-if-buffer-read-only", gnu = "buffer.c:2465", effect = Reads,
fn barf_if_buffer_read_only(cx; optional position: Value) {
    barf_if_read_only(cx, position)?;
    Ok(Value::NIL)
}}
