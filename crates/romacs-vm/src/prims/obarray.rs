//! GNU lread.c obarrays, with callback state held in traced T2 registers.
use super::*;

defsubr! { name = "obarrayp", gnu = "lread.c:5072", effect = Pure,
fn obarrayp(cx, object: Value) {
    Ok(if cx.is_obarray(object) { Value::TRUE } else { Value::NIL })
}}
defsubr! { name = "obarray-make", gnu = "lread.c:5052", effect = Pure,
fn obarray_make(cx; optional size: Value) { cx.make_obarray(size) }}
defsubr! { name = "intern-soft", gnu = "lread.c:4790", effect = Global,
fn intern_soft(cx, name: Value; optional obarray: Value) {
    let table = cx.resolve_obarray(obarray)?;
    cx.lookup_in(table, name)
}}
defsubr! { name = "unintern", gnu = "lread.c:4828", effect = Global,
fn unintern(cx, name: Value, obarray: Value) {
    let table = cx.resolve_obarray(obarray)?;
    cx.unintern_in(table, name)
}}
defsubr! { name = "mapatoms", gnu = "lread.c:5119", effect = Global,
fn mapatoms(cx, function: Value; optional obarray: Value) -> Step {
    let table = cx.resolve_obarray(obarray)?;
    walk(cx, vec![function, table, Value::integer(0).unwrap(), Value::UNBOUND])
}}
fn walk(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    let bucket = saved[2].integer_value().unwrap() as usize;
    let Some((bucket, symbol)) = cx.obarray_next(saved[1], bucket, saved[3]) else {
        return Ok(Step::Done(Value::NIL));
    };
    saved[2] = Value::integer(bucket as i64).unwrap();
    saved[3] = symbol;
    Ok(Step::Call { f: saved[0], args: vec![symbol], then: next, saved })
}
fn next(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { walk(cx, saved.to_vec()) }
