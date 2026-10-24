//! GNU fns.c:1516, copying only the alist spine and its entry conses.
use super::*;

defsubr! { name = "copy-alist", gnu = "fns.c:1516", effect = Pure,
fn copy_alist(cx, alist: Value) {
    if alist == Value::NIL { return Ok(alist); }
    let Some((first, mut tail)) = cx.pair(alist) else {
        return Err(Fault::Wrong("listp", alist));
    };
    // Fcopy_sequence starts FOR_EACH_TAIL at CDR, not at the first cons.
    let mut elements = vec![first];
    let mut tortoise = tail;
    let mut steps = 0u64;
    while let Some((car, cdr)) = cx.pair(tail) {
        elements.push(car);
        tail = cdr;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
    let mut out = Value::NIL;
    for element in elements.into_iter().rev() {
        let entry = if let Some((car, cdr)) = cx.pair(element) {
            cx.cons(car, cdr)
        } else { element };
        out = cx.cons(entry, out);
    }
    Ok(out)
}}
