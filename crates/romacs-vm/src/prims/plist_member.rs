use super::*;

defsubr! { name = "plist-member", gnu = "fns.c:2745", effect = Global,
fn plist_member(cx, plist: Value, property: Value; optional predicate: Value) -> Step {
    scan(cx, &[plist, property, predicate, plist, plist, Value::integer(0).unwrap()])
}}

fn scan(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let [plist, property, predicate, mut tail, mut tortoise, step] = *saved else {
        unreachable!("plist-member saves six registers")
    };
    let mut steps = step.integer_value().unwrap();
    while let Some((key, _)) = cx.pair(tail) {
        if predicate != Value::NIL {
            return Ok(Step::Call {
                f: predicate,
                args: vec![key, property],
                then: compared,
                saved: vec![plist, property, predicate, tail, tortoise, Value::integer(steps).unwrap()],
            });
        }
        if key == property { return Ok(Step::Done(tail)); }
        tail = cx.pair(tail).unwrap().1;
        if cx.pair(tail).is_none() { break; }
        (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, tail, tortoise, steps)?;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("plistp", plist)); }
    Ok(Step::Done(Value::NIL))
}

fn compared(cx: &mut Leaf<'_>, matched: Value, saved: &[Value]) -> Next {
    if matched != Value::NIL { return Ok(Step::Done(saved[3])); }
    let mut state = saved.to_vec();
    state[3] = cx.pair(state[3]).unwrap().1;
    if cx.pair(state[3]).is_some() {
        let (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, state[3], state[4], state[5].integer_value().unwrap())?;
        state[3] = tail;
        state[4] = tortoise;
        state[5] = Value::integer(steps).unwrap();
    }
    scan(cx, &state)
}
