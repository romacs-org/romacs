//! Inspect genuine call activations; callbacks run through ordinary T2 calls.
use super::*;

defsubr! { name = "mapbacktrace", gnu = "eval.c:4050", effect = Global,
fn mapbacktrace(cx, function: Value; optional base: Value) -> Step {
    let token = cx.backtrace_first(base)?.unwrap_or(0);
    next_frame(cx, function, token)
}}
fn next_frame(cx: &mut Leaf<'_>, function: Value, token: usize) -> Next {
    let Some((args, next)) = cx.backtrace_entry(token)? else {
        return Ok(Step::Done(Value::NIL));
    };
    Ok(Step::Call {
        f: function,
        args,
        then: next_result,
        saved: vec![
            function,
            Value::integer(next as i64).expect("backtrace token"),
        ],
    })
}
fn next_result(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    next_frame(
        cx,
        saved[0],
        saved[1].integer_value().expect("backtrace token") as usize,
    )
}
defsubr! { name = "backtrace--locals", gnu = "eval.c:4263", effect = Reads,
fn backtrace_locals(cx, level: Value; optional base: Value) {
    cx.backtrace_locals(level, base)
}}
