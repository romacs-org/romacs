//! Literal regexp quoting, following GNU search.c.
use super::*;

defsubr! { name = "regexp-quote", gnu = "search.c:3177", effect = Pure,
fn regexp_quote(cx, string: Value) {
    let view = cx.string_view(string)?;
    let mut bytes = Vec::with_capacity(view.bytes.len());
    let mut changed = false;
    for &byte in &view.bytes {
        if matches!(byte, b'[' | b'*' | b'.' | b'\\' | b'?' | b'+' | b'^' | b'$') {
            bytes.push(b'\\');
            changed = true;
        }
        bytes.push(byte);
    }
    // GNU preserves identity and properties when no quoting was necessary.
    if !changed { return Ok(string); }
    Ok(cx.make_string(bytes, view.multibyte))
}}
