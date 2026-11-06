//! List primitives from GNU `fns.c` / `data.c`, declared with `defsubr!`.
//! Adding a primitive here touches NO shared file: build.rs discovers it.
use super::*;

/// GNU `FOR_EACH_TAIL_SAFE` (lisp.h:5845-5905), reproduced exactly, because
/// Lisp observes its counts (`safe-length` on a cycle). Returns the tail at
/// exit: nil after a detected cycle, the non-cons terminator otherwise.
fn for_each_tail_safe(cx: &Leaf<'_>, mut tail: Value, mut body: impl FnMut(Value)) -> Value {
    let mut tortoise = tail;
    let mut l: u64 = 0;
    while let Some((_, cdr)) = cx.pair(tail) {
        body(tail);
        tail = cdr;
        if tail == tortoise {
            return Value::NIL;
        }
        l += 1;
        if l & 4095 == 0 && l.is_power_of_two() {
            tortoise = tail;
        }
    }
    tail
}

defsubr! { name = "nthcdr", gnu = "fns.c:1749", effect = Pure,
fn nthcdr(cx, n: Value, list: Value) {
    let Some(n) = n.integer_value() else {
        let n = cx.bigint_value(n).ok_or(Fault::Wrong("integerp", n))?;
        return super::fns_access::bignum_nthcdr(cx, n, list);
    };
    let mut tail = list;
    if n <= 127 {
        // GNU's SMALL_LIST_LEN_MAX path: no cycle check.
        for _ in 0..n.max(0) {
            match cx.pair(tail) {
                Some((_, cdr)) => tail = cdr,
                None if tail == Value::NIL => return Ok(Value::NIL),
                // CHECK_LIST_END (tail, list): GNU names the WHOLE list.
                None => return Err(Fault::Wrong("listp", list)),
            }
        }
        return Ok(tail);
    }
    // Long counts: walk once, and on a cycle reduce the count modulo its length.
    let mut remaining = n;
    let mut seen = 0usize;
    let mut tortoise = tail;
    let mut power = 1usize;
    let mut period = 0usize;
    while remaining > 0 {
        let Some((_, cdr)) = cx.pair(tail) else {
            return if tail == Value::NIL { Ok(Value::NIL) } else { Err(Fault::Wrong("listp", list)) };
        };
        tail = cdr;
        remaining -= 1;
        seen += 1;
        period += 1;
        if tail == tortoise && remaining > 0 {
            remaining %= period as i64;
        } else if period == power {
            tortoise = tail;
            power *= 2;
            period = 0;
        }
        if seen > cx.list_limit() * 4 {
            return Err(Fault::Circular(list));
        }
    }
    Ok(tail)
}}

defsubr! { name = "proper-list-p", gnu = "fns.c:244", effect = Pure,
fn proper_list_p(cx, object: Value) {
    let mut len = 0i64;
    let mut last = object;
    for_each_tail_safe(cx, object, |t| {
        len += 1;
        last = cx.pair(t).map_or(Value::NIL, |(_, cdr)| cdr);
    });
    Ok(if last == Value::NIL { Value::integer(len).unwrap() } else { Value::NIL })
}}

defsubr! { name = "safe-length", gnu = "fns.c:162", effect = Pure,
fn safe_length(cx, list: Value) {
    let mut len = 0i64;
    for_each_tail_safe(cx, list, |_| len += 1);
    Ok(Value::integer(len).unwrap())
}}

defsubr! { name = "cdr-safe", gnu = "data.c:688", effect = Pure,
fn cdr_safe(cx, object: Value) {
    Ok(cx.pair(object).map_or(Value::NIL, |(_, cdr)| cdr))
}}

defsubr! { name = "take", gnu = "fns.c:1671", effect = Pure,
fn take(cx, n: Value, list: Value) {
    // GNU's check order is conditional: N <= 0 returns before LIST is checked.
    let mut m = match n.integer_value() {
        Some(m) if m <= 0 => return Ok(Value::NIL),
        Some(m) => m,
        None => return Err(Fault::Wrong("integerp", n)),
    };
    if list != Value::NIL && cx.pair(list).is_none() {
        return Err(Fault::Wrong("listp", list));
    }
    let mut items = Vec::new();
    let mut tail = list;
    while m > 0 {
        let Some((car, cdr)) = cx.pair(tail) else { break };
        items.push(car);
        tail = cdr;
        m -= 1;
        if items.len() > cx.list_limit() {
            // GNU allocates until memory is exhausted here (VM2-TERM1 class).
            return Err(Fault::Circular(list));
        }
    }
    if m > 0 && tail != Value::NIL {
        return Err(Fault::Wrong("listp", tail));
    }
    Ok(items.iter().rev().fold(Value::NIL, |acc, &car| cx.cons(car, acc)))
}}
