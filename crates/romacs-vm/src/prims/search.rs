//! GNU search.c match-register translation, in character coordinates.
use super::*;

defsubr! { name = "match-data--translate", gnu = "search.c:3062", effect = Global,
fn match_data_translate(cx, n: Value) {
    let delta = n.integer_value().ok_or(Fault::Wrong("fixnump", n))?;
    cx.translate_match_data(delta);
    Ok(Value::NIL)
}}

defsubr! { name = "replace-match", gnu = "search.c:2344", effect = Global,
fn replace_match(cx, newtext: Value; optional fixedcase: Value, literal: Value, string: Value, subexp: Value) {
    cx.replace_match(newtext, fixedcase, literal, string, subexp)
}}

// fns.c searches encoded bytes after reconciling the strings' multibyteness.
// The return value and START-POS are nevertheless character coordinates.
defsubr! { name = "string-search", gnu = "fns.c:6576", effect = Pure,
fn string_search(cx, needle: Value, haystack: Value; optional start_pos: Value) {
    let needle = cx.string_view(needle)?;
    let haystack = cx.string_view(haystack)?;
    let start = if start_pos == Value::NIL { 0 } else {
        start_pos.integer_value().ok_or(Fault::Wrong("fixnump", start_pos))?
    };
    if start < 0 || start as usize > haystack.len() {
        return Err(Fault::Range1(start_pos));
    }
    let start = start as usize;
    if needle.len() > haystack.len() - start { return Ok(Value::NIL); }
    let mut pattern = needle.bytes().to_vec();
    if haystack.is_multibyte() && !needle.is_multibyte() && !pattern.is_ascii() {
        pattern.clear();
        for code in needle.chars() {
            let code = if code >= 128 { code + crate::lstring::BYTE8_OFFSET } else { code };
            let mut encoded = [0; 5];
            let width = crate::lstring::char_string(code, &mut encoded);
            pattern.extend_from_slice(&encoded[..width]);
        }
    } else if !haystack.is_multibyte() && needle.is_multibyte() {
        pattern.clear();
        for code in needle.chars() {
            if code < 128 { pattern.push(code as u8); }
            else if code >= crate::lstring::BYTE8_OFFSET + 128 {
                pattern.push((code - crate::lstring::BYTE8_OFFSET) as u8);
            } else { return Ok(Value::NIL); }
        }
    }
    // Build boundaries from the canonical GNU decoder, not Rust UTF-8:
    // raw-byte characters, surrogates and five-byte characters are valid here.
    let mut boundaries = Vec::with_capacity(haystack.len() + 1);
    let mut byte = 0;
    boundaries.push(byte);
    for code in haystack.chars() {
        byte += if haystack.is_multibyte() {
            crate::lstring::char_string(code, &mut [0; 5])
        } else { 1 };
        boundaries.push(byte);
    }
    let start_byte = boundaries[start];
    let found = if pattern.is_empty() { Some(start_byte) } else {
        haystack.bytes()[start_byte..].windows(pattern.len())
            .position(|window| window == pattern).map(|offset| start_byte + offset)
    };
    match found {
        None => Ok(Value::NIL),
        Some(byte) => {
            let character = boundaries.partition_point(|&boundary| boundary <= byte) - 1;
            Ok(Value::integer(character as i64).unwrap())
        }
    }
}}

