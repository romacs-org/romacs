//! fns.c `maphash`, for callers that name it at run time (`apply`,
//! `funcall` of a variable). A call written (maphash F TABLE) is compiled
//! to the same walk in place (expand.rs).
use super::{Leaf, Next, Step, Value};

// Fmaphash over DOHASH_SAFE: FUNCTION on each used slot's key and value,
// the next slot read once the call returns.
defsubr! { name = "maphash", gnu = "fns.c:5960", effect = Global,
fn maphash(cx, function: Value, table: Value) -> Step {
    maphash_from(cx, function, table, 0)
}}

fn maphash_from(cx: &mut Leaf<'_>, function: Value, table: Value, from: usize) -> Next {
    let Some((slot, key, value)) = cx.hash_slot_from(table, from)? else { return Ok(Step::Done(Value::NIL)) };
    let next = Value::integer(i64::try_from(slot + 1).expect("a slot index is a fixnum")).expect("a slot index is a fixnum");
    Ok(Step::Call { f: function, args: vec![key, value], then: maphash_called, saved: vec![function, table, next] })
}

fn maphash_called(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let &[function, table, next] = saved else { unreachable!("maphash saves three registers") };
    maphash_from(cx, function, table, next.integer_value().and_then(|n| usize::try_from(n).ok()).expect("a saved slot index"))
}
