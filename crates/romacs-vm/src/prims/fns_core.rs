//! Core sequence primitives moved off the legacy enum path, following GNU
//! src/fns.c. Verified by primh (`tools/primh/fns-core-specs`) and by the bulk
//! `core` corpus (`tools/primh/bulk/core.corpus.json`). `nreverse` cannot live
//! here: see the header of `src/tails.rs`.
use super::*;

defsubr! { name = "substring", gnu = "fns.c:1579", effect = Pure,
fn substring(cx, string: Value; optional from: Value, to: Value) {
    // CHECK_VECTOR_OR_STRING, then validate_subarray (fns.c:1545-1577).
    let size = if let Ok(n) = cx.vector_len(string) {
        n as i64
    } else if let Ok(view) = cx.string_view(string) {
        view.chars as i64
    } else {
        return Err(Fault::Wrong("arrayp", string));
    };
    let bound = |v: Value, default: i64| -> std::result::Result<i64, Fault> {
        if v == Value::NIL {
            return Ok(default);
        }
        let n = v.integer_value().ok_or(Fault::Wrong("integerp", v))?;
        Ok(if n < 0 { n + size } else { n })
    };
    let f = bound(from, 0)?;
    let t = bound(to, size)?;
    if !(0 <= f && f <= t && t <= size) {
        return Err(Fault::Range3(string, from, to));
    }
    if let Ok(values) = cx.vector_values(string) {
        return Ok(cx.vector(&values[f as usize..t as usize]));
    }
    cx.string_slice(string, f as usize, t as usize)
}}
