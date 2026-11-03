//! GNU editfns.c compare-buffer-substrings.
use super::*;

defsubr! { name = "compare-buffer-substrings", gnu = "editfns.c:1752", effect = Reads,
fn compare_buffer_substrings(cx, buffer1: Value, start1: Value, end1: Value, buffer2: Value, start2: Value, end2: Value) {
    let fold = cx.intern("case-fold-search");
    let fold = cx.variable_value(fold)?.is_some_and(|v| v != Value::NIL);
    let first = cx.buffer_substring_codes(buffer1, start1, end1)?;
    let second = cx.buffer_substring_codes(buffer2, start2, end2)?;
    // GNU translates both through the current buffer's case_canon_table
    // when folding; equal characters need no translation.
    let mut canon = None;
    for (n, (&a, &b)) in first.iter().zip(&second).enumerate() {
        if a == b { continue; }
        let (a, b) = if fold {
            if canon.is_none() { canon = Some(cx.case_canon_translation()?); }
            let table = canon.as_ref().expect("made above");
            (table.translate(a), table.translate(b))
        } else { (a, b) };
        if a != b { return Ok(Value::integer(if a < b { -1 - n as i64 } else { n as i64 + 1 }).unwrap()); }
    }
    let n = first.len().min(second.len()) as i64;
    Ok(Value::integer(match first.len().cmp(&second.len()) {
        std::cmp::Ordering::Greater => n + 1,
        std::cmp::Ordering::Less => -n - 1,
        std::cmp::Ordering::Equal => 0,
    }).unwrap())
}}
