//! Plain leaf sequence operations, following GNU src/fns.c.
use super::*;

defsubr! { name = "memql", gnu = "fns.c:1933", effect = Pure,
fn memql(cx, elt: Value, list: Value) {
    search(cx, elt, list, false)
}}

defsubr! { name = "rassq", gnu = "fns.c:2049", effect = Pure,
fn rassq(cx, key: Value, alist: Value) {
    search(cx, key, alist, true)
}}

// GNU FOR_EACH_TAIL (lisp.h:5839-5904) reports the detecting tail,
// not necessarily the original list. Preserve its delayed tortoise movement.
fn search(cx: &Leaf<'_>, key: Value, list: Value, alist: bool) -> Out {
    let mut tail = list;
    let mut tortoise = list;
    let mut steps = 0u64;
    while let Some((car, cdr)) = cx.pair(tail) {
        if alist {
            if let Some((_, value)) = cx.pair(car) {
                if cx.eq(key, value) { return Ok(car); }
            }
        } else if cx.eql(key, car) {
            return Ok(tail);
        }
        tail = cdr;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(Value::NIL)
}

defsubr! { name = "delq", gnu = "fns.c:2081", effect = MutatesArgs,
fn delq(cx, elt: Value, list: Value) {
    delete_eq(cx, elt, list)
}}

defsubr! { name = "ntake", gnu = "fns.c:1712", effect = MutatesArgs,
fn ntake(cx, n: Value, list: Value) {
    truncate(cx, n, list)
}}

// GNU Fnconc: each list but the last must be a cons, walked with
// FOR_EACH_TAIL (a cycle signals); its last cons takes the next argument,
// and a nil argument hands that cons on to the one after.
defsubr! { name = "nconc", gnu = "fns.c:3390", effect = MutatesArgs,
fn nconc(cx; rest lists) {
    nconc_values(cx, lists)
}}

// `nconc' of the elements of a list: Fmapcan's Fnconc over its results,
// which the prelude's `mapcan' collects (lisp/romacs/vm/prelude.el).
defsubr! { name = "romacs--nconc-list", gnu = "fns.c:3585", effect = MutatesArgs,
fn nconc_list(cx, lists: Value) {
    let lists = super::widen::list_elements(cx, lists)?;
    nconc_values(cx, &lists)
}}

fn nconc_values(cx: &mut Leaf<'_>, lists: &[Value]) -> Out {
    let mut lists = lists.to_vec();
    let mut value = Value::NIL;
    for k in 0..lists.len() {
        let list = lists[k];
        if list == Value::NIL {
            continue;
        }
        if value == Value::NIL {
            value = list;
        }
        if k + 1 == lists.len() {
            break;
        }
        if cx.pair(list).is_none() {
            return Err(Fault::Wrong("consp", list));
        }
        let (mut tail, mut last, mut tortoise, mut steps) = (list, list, list, 0);
        while cx.pair(tail).is_some() {
            last = tail;
            (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, tail, tortoise, steps)?;
        }
        let next = lists[k + 1];
        cx.setcdr(last, next)?;
        if next == Value::NIL {
            lists[k + 1] = last;
        }
    }
    Ok(value)
}

fn delete_eq(cx: &mut Leaf<'_>, elt: Value, mut list: Value) -> Out {
    let mut tail = list;
    let mut previous = Value::NIL;
    let mut tortoise = list;
    let mut steps = 0u64;
    while let Some((car, cdr)) = cx.pair(tail) {
        if cx.eq(elt, car) {
            if previous == Value::NIL { list = cdr; }
            else { cx.setcdr(previous, cdr)?; }
        } else { previous = tail; }
        tail = cx.pair(tail).unwrap().1;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(list)
}

fn truncate(cx: &mut Leaf<'_>, n: Value, list: Value) -> Out {
    let n = n.integer_value().ok_or(Fault::Wrong("integerp", n))?;
    if n <= 0 { return Ok(Value::NIL); }
    if list != Value::NIL && cx.pair(list).is_none() {
        return Err(Fault::Wrong("listp", list));
    }
    let mut remaining = n - 1;
    let mut tail = list;
    let mut seen = std::collections::HashMap::new();
    while remaining > 0 {
        let Some((_, cdr)) = cx.pair(tail) else { break };
        // GNU's finite count can be reduced modulo a cycle without changing
        // the cons eventually truncated, even at most-positive-fixnum.
        if let Some(previous) = seen.insert(tail.0, remaining) {
            remaining %= previous - remaining;
            if remaining == 0 { break; }
            seen.clear();
        }
        tail = cdr;
        remaining -= 1;
    }
    if cx.pair(tail).is_some() { cx.setcdr(tail, Value::NIL)?; }
    else if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(list)
}
