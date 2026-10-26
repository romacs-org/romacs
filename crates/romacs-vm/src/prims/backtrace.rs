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
defsubr! { name = "backtrace-frame--internal", gnu = "eval.c:4079", effect = Global,
fn backtrace_frame_internal(cx, function: Value, nframes: Value, base: Value) -> Step {
    let count = nframes.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("wholenump", nframes))?;
    let mut token = cx.backtrace_first(base)?;
    for _ in 0..count {
        let Some(at) = token else { break; };
        token = cx.backtrace_entry(at)?.map(|(_, next)| next);
    }
    // backtrace_frame_apply: nil when no such frame exists.
    let Some((args, _)) = token.map(|at| cx.backtrace_entry(at)).transpose()?.flatten() else {
        return Ok(Step::Done(Value::NIL));
    };
    Ok(Step::Call { f: function, args, then: frame_result, saved: vec![] })
}}
fn frame_result(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }
