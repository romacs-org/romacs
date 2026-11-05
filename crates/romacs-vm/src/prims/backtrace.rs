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
    // backtrace_frame_apply: nil when no such frame exists.
    let Some(args) = cx.backtrace_frame_at(nframes, base)? else {
        return Ok(Step::Done(Value::NIL));
    };
    Ok(Step::Call { f: function, args, then: frame_result, saved: vec![] })
}}
fn frame_result(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }
defsubr! { name = "backtrace-debug", gnu = "eval.c:4035", effect = Global,
fn backtrace_debug(cx, level: Value, flag: Value; optional base: Value) {
    cx.backtrace_debug(level, flag, base)
}}
// Every frame with its locals in one walk, for `backtrace-get-frames`
// (rlisp/romacs-backtrace.el), where GNU calls `backtrace--locals' per frame.
defsubr! { name = "backtrace--frames-and-locals", gnu = "romacs:backtrace", effect = Reads,
fn backtrace_frames_and_locals(cx; optional base: Value) {
    cx.backtrace_frames_and_locals(base)
}}
// The two halves of `backtrace-eval' (rlisp/vm/prelude.el): GNU's
// specpdl_unrewind to a frame's entry, and back.
defsubr! { name = "romacs--backtrace-unrewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_unrewind(cx, nframes: Value; optional base: Value) {
    cx.backtrace_unrewind(nframes, base)
}}
defsubr! { name = "romacs--backtrace-rewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_rewind(cx, span: Value) {
    cx.backtrace_rewind(span)
}}
