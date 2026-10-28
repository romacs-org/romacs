//! GNU fns.c string comparison, including representation and range semantics.
use super::*;
use crate::lstring::BYTE8_OFFSET;
use std::cmp::Ordering;

defsubr! { name = "compare-strings", gnu = "fns.c:361", effect = Reads,
fn compare_strings(cx, first: Value, from: Value, to: Value, second: Value,
                   other_from: Value, other_to: Value; optional fold: Value) {
    let a = cx.string_view(first)?;
    let b = cx.string_view(second)?;
    let (start, end) = bounds(first, from, to, a.chars)?;
    let (other_start, other_end) = bounds(second, other_from, other_to, b.chars)?;
    let a = codes(&a);
    let b = codes(&b);
    let (order, matched) = compare(cx, &a[start..end], &b[other_start..other_end], fold != Value::NIL)?;
    Ok(match order {
        Ordering::Equal => Value::TRUE,
        Ordering::Less => Value::integer(-((matched + 1) as i64)).unwrap(),
        Ordering::Greater => Value::integer((matched + 1) as i64).unwrap(),
    })
}}

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

pub(super) fn codes(string: &crate::lstring::OwnedStr) -> Vec<u32> {
    string
        .chars()
        .map(|c| {
            if !string.multibyte && c >= 128 {
                c + BYTE8_OFFSET
            } else {
                c
            }
        })
        .collect()
}

pub(super) fn compare(
    cx: &mut Leaf<'_>,
    a: &[u32],
    b: &[u32],
    fold: bool,
) -> std::result::Result<(Ordering, usize), Fault> {
    for (index, (&x, &y)) in a.iter().zip(b).enumerate() {
        if x == y {
            continue;
        }
        let upper =
            |cx: &mut Leaf<'_>, c| -> std::result::Result<u32, Fault> { cx.completion_upcase(c) };
        let (x, y) = if fold {
            (upper(cx, x)?, upper(cx, y)?)
        } else {
            (x, y)
        };
        if x != y {
            return Ok((x.cmp(&y), index));
        }
    }
    Ok((a.len().cmp(&b.len()), a.len().min(b.len())))
}

pub(super) fn prefix(
    cx: &mut Leaf<'_>,
    a: Value,
    b: Value,
    length: usize,
    fold: bool,
) -> std::result::Result<(Ordering, usize), Fault> {
    let a = codes(&cx.string_view(a)?);
    let b = codes(&cx.string_view(b)?);
    compare(
        cx,
        &a[..length.min(a.len())],
        &b[..length.min(b.len())],
        fold,
    )
}

/// Symbol names compare as their strings (minibuf.c Fassoc_string).
fn name_string(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Option<Vec<u32>>, Fault> {
    if cx.kind(value) == crate::prim::leaf::Kind::Symbol {
        let name = cx.symbol_name(value)?;
        let name = cx.string(&name);
        return Ok(Some(codes(&cx.string_view(name)?)));
    }
    if matches!(cx.kind(value), crate::prim::leaf::Kind::String | crate::prim::leaf::Kind::Bytes) {
        return Ok(Some(codes(&cx.string_view(value)?)));
    }
    Ok(None)
}

defsubr! { name = "assoc-string", gnu = "minibuf.c:2203", effect = Reads,
fn assoc_string(cx, key: Value, list: Value; optional fold: Value) {
    // GNU checks KEY only when an element's string is compared with it.
    let key_codes = name_string(cx, key)?;
    let mut tail = list;
    let mut seen = std::collections::HashSet::new();
    while let Some((element, rest)) = cx.pair(tail) {
        if !seen.insert(tail.0) { return Err(Fault::Circular(list)); }
        let candidate = cx.pair(element).map_or(element, |(car, _)| car);
        if let Some(candidate) = name_string(cx, candidate)? {
            let key = key_codes.as_deref().ok_or(Fault::Wrong("stringp", key))?;
            let (order, _) = compare(cx, &candidate, key, fold != Value::NIL)?;
            if order == Ordering::Equal { return Ok(element); }
        }
        tail = rest;
    }
    Ok(Value::NIL)
}}
