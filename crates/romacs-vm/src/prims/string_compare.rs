//! GNU fns.c string comparison, including representation and range semantics.
use super::*;
use crate::prim::leaf::Span;
use std::cmp::Ordering;

defsubr! { name = "compare-strings", gnu = "fns.c:361", effect = Reads,
fn compare_strings(cx, first: Value, from: Value, to: Value, second: Value,
                   other_from: Value, other_to: Value; optional fold: Value) {
    let (length, other_length) = (chars(cx, first)?, chars(cx, second)?);
    let (start, end) = bounds(first, from, to, length)?;
    let (other_start, other_end) = bounds(second, other_from, other_to, other_length)?;
    let (a, b) = (Span { text: first, start, end }, Span { text: second, start: other_start, end: other_end });
    let (order, matched) = cx.compare_text(a, b, fold != Value::NIL)?;
    Ok(match order {
        Ordering::Equal => Value::TRUE,
        Ordering::Less => Value::integer(-((matched + 1) as i64)).unwrap(),
        Ordering::Greater => Value::integer((matched + 1) as i64).unwrap(),
    })
}}

/// STRING's length in characters.
pub(super) fn chars(cx: &Leaf<'_>, string: Value) -> std::result::Result<usize, Fault> {
    cx.string_chars(string).ok_or(Fault::Wrong("stringp", string))
}

fn bounds(
    string: Value,
    from: Value,
    to: Value,
    length: usize,
) -> std::result::Result<(usize, usize), Fault> {
    let size = length as i64;
    let position = |value: Value, default| {
        if value == Value::NIL {
            return Ok(default);
        }
        let n = value
            .integer_value()
            .ok_or(Fault::Wrong("integerp", value))?;
        Ok(if n < 0 { n + size } else { n })
    };
    let end = if to.integer_value().is_some_and(|n| n > size) {
        Value::integer(size).unwrap()
    } else {
        to
    };
    let start = position(from, 0)?;
    let finish = position(end, size)?;
    if !(0 <= start && start <= finish && finish <= size) {
        return Err(Fault::Range3(string, from, end));
    }
    Ok((start as usize, finish as usize))
}

/// The order of the first LENGTH characters of the strings A and B, and
/// how many of them match.
pub(super) fn prefix(
    cx: &mut Leaf<'_>,
    a: Value,
    b: Value,
    length: usize,
    fold: bool,
) -> std::result::Result<(Ordering, usize), Fault> {
    chars(cx, a)?;
    chars(cx, b)?;
    cx.compare_text(Span { text: a, start: 0, end: length }, Span { text: b, start: 0, end: length }, fold)
}

defsubr! { name = "assoc-string", gnu = "minibuf.c:2203", effect = Reads,
fn assoc_string(cx, key: Value, list: Value; optional fold: Value) {
    let mut tail = list;
    let mut seen = std::collections::HashSet::new();
    while let Some((element, rest)) = cx.pair(tail) {
        if !seen.insert(tail.0) { return Err(Fault::Circular(list)); }
        // A symbol's name compares as its string (minibuf.c Fassoc_string).
        let candidate = cx.pair(element).map_or(element, |(car, _)| car);
        if cx.compared_text_p(candidate) {
            // GNU checks KEY only when an element's string is compared with it.
            if !cx.compared_text_p(key) { return Err(Fault::Wrong("stringp", key)); }
            let (order, _) = cx.compare_text(Span::whole(candidate), Span::whole(key), fold != Value::NIL)?;
            if order == Ordering::Equal { return Ok(element); }
        }
        tail = rest;
    }
    Ok(Value::NIL)
}}

defsubr! { name = "string-collate-lessp", gnu = "fns.c:603", effect = Reads,
fn string_collate_lessp(cx, first: Value, second: Value; optional locale: Value, ignore_case: Value) {
    Ok(if collate(cx, first, second, locale, ignore_case)?.is_lt() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "string-collate-equalp", gnu = "fns.c:650", effect = Reads,
fn string_collate_equalp(cx, first: Value, second: Value; optional locale: Value, ignore_case: Value) {
    Ok(if collate(cx, first, second, locale, ignore_case)?.is_eq() { Value::TRUE } else { Value::NIL })
}}

/// sysdep.c str_collate over two strings or symbols' names, in LOCALE (a
/// string) or the environment's; a unibyte string's characters are its bytes.
fn collate(cx: &mut Leaf<'_>, first: Value, second: Value, locale: Value, ignore_case: Value) -> std::result::Result<Ordering, Fault> {
    let codes = |cx: &mut Leaf<'_>, value: Value| -> std::result::Result<Vec<u32>, Fault> {
        match cx.symbol_name(value) {
            Ok(name) => Ok(name.chars().map(u32::from).collect()),
            Err(_) => Ok(cx.string_view(value)?.chars().collect()),
        }
    };
    let (a, b) = (codes(cx, first)?, codes(cx, second)?);
    let locale = if locale == Value::NIL { None } else {
        let bytes = cx.string_view(locale)?.bytes;
        let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
        Some(std::ffi::CString::new(&bytes[..end]).expect("no NUL before END"))
    };
    crate::locale_info::collate(&a, &b, locale.as_deref(), ignore_case != Value::NIL).map_err(Fault::Error)
}
