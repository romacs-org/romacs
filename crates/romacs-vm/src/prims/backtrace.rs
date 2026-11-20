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
// The current call stack as bounded JSON text, a string: the snapshot an inspect
// request's answer carries (inspect.rs).
defsubr! { name = "romacs--inspect-frames", gnu = "romacs:inspect", effect = Reads,
fn romacs_inspect_frames(cx) -> Step {
    Ok(Step::Done(cx.inspect_frames()))
}}
// Listen for inspect requests on DIRECTORY/inspect.sock, DIRECTORY a directory name,
// on a socket only this user may use, after stopping this session's earlier listener.  The
// caller checks that DIRECTORY is private to this user (`romacs-mcp-start' does).  A socket
// there that refuses a connection is a dead session's and is replaced; one that answers, or
// a listener that cannot be set up, signals an error.  With STORE, a directory name, the
// newest snapshots, up to 8, are saved there as they come, when they can be: it is made with
// mode 0700 when missing, and a STORE that is not this user's private directory keeps none.
// Returns nil.
defsubr! { name = "romacs--inspect-listen", gnu = "romacs:inspect", effect = Global,
fn romacs_inspect_listen(cx, directory: Value; optional store: Value) {
    let directory = cx.string_text(directory)?;
    let store = if store == Value::NIL { None } else { Some(cx.string_text(store)?) };
    crate::host::inspect_listen(&directory, store.as_deref()).map_err(|error| Fault::Error(format!("inspect listener: {error}")))?;
    Ok(Value::NIL)
}}
// The main VM's stalls over a second since the last call, for the usage
// log: (DROPPED LINE...), each LINE a stall's JSON record, oldest first;
// with ENDING (the session ends) the stall still going too, last
// (romacs-vm-gui watchdog.rs).
defsubr! { name = "romacs--take-stalls", gnu = "romacs:inspect", effect = Global,
fn romacs_take_stalls(cx; optional ending: Value) {
    let (lines, dropped) = crate::host::take_stalls(ending != Value::NIL);
    let mut list = Value::NIL;
    for line in lines.iter().rev() {
        let text = cx.string(line);
        list = cx.cons(text, list);
    }
    let dropped = Value::integer(i64::try_from(dropped).unwrap_or(i64::MAX)).unwrap_or(Value::NIL);
    Ok(cx.cons(dropped, list))
}}
// Stop listening for inspect requests: the listener's threads end and its socket is
// removed, if it is still this session's.  Returns nil.
defsubr! { name = "romacs--inspect-stop", gnu = "romacs:inspect", effect = Global,
fn romacs_inspect_stop(_cx) {
    crate::host::inspect_stop();
    Ok(Value::NIL)
}}
defsubr! { name = "backtrace-debug", gnu = "eval.c:4035", effect = Global,
fn backtrace_debug(cx, level: Value, flag: Value; optional base: Value) -> Step {
    cx.backtrace_debug(level, flag, base).map(Step::Done)
}}
// Every frame from BASE's outward in one walk, each (EVALD FUNCTION ARGS FLAGS
// LOCALS), for `backtrace-get-frames' (lisp/emacs-lisp/romacs-backtrace.el), where GNU calls
// `backtrace--locals' per frame.  BASE is nil for the innermost frame, a function for its
// nearest activation, or (N . FUNCTION) for N frames out from that.  The first frame's
// LOCALS is nil.  nil when BASE is not on the stack.
defsubr! { name = "backtrace--frames-and-locals", gnu = "romacs:backtrace", effect = Reads,
fn backtrace_frames_and_locals(cx; optional base: Value) -> Step {
    cx.backtrace_frames_and_locals(base).map(Step::Done)
}}
// The two halves of `backtrace-eval' (lisp/vm/prelude.el): GNU's
// specpdl_unrewind to a frame's entry, and back.
defsubr! { name = "romacs--backtrace-unrewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_unrewind(cx, nframes: Value; optional base: Value) {
    cx.backtrace_unrewind(nframes, base)
}}
defsubr! { name = "romacs--backtrace-rewind", gnu = "eval.c:4236", effect = Global,
fn backtrace_rewind(cx, span: Value) {
    cx.backtrace_rewind(span)
}}
