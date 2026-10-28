//! GNU character.c `char-width' and `string-width' (`prim/leaf/width.rs').
//! Automatic compositions (composition-function-table) are not applied:
//! DIVERGENCES.md VM2-WIDTH1.
use super::*;

defsubr! { name = "char-width", gnu = "character.c:262", effect = Reads,
fn char_width(cx, ch: Value) {
    let c = ch.integer_value().filter(|c| (0..=0x3fffff).contains(c)).ok_or(Fault::Wrong("characterp", ch))?;
    cx.char_width(c as u32)
}}

defsubr! { name = "string-width", gnu = "character.c:466", effect = Reads,
fn string_width(cx, string: Value; optional from: Value, to: Value) {
    // CHECK_STRING, then validate_subarray (fns.c).
    let size = cx.string_view(string).map_err(|_| Fault::Wrong("stringp", string))?.chars as i64;
    let bound = |v: Value, default: i64| -> std::result::Result<i64, Fault> {
        if v == Value::NIL { return Ok(default); }
        let n = v.integer_value().ok_or(Fault::Wrong("integerp", v))?;
        Ok(if n < 0 { n + size } else { n })
    };
    let (f, t) = (bound(from, 0)?, bound(to, size)?);
    if !(0 <= f && f <= t && t <= size) {
        return Err(Fault::Range3(string, from, to));
    }
    cx.string_width(string, f as usize, t as usize)
}}
