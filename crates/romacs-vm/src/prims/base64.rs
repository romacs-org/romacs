//! GNU fns.c base64 string coding (RFC 4648, with the URL variant).
use super::*;

/// MIME_LINE_LENGTH: base64-encode-string breaks lines at 76 characters.
const LINE_GROUPS: usize = 76 / 4;

/// fns.c base64_char_to_value: -1 ignorable whitespace, 0 invalid, else
/// one more than the digit's value.
fn digit(c: u8, url: bool) -> i32 {
    match c {
        b'\t' | b'\n' | 0x0c | b'\r' | b' ' => -1,
        b'A'..=b'Z' => i32::from(c - b'A') + 1,
        b'a'..=b'z' => i32::from(c - b'a') + 27,
        b'0'..=b'9' => i32::from(c - b'0') + 53,
        b'+' if !url => 63,
        b'/' if !url => 64,
        b'-' if url => 63,
        b'_' if url => 64,
        _ => 0,
    }
}

fn alphabet(url: bool) -> &'static [u8; 64] {
    if url { b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_" }
    else { b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/" }
}

/// fns.c base64_encode_1 over octets.
fn encode(octets: &[u8], line_break: bool, pad: bool, url: bool) -> Vec<u8> {
    let table = alphabet(url);
    let mut out = Vec::with_capacity(octets.len() / 3 * 4 + 4);
    for (group, chunk) in octets.chunks(3).enumerate() {
        if line_break && group > 0 && group % LINE_GROUPS == 0 { out.push(b'\n'); }
        let value = chunk.iter().enumerate().fold(0u32, |v, (i, &b)| v | u32::from(b) << (16 - 8 * i));
        let digits = chunk.len() + 1;
        for i in 0..4 {
            if i < digits { out.push(table[(value >> (18 - 6 * i) & 63) as usize]); }
            else if pad { out.push(b'='); }
        }
    }
    out
}

/// fns.c base64_decode_1 (unibyte result), None for invalid data.
fn decode(input: &[u8], url: bool, ignore_invalid: bool) -> Option<Vec<u8>> {
    let mut bytes = input.iter().copied();
    // The next significant byte: whitespace, and with IGNORE-INVALID any
    // invalid byte, is skipped.
    let mut next = |bytes: &mut std::iter::Copied<std::slice::Iter<'_, u8>>| loop {
        let c = bytes.next()?;
        let v = digit(c, url);
        if !(v < 0 || (v == 0 && ignore_invalid)) { return Some((c, v)); }
    };
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    let short = |out: Vec<u8>| if url || ignore_invalid { Some(out) } else { None };
    loop {
        let Some((_, v)) = next(&mut bytes) else { return Some(out) };
        if v == 0 { return None; }
        let mut value = ((v - 1) as u32) << 18;
        let (_, v) = next(&mut bytes)?;
        if v == 0 { return None; }
        value += ((v - 1) as u32) << 12;
        out.push((value >> 16) as u8);
        let Some((c, v)) = next(&mut bytes) else { return short(out) };
        if c == b'=' {
            // "==": the second pad must follow, past whitespace only.
            loop {
                let c = bytes.next()?;
                if digit(c, url) >= 0 { if c != b'=' { return None; } break; }
            }
            continue;
        }
        if v == 0 { return None; }
        value += ((v - 1) as u32) << 6;
        out.push((value >> 8) as u8);
        let Some((c, v)) = next(&mut bytes) else { return short(out) };
        if c == b'=' { continue; }
        if v == 0 { return None; }
        value += (v - 1) as u32;
        out.push(value as u8);
    }
}

/// The octets of STRING: raw bytes as themselves, a multibyte non-ASCII
/// character refused.
fn octets(cx: &mut Leaf<'_>, string: Value) -> std::result::Result<Vec<u8>, Fault> {
    let view = cx.string_view(string)?;
    if !view.is_multibyte() { return Ok(view.bytes().to_vec()); }
    view.chars().map(|c| {
        if c >= crate::lstring::BYTE8_OFFSET + 128 { Ok((c - crate::lstring::BYTE8_OFFSET) as u8) }
        else if c < 128 { Ok(c as u8) }
        else { Err(Fault::Error("Multibyte character in data for base64 encoding".into())) }
    }).collect()
}

defsubr! { name = "base64-encode-string", gnu = "fns.c:4168", effect = Pure,
fn base64_encode_string(cx, string: Value; optional no_line_break: Value) {
    let octets = octets(cx, string)?;
    Ok(cx.make_string(encode(&octets, no_line_break == Value::NIL, true, false), false))
}}

defsubr! { name = "base64url-encode-string", gnu = "fns.c:4179", effect = Pure,
fn base64url_encode_string(cx, string: Value; optional no_pad: Value) {
    let octets = octets(cx, string)?;
    Ok(cx.make_string(encode(&octets, false, no_pad == Value::NIL, true), false))
}}

defsubr! { name = "base64-decode-string", gnu = "fns.c:4409", effect = Pure,
fn base64_decode_string(cx, string: Value; optional base64url: Value, ignore_invalid: Value) {
    let bytes = cx.string_view(string)?.bytes().to_vec();
    let decoded = decode(&bytes, base64url != Value::NIL, ignore_invalid != Value::NIL)
        .ok_or_else(|| Fault::Error("Invalid base64 data".into()))?;
    Ok(cx.make_string(decoded, false))
}}
