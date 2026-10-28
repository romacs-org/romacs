//! GNU search.c match-register translation, in character coordinates.
use super::*;

defsubr! { name = "match-data--translate", gnu = "search.c:3062", effect = Global,
fn match_data_translate(cx, n: Value) {
    let delta = n.integer_value().ok_or(Fault::Wrong("fixnump", n))?;
    cx.translate_match_data(delta);
    Ok(Value::NIL)
}}

fn fixnum(n: i64) -> Value { Value::integer(n).expect("buffer position") }

// A buffer replacement is replace_range with its hooks, undo and match-data
// adjustment (`romacs--replace-range'), the case conversion of the new text
// (`upcase-region' or `upcase-initials-region' from the match's adjusted
// start), then point after the new text. Registers between the steps:
//   [SUB, NEWPOINT, CASE]
defsubr! { name = "replace-match", gnu = "search.c:2344", effect = Global,
fn replace_match(cx, newtext: Value; optional fixedcase: Value, literal: Value, string: Value, subexp: Value) -> Step {
    if string != Value::NIL {
        return Ok(Step::Done(cx.replace_match(newtext, fixedcase, literal, string, subexp)?));
    }
    let (sub, start, end, case, text) = cx.buffer_replacement(newtext, fixedcase, literal, subexp)?;
    let newpoint = start + cx.string_view(text)?.chars;
    let args = vec![fixnum(start as i64), fixnum(end as i64), text, Value::TRUE];
    let saved = vec![fixnum(sub as i64), fixnum(newpoint as i64), fixnum(i64::from(case))];
    Ok(Step::Call { f: cx.intern("romacs--replace-range"), args, then: replaced, saved })
}}

fn replaced(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let function = match saved[2].integer_value() {
        Some(1) => "upcase-region",
        Some(2) => "upcase-initials-region",
        _ => return moved(cx, Value::NIL, saved),
    };
    let sub = saved[0].integer_value().expect("subexpression") as usize;
    // A hook that ran a search may have left SUB unmatched, which
    // validate_region refuses as GNU's -1 start.
    let start = cx.match_group(sub).map_or(-1, |(start, _)| start as i64);
    let args = vec![fixnum(start), saved[1], Value::NIL];
    Ok(Step::Call { f: cx.intern(function), args, then: moved, saved: saved.to_vec() })
}

// move_if_not_intangible: point to the end of the replacement. VM2 has no
// intangible text, so `inhibit-point-motion-hooks' changes nothing here.
fn moved(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (_, begv, zv) = cx.point_extent()?;
    cx.goto_char_position(saved[1].integer_value().expect("newpoint").clamp(begv, zv))?;
    Ok(Step::Done(Value::NIL))
}

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

