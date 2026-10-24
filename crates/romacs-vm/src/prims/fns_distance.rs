//! GNU fns.c:272, Levenshtein distance in GNU characters or storage bytes.
use super::*;

defsubr! { name = "string-distance", gnu = "fns.c:272", effect = Pure,
fn string_distance(cx, string1: Value, string2: Value; optional bytecompare: Value) {
    let s1 = cx.string_view(string1)?;
    let s2 = cx.string_view(string2)?;
    let bytes = bytecompare != Value::NIL || (!s1.multibyte && !s2.multibyte);
    let a: Vec<u32> = if bytes { s1.bytes.iter().map(|&b| u32::from(b)).collect() }
        else { s1.chars().collect() };
    let b: Vec<u32> = if bytes { s2.bytes.iter().map(|&b| u32::from(b)).collect() }
        else { s2.chars().collect() };
    let mut column: Vec<usize> = (0..=a.len()).collect();
    for (x, &c2) in b.iter().enumerate() {
        column[0] = x + 1;
        let mut lastdiag = x;
        for (y, &c1) in a.iter().enumerate() {
            let olddiag = column[y + 1];
            column[y + 1] = (column[y + 1] + 1).min(column[y] + 1)
                .min(lastdiag + usize::from(c1 != c2));
            lastdiag = olddiag;
        }
    }
    Value::integer(column[a.len()] as i64).map_err(|e| Fault::Error(e.into()))
}}
