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
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok(cx.make_string(hex.into_bytes(), false))
}}
