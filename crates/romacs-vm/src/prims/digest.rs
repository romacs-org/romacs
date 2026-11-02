//! fns.c's digests (`crate::digest'); `secure-hash' and `md5' choose and
//! encode their data in Lisp, as extract_data_from_object does in C
//! (rlisp/vm/prelude.el).
use super::*;

// secure_hash after extract_data_from_object: BYTES is the encoded data.
// An unknown ALGORITHM signals as GNU does, after the data was extracted.
defsubr! { name = "romacs--secure-hash", gnu = "fns.c:6289", effect = Pure,
fn romacs_secure_hash(cx, algorithm: Value, bytes: Value; optional binary: Value) {
    let name = cx.symbol_name(algorithm)?;
    let view = cx.string_view(bytes).map_err(|_| Fault::Wrong("stringp", bytes))?;
    if view.multibyte && !view.bytes().is_ascii() {
        return Err(Fault::Error("romacs--secure-hash needs encoded (unibyte) data".into()));
    }
    let data = view.bytes().to_vec();
    // GNU compares ALGORITHM with EQ: an uninterned namesake is no algorithm.
    let digest = if algorithm == cx.intern(&name) { crate::digest::digest(&name, &data) } else { None };
    let Some(digest) = digest else {
        return Err(Fault::Error(format!("Invalid algorithm arg: {name}")));
    };
    if binary != Value::NIL { return Ok(cx.make_string(digest, false)); }
    Ok(cx.make_string(hex(&digest), false))
}}

// The SHA-1 of the whole text of BUFFER-OR-NAME, narrowing aside, in the
// buffer's internal form, so no coding system takes part.
defsubr! { name = "buffer-hash", gnu = "fns.c:6447", effect = Reads,
fn buffer_hash(cx; optional buffer_or_name: Value) {
    let buffer = if buffer_or_name == Value::NIL { cx.current_buffer()? } else { cx.lifecycle_get(buffer_or_name)? };
    if buffer == Value::NIL {
        // nsberror (buffer.c:408)
        return Err(Fault::Error(format!("No buffer named {}", cx.string_text(buffer_or_name)?)));
    }
    let bytes = cx.buffer_internal_bytes(buffer)?;
    Ok(cx.make_string(hex(&crate::digest::digest("sha1", &bytes).expect("sha1 is an algorithm")), false))
}}

fn hex(digest: &[u8]) -> Vec<u8> {
    digest.iter().map(|b| format!("{b:02x}")).collect::<String>().into_bytes()
}
