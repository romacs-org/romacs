//! List primitives from GNU `fns.c` / `data.c`, declared with `defsubr!`.
//! Adding a primitive here touches NO shared file: build.rs discovers it.
use super::*;

/// GNU `FOR_EACH_TAIL_SAFE` (lisp.h:5845-5905), reproduced exactly, because
/// Lisp observes its counts (`safe-length` on a cycle): the conses of LIST
/// counted, each read once, and the tail that ends it, None after a cycle.
fn walk_tails(cx: &Leaf<'_>, list: Value) -> (i64, Option<Value>) {
    // `Leaf::pair` reads a cons of the VM's own table in place, so the walk
    // costs a load and a compare a cons where it once read each cons twice.
    let (mut tail, mut tortoise, mut count) = (list, list, 0i64);
    while let Some((_, cdr)) = cx.pair(tail) {
        tail = cdr;
        count += 1;
        if tail == tortoise {
            return (count, None);
        }
        if count & 4095 == 0 && count.count_ones() == 1 {
            tortoise = tail;
        }
    }
    (count, Some(tail))
}

defsubr! { name = "nthcdr", gnu = "fns.c:1749", effect = Pure,
fn nthcdr(cx, n: Value, list: Value) {
    let n = nth_count(cx, n, list)?;
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
    // A cycle, or a tail that is not nil, is no proper list; its length
    // is the conses walked.
    let (len, end) = walk_tails(cx, object);
    Ok(match end {
        Some(end) if end == Value::NIL => Value::integer(len).unwrap(),
        _ => Value::NIL,
    })
}}

defsubr! { name = "safe-length", gnu = "fns.c:162", effect = Pure,
fn safe_length(cx, list: Value) {
    // A circular list counts the conses walked before the cycle showed.
    let (len, _end) = walk_tails(cx, list);
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

/// N as a step count: a fixnum as it is; a negative bignum none at all; a
/// positive bignum, which no list reaches, its equivalent on LIST's cycle, or
/// the largest count, which walks to the end of a list that has none.
fn nth_count(cx: &Leaf<'_>, n: Value, list: Value) -> std::result::Result<i64, Fault> {
    if let Some(n) = n.integer_value() {
        return Ok(n);
    }
    let big = cx.bigint_value(n).ok_or(Fault::Wrong("integerp", n))?;
    Ok(big_nth_count(&big, list, |tail| cx.pair(tail).map(|(_, cdr)| cdr)) as i64)
}

/// The steps from LIST, along NEXT, that land where BIG steps would.
pub(crate) fn big_nth_count(big: &num_bigint::BigInt, list: Value, next: impl Fn(Value) -> Option<Value>) -> u64 {
    use num_bigint::{BigInt, Sign};
    if big.sign() == Sign::Minus {
        return 0;
    }
    let (mut tail, mut anchor, mut steps, mut period, mut power) = (list, list, 0u64, 0u64, 1u64);
    while let Some(cdr) = next(tail) {
        tail = cdr;
        steps += 1;
        period += 1;
        if tail == anchor {
            let reduced = (big - BigInt::from(steps)) % BigInt::from(period);
            return steps + u64::try_from(reduced).expect("a positive bignum exceeds any walked length");
        }
        if period == power {
            (anchor, period, power) = (tail, 0, power * 2);
        }
    }
    i64::MAX as u64
}
