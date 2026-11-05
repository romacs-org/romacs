//! GNU json.c: native JSON serialization and parsing (`prim/leaf/json.rs`).
use super::*;

defsubr! { name = "json-serialize", gnu = "json.c:589", effect = Reads,
fn json_serialize(cx, object: Value; rest args) {
    let bytes = cx.json_serialize_bytes(object, args)?;
    Ok(cx.make_string(bytes, false))
}}

defsubr! { name = "json-insert", gnu = "json.c:627", effect = Global,
fn json_insert(cx, object: Value; rest args) -> Step {
    let bytes = cx.json_serialize_bytes(object, args)?;
    // Characters into a multibyte buffer, UTF-8 bytes into a unibyte one.
    let multibyte = cx.buffer_multibyte()?;
    let text = cx.make_string(bytes, multibyte);
    Ok(Step::Call { f: cx.intern("insert"), args: vec![text], then: inserted, saved: vec![] })
}}
fn inserted(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }

defsubr! { name = "json-parse-string", gnu = "json.c:1665", effect = Pure,
fn json_parse_string(cx, string: Value; rest args) {
    cx.json_parse_string(string, args)
}}

defsubr! { name = "json-parse-buffer", gnu = "json.c:1733", effect = Global,
fn json_parse_buffer(cx; rest args) {
    cx.json_parse_buffer(args)
}}
