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
fn backtrace_locals(cx, level: Value; optional base: Value) -> Step {
    cx.backtrace_locals(level, base).map(Step::Done)
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
defsubr! { name = "romacs--inspect-frames", gnu = "romacs:inspect", effect = Reads,
fn romacs_inspect_frames(cx) -> Step {
    Ok(Step::Done(cx.inspect_frames()))
}}
defsubr! { name = "romacs--inspect-listen", gnu = "romacs:inspect", effect = Global,
fn romacs_inspect_listen(cx, directory: Value; optional store: Value) {
    let directory = cx.string_text(directory)?;
    let store = if store == Value::NIL { None } else { Some(cx.string_text(store)?) };
    crate::host::inspect_listen(&directory, store.as_deref()).map_err(|error| Fault::Error(format!("inspect listener: {error}")))?;
    Ok(Value::NIL)
}}
defsubr! { name = "romacs--inspect-stop", gnu = "romacs:inspect", effect = Global,
fn romacs_inspect_stop(_cx) {
    crate::host::inspect_stop();
    Ok(Value::NIL)
}}
defsubr! { name = "backtrace-debug", gnu = "eval.c:4035", effect = Global,
fn backtrace_debug(cx, level: Value, flag: Value; optional base: Value) -> Step {
    cx.backtrace_debug(level, flag, base).map(Step::Done)
}}
// Every frame with its locals in one walk, for `backtrace-get-frames`
// (lisp/romacs/romacs-backtrace.el), where GNU calls `backtrace--locals' per frame.
defsubr! { name = "backtrace--frames-and-locals", gnu = "romacs:backtrace", effect = Reads,
fn backtrace_frames_and_locals(cx; optional base: Value) -> Step {
    cx.backtrace_frames_and_locals(base).map(Step::Done)
}}
// The two halves of `backtrace-eval' (lisp/romacs/vm/prelude.el): GNU's
// specpdl_unrewind to a frame's entry, and back.
defsubr! { name = "romacs--backtrace-unrewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_unrewind(cx, nframes: Value; optional base: Value) {
    cx.backtrace_unrewind(nframes, base)
}}
defsubr! { name = "romacs--backtrace-rewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_rewind(cx, span: Value) {
    cx.backtrace_rewind(span)
}}
