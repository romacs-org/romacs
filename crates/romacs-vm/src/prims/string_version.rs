//! GNU fns.c string-version-lessp, with gnulib filevercmp.c's ordering.
use super::*;

// GNU compares the strings' internal bytes with gnulib filenvercmp.
defsubr! { name = "string-version-lessp", gnu = "fns.c:577", effect = Pure,
fn string_version_lessp(cx, first: Value, second: Value) {
    let bytes = |value| -> std::result::Result<Vec<u8>, Fault> {
        match cx.symbol_name(value) { Ok(name) => Ok(name.into_bytes()), Err(_) => Ok(cx.string_view(value)?.bytes) }
    };
    Ok(if file_version_order(&bytes(first)?, &bytes(second)?).is_lt() { Value::TRUE } else { Value::NIL })
}}

/// gnulib filevercmp.c file_prefixlen: S without its longest suffix of
/// `.ALPHA[ALNUM~]*` parts, never all of a nonempty S.
fn file_prefix_len(s: &[u8]) -> usize {
    let (mut i, mut prefix) = (0, 0);
    while i < s.len() {
        i += 1;
        prefix = i;
        while i + 1 < s.len() && s[i] == b'.' && (s[i + 1].is_ascii_alphabetic() || s[i + 1] == b'~') {
            i += 2;
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'~') { i += 1; }
        }
    }
    prefix
}

/// filevercmp.c order: digits 0, letters themselves, `~' before the end of
/// the string, other bytes after every letter.
fn version_byte_order(s: &[u8], at: usize) -> i32 {
    match s.get(at) {
        None => -1,
        Some(c) if c.is_ascii_digit() => 0,
        Some(&c) if c.is_ascii_alphabetic() => i32::from(c),
        Some(b'~') => -2,
        Some(&c) => i32::from(c) + 256,
    }
}

/// filevercmp.c verrevcmp: Debian's version comparison.
fn version_compare(a: &[u8], b: &[u8]) -> i32 {
    let digit = |s: &[u8], at: usize| s.get(at).is_some_and(u8::is_ascii_digit);
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        while (i < a.len() && !digit(a, i)) || (j < b.len() && !digit(b, j)) {
            let (x, y) = (version_byte_order(a, i), version_byte_order(b, j));
            if x != y { return x - y; }
            i += 1;
            j += 1;
        }
        while a.get(i) == Some(&b'0') { i += 1; }
        while b.get(j) == Some(&b'0') { j += 1; }
        let mut first_diff = 0;
        while digit(a, i) && digit(b, j) {
            if first_diff == 0 { first_diff = i32::from(a[i]) - i32::from(b[j]); }
            i += 1;
            j += 1;
        }
        if digit(a, i) { return 1; }
        if digit(b, j) { return -1; }
        if first_diff != 0 { return first_diff; }
    }
    0
}

/// gnulib filenvercmp: empty first, then ".", "..", other dot names, then
/// a version comparison of the suffix-less names, then of the whole names.
fn file_version_order(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        ([], []) => return Equal,
        ([], _) => return Less,
        (_, []) => return Greater,
        ([b'.', ..], [first, ..]) if *first != b'.' => return Less,
        ([first, ..], [b'.', ..]) if *first != b'.' => return Greater,
        _ => {}
    }
    if a[0] == b'.' {
        for special in [&b"."[..], &b".."[..]] {
            match (a == special, b == special) {
                (true, true) => return Equal,
                (true, false) => return Less,
                (false, true) => return Greater,
                _ => {}
            }
        }
    }
    let (p, q) = (file_prefix_len(a), file_prefix_len(b));
    let result = version_compare(&a[..p], &b[..q]);
    let result = if result != 0 || (p == a.len() && q == b.len()) { result } else { version_compare(a, b) };
    result.cmp(&0)
}
