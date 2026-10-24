//! GNU fns.c property-list mutation. Symbol `put` additionally owns its
//! symbol slot and image-copy policy; here the caller's cons identity is kept.
use super::*;
use super::fns_equal::for_each_tail_step;

defsubr! { name = "plist-put", gnu = "fns.c:2667", effect = Global,
fn plist_put(cx, plist: Value, prop: Value, val: Value; optional predicate: Value) -> Step {
    scan(cx, &[plist, prop, val, predicate, plist, Value::NIL, plist,
               Value::integer(0).unwrap()])
}}

// All cursor state lives in traced continuation registers across PREDICATE.
// GNU reads the cursor's cdr again AFTER calling the predicate, so a callback
// can change the cell that is written or traversed.
fn scan(cx: &mut Leaf<'_>, state: &[Value]) -> Next {
    let [plist, prop, val, predicate, mut tail, mut prev, mut tortoise, count] =
        *<&[Value; 8]>::try_from(state).expect("plist-put saves eight registers");
    let mut steps = count.integer_value().expect("saved walk count");
    while let Some((key, rest)) = cx.pair(tail) {
        if cx.pair(rest).is_none() { break; }
        if predicate != Value::NIL {
            return Ok(Step::Call {
                f: predicate,
                args: vec![key, prop],
                then: tested,
                saved: vec![plist, prop, val, predicate, tail, prev, tortoise,
                            Value::integer(steps).expect("walk count is a fixnum")],
            });
        }
        if key == prop {
            cx.setcar(rest, val)?;
            return Ok(Step::Done(plist));
        }
        prev = tail;
        (tail, tortoise, steps) = for_each_tail_step(cx, rest, tortoise, steps)?;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("plistp", plist)); }
    let pair = cx.cons(val, Value::NIL);
    let pair = cx.cons(prop, pair);
    if prev == Value::NIL { return Ok(Step::Done(pair)); }
    let rest = cx.pair(prev).ok_or(Fault::Wrong("listp", prev))?.1;
    cx.setcdr(rest, pair)?;
    Ok(Step::Done(plist))
}

fn tested(cx: &mut Leaf<'_>, matched: Value, saved: &[Value]) -> Next {
    let mut state = <[Value; 8]>::try_from(saved).expect("plist-put saves eight registers");
    let rest = cx.pair(state[4]).ok_or(Fault::Wrong("listp", state[4]))?.1;
    if matched != Value::NIL {
        cx.setcar(rest, state[2])?;
        return Ok(Step::Done(state[0]));
    }
    // A predicate can destroy the pair GNU subsequently accesses unchecked.
    // Refuse that malformed state rather than panic in the cons-only walker.
    if cx.pair(rest).is_none() { return Err(Fault::Wrong("plistp", state[0])); }
    let (tail, tortoise, steps) = for_each_tail_step(
        cx, rest, state[6], state[7].integer_value().expect("saved walk count"))?;
    state[5] = state[4];
    state[4] = tail;
    state[6] = tortoise;
    state[7] = Value::integer(steps).expect("walk count is a fixnum");
    scan(cx, &state)
}
