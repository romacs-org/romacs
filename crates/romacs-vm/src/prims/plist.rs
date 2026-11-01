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

defsubr! { name = "plist-get", gnu = "fns.c:2609", effect = Global,
fn plist_get(cx, plist: Value, prop: Value; optional predicate: Value) -> Step {
    get_scan(cx, &[prop, predicate, plist, plist, Value::integer(0).unwrap()])
}}

// GNU Fplist_get: FOR_EACH_TAIL_SAFE over PROP VALUE pairs, so a dotted
// or circular PLIST ends the walk with nil instead of signalling. A
// predicate's cursor lives in traced continuation registers.
fn get_scan(cx: &mut Leaf<'_>, state: &[Value]) -> Next {
    let [prop, predicate, mut tail, mut tortoise, count] =
        *<&[Value; 5]>::try_from(state).expect("plist-get saves five registers");
    let mut steps = count.integer_value().expect("saved walk count");
    while let Some((key, rest)) = cx.pair(tail) {
        let Some((value, _)) = cx.pair(rest) else { break };
        if predicate != Value::NIL {
            return Ok(Step::Call {
                f: predicate,
                args: vec![key, prop],
                then: get_tested,
                saved: vec![prop, predicate, tail, tortoise, Value::integer(steps).expect("walk count is a fixnum")],
            });
        }
        if key == prop { return Ok(Step::Done(value)); }
        match get_step(cx, rest, tortoise, steps) {
            Some(next) => (tail, tortoise, steps) = next,
            None => break,
        }
    }
    Ok(Step::Done(Value::NIL))
}

fn get_tested(cx: &mut Leaf<'_>, matched: Value, saved: &[Value]) -> Next {
    let mut state = <[Value; 5]>::try_from(saved).expect("plist-get saves five registers");
    // GNU reads XCDR (tail) again after the call; a predicate that broke
    // that pair leaves a malformed list, refused rather than read.
    let rest = cx.pair(state[2]).ok_or(Fault::Wrong("plistp", state[2]))?.1;
    let (value, _) = cx.pair(rest).ok_or(Fault::Wrong("plistp", rest))?;
    if matched != Value::NIL { return Ok(Step::Done(value)); }
    let Some((tail, tortoise, steps)) = get_step(cx, rest, state[3], state[4].integer_value().expect("saved walk count")) else {
        return Ok(Step::Done(Value::NIL));
    };
    state[2] = tail;
    state[3] = tortoise;
    state[4] = Value::integer(steps).expect("walk count is a fixnum");
    get_scan(cx, &state)
}

/// FOR_EACH_TAIL_SAFE's step past REST, or None where it found a cycle.
fn get_step(cx: &Leaf<'_>, rest: Value, tortoise: Value, steps: i64) -> Option<(Value, Value, i64)> {
    for_each_tail_step(cx, rest, tortoise, steps).ok()
}
