//! GNU fns.c:2062, reverse association using structural equality.
use super::*;

defsubr! { name = "rassoc", gnu = "fns.c:2062", effect = Pure,
fn rassoc(cx, key: Value, alist: Value) {
    let mut tail = alist;
    let mut tortoise = alist;
    let mut steps = 0u64;
    while let Some((entry, cdr)) = cx.pair(tail) {
        if let Some((_, value)) = cx.pair(entry) {
            let matched = cx.equal(value, key).map_err(|e| match e {
                crate::hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
                crate::hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
                crate::hash::HashFault::Error(m, _) => Fault::Error(m.into()),
                _ => Fault::Error("equal failed".into()),
            })?;
            if matched { return Ok(entry); }
        }
        tail = cdr;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", alist)); }
    Ok(Value::NIL)
}}
