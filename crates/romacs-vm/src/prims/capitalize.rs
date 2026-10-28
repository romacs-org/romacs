//! GNU casefiddle.c:capitalize. Default case mappings are oracle-exported;
//! word membership comes from the current buffer's syntax table.
use super::*;
use crate::lstring::{char_string, BYTE8_OFFSET};

include!("capitalize-tables.in");

fn mapping(code: u32) -> Option<&'static (u32, u32, &'static [u32], &'static [u32])> {
    CASES.binary_search_by_key(&code, |row| row.0).ok().map(|i| &CASES[i])
}

pub(crate) fn capitalized(cx: &mut Leaf<'_>, object: Value, initials: bool) -> Out {
    if let Some(n) = object.integer_value().filter(|n| *n >= 0) {
        // casefiddle.c narrows XFIXNAT to int before checking the range.
        let ch = n as i32;
        if !(0..=0x0fff_ffff).contains(&ch) { return Ok(object); }
        let flags = ch as u32 & 0x0fc0_0000;
        let code = ch as u32 & !0x0fc0_0000;
        let multibyte = cx.casing_multibyte()? || code >= 256;
        let code = if !multibyte && (128..256).contains(&code) {
            code + BYTE8_OFFSET
        } else { code };
        let cased = word_start(cx, code)?;
        if cased == code { return Ok(object); }
        let cased = if !multibyte && cased >= BYTE8_OFFSET { cased - BYTE8_OFFSET } else { cased };
        return Ok(Value::integer(i64::from(cased | flags)).unwrap());
    }
    let view = cx.string_view(object).map_err(|_| Fault::Wrong("char-or-string-p", object))?;
    if view.chars == 0 { return Ok(object); }
    let multibyte = view.is_multibyte();
    let chars: Vec<u32> = view.chars().map(|c| if !multibyte && c >= 128 { c + BYTE8_OFFSET } else { c }).collect();
    let mut bytes = Vec::new();
    for (&code, out) in chars.iter().zip(capitalized_chars(cx, &chars, None, initials)?) {
        for c in out {
            if multibyte {
                let mut encoded = [0; 5];
                let n = char_string(c, &mut encoded);
                bytes.extend_from_slice(&encoded[..n]);
            } else {
                bytes.push(Leaf::byte_case_result(c, if code >= BYTE8_OFFSET { code - BYTE8_OFFSET } else { code }, false));
            }
        }
    }
    // GNU's multibyte path allocates a property-free string; its unibyte
    // path uses copy-sequence and retains properties.
    Ok(if multibyte { cx.make_string(bytes, true) }
       else { cx.buffer_name_string(object, bytes, false) })
}

/// case_character over CHARS for `capitalize', or `upcase-initials' when
/// INITIALS: what each character becomes. In a buffer, a character with
/// the syntax prefix flag (PREFIXES) starts no word.
pub(crate) fn capitalized_chars(cx: &mut Leaf<'_>, chars: &[u32], prefixes: Option<&[bool]>, initials: bool) -> std::result::Result<Vec<Vec<u32>>, Fault> {
    let words = word_flags(cx, chars)?;
    let mut inword = false;
    let mut cased = Vec::with_capacity(chars.len());
    for (i, &code) in chars.iter().enumerate() {
        // case_character: a special casing of several characters first,
        // then the case table (in a word) or the titlecase (at its start).
        let special = |full: &'static [u32]| (full.len() > 1).then(|| full.to_vec());
        let row = mapping(code);
        let out = if inword && initials { vec![code] }
            else if !initials && inword && code == 0x3a3 && !words.get(i + 1).copied().unwrap_or(false) { vec![0x3c2] }
            else if inword {
                row.and_then(|r| special(r.3)).map_or_else(|| cx.case_mapping_character(code, true).map(|c| vec![c]), Ok)?
            } else {
                row.and_then(|r| special(r.2)).map_or_else(|| word_start(cx, code).map(|c| vec![c]), Ok)?
            };
        cased.push(out);
        inword = words[i] && (inword || !prefixes.is_some_and(|p| p[i]));
    }
    Ok(cased)
}

/// Whether each of CHARS is a word character to casefiddle.c: the
/// current syntax table, else GNU's standard syntax.
pub(crate) fn word_flags(cx: &mut Leaf<'_>, chars: &[u32]) -> std::result::Result<Vec<bool>, Fault> {
    let defaults: Vec<u8> = chars.iter().map(|&code| {
        let i = WORD_RANGES.partition_point(|row| row.1 < code);
        WORD_RANGES[i].2
    }).collect();
    cx.casing_word_flags(chars, &defaults)
}

defsubr! { name = "capitalize", gnu = "casefiddle.c:400", effect = Reads,
fn capitalize(cx, object: Value) {
    capitalized(cx, object, false)
}}

defsubr! { name = "upcase-initials", gnu = "casefiddle.c:418", effect = Reads,
fn upcase_initials(cx, object: Value) {
    capitalized(cx, object, true)
}}

/// case_character's one-character titlecase of CODE at a word start: its
/// titlecase property when that changes it, a titlecase letter itself
/// (its uppercase differs), else the current upcase table.
fn word_start(cx: &mut Leaf<'_>, code: u32) -> std::result::Result<u32, Fault> {
    match mapping(code) {
        Some(row) if row.1 != code => Ok(row.1),
        _ if Leaf::unicode_case(code, false) != code => Ok(code),
        _ => cx.case_mapping_character(code, false),
    }
}
