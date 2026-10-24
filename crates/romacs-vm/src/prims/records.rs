//! GNU alloc.c records are distinct vectorlikes; their first slot is the type.
use super::*;

defsubr! { name = "record", gnu = "alloc.c:3516", effect = Pure,
fn record(cx, tag: Value; rest slots) {
    let mut values = Vec::with_capacity(slots.len() + 1);
    values.push(tag);
    values.extend_from_slice(slots);
    cx.record(values)
}}

defsubr! { name = "make-record", gnu = "alloc.c:3499", effect = Pure,
fn make_record(cx, tag: Value, slots: Value, init: Value) {
    let n = slots.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or(Fault::Wrong("wholenump", slots))?;
    // PSEUDOVECTOR_SIZE_BITS is 12 in the pinned GNU lisp.h.
    let count = n.checked_add(1).ok_or(Fault::Wrong("wholenump", slots))?;
    if count > 4095 {
        return Err(Fault::Error(format!("Attempt to allocate a record of {count} slots; max is 4095")));
    }
    let mut values = vec![init; count];
    values[0] = tag;
    cx.record(values)
}}

defsubr! { name = "recordp", gnu = "data.c:405", effect = Pure,
fn recordp(cx, object: Value) {
    Ok(if cx.kind(object) == leaf::Kind::Record { Value::TRUE } else { Value::NIL })
}}
