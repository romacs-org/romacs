//! Isolates' leaf primitives (vm-concurrency design §4): the substrate
//! rlisp/vm/isolate.el's `isolate-*' API runs on. Not in GNU
//! (DIVERGENCES.md VM2-ISOLATE). Peer 0 is a worker's parent.
use super::*;

fn id(cx: &Leaf<'_>, isolate: Value) -> std::result::Result<u64, Fault> {
    let _ = cx;
    isolate.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or(Fault::Wrong("natnump", isolate))
}

// (romacs--isolate-start PROGRAM ARGS): a worker process running PROGRAM
// (a file name) with the strings ARGS after `--isolate FD'; its id.
defsubr! { name = "romacs--isolate-start", gnu = "romacs:isolate", effect = Global,
fn isolate_start(cx, program: Value, args: Value) {
    cx.isolate_start(program, args)
}}

// (romacs--isolate-post ID KIND GENERATION VALUE): VALUE, frozen, as a
// frame of KIND (`job', `result', `error', `message', `hello', `bye').
defsubr! { name = "romacs--isolate-post", gnu = "romacs:isolate", effect = Global,
fn isolate_post(cx, isolate: Value, kind: Value, generation: Value, value: Value) {
    cx.isolate_post(id(cx, isolate)?, kind, generation, value)?;
    Ok(Value::NIL)
}}

// (romacs--isolate-wait TIMEOUT): wait up to TIMEOUT seconds (nil: as
// long as it takes, 0: look) for a frame or a worker's death; t if any.
defsubr! { name = "romacs--isolate-wait", gnu = "romacs:isolate", effect = Global,
fn isolate_wait(cx, timeout: Value) {
    let timeout = if timeout == Value::NIL { None } else {
        Some(cx.float_value(timeout).or_else(|_| timeout.integer_value().map(|n| n as f64).ok_or(Fault::Wrong("numberp", timeout)))?)
    };
    cx.isolate_wait(timeout.map(|s| std::time::Duration::from_secs_f64(s.max(0.0))))
}}

// (romacs--isolate-take ID): the next frame from peer ID as (KIND
// GENERATION VALUE), VALUE frozen; nil when none waits.
defsubr! { name = "romacs--isolate-take", gnu = "romacs:isolate", effect = Global,
fn isolate_take(cx, isolate: Value) {
    cx.isolate_take(id(cx, isolate)?)
}}

// (romacs--isolate-signal ID SIGNAL): SIGNAL (a number) to worker ID.
defsubr! { name = "romacs--isolate-signal", gnu = "romacs:isolate", effect = Global,
fn isolate_signal(cx, isolate: Value, signal: Integer) {
    cx.isolate_signal(id(cx, isolate)?, signal.0)
}}

// (romacs--isolate-reap ID): worker ID's wait status once it exited, else nil.
defsubr! { name = "romacs--isolate-reap", gnu = "romacs:isolate", effect = Global,
fn isolate_reap(cx, isolate: Value) {
    cx.isolate_reap(id(cx, isolate)?)
}}

// (romacs--isolate-signal-children ID): SIGHUP to the groups of the
// children worker ID reported and did not retire; the outcomes.
defsubr! { name = "romacs--isolate-signal-children", gnu = "romacs:isolate", effect = Global,
fn isolate_signal_children(cx, isolate: Value) {
    cx.isolate_signal_children(id(cx, isolate)?)
}}

// (romacs--isolate-children ID): the pids worker ID reported and has not retired.
defsubr! { name = "romacs--isolate-children", gnu = "romacs:isolate", effect = Reads,
fn isolate_children(cx, isolate: Value) {
    cx.isolate_children(id(cx, isolate)?)
}}

// (romacs--isolate-close ID): forget worker ID; t if it was known.
defsubr! { name = "romacs--isolate-close", gnu = "romacs:isolate", effect = Global,
fn isolate_close(cx, isolate: Value) {
    Ok(if cx.isolate_close(id(cx, isolate)?) { Value::TRUE } else { Value::NIL })
}}

// (romacs--isolate-pid ID): worker ID's process id, or nil.
defsubr! { name = "romacs--isolate-pid", gnu = "romacs:isolate", effect = Reads,
fn isolate_pid(cx, isolate: Value) {
    cx.isolate_pid(id(cx, isolate)?)
}}

// (romacs--isolate-frames ID): frames read from peer ID and not yet taken.
defsubr! { name = "romacs--isolate-frames", gnu = "romacs:isolate", effect = Reads,
fn isolate_frames(cx, isolate: Value) {
    Value::integer(cx.isolate_frames(id(cx, isolate)?) as i64).map_err(Fault::Error)
}}

// (romacs--isolate-ids): the ids of the workers this VM spawned.
defsubr! { name = "romacs--isolate-ids", gnu = "romacs:isolate", effect = Reads,
fn isolate_ids(cx) {
    cx.isolate_ids()
}}

// (romacs--isolate-parent-p): t in a worker, which has a parent peer 0.
defsubr! { name = "romacs--isolate-parent-p", gnu = "romacs:isolate", effect = Reads,
fn isolate_parent_p(cx) {
    Ok(if cx.isolate_has_parent() { Value::TRUE } else { Value::NIL })
}}

// (romacs--isolate-running GENERATION): in a worker, the job of
// GENERATION runs now; nil once it is done.
defsubr! { name = "romacs--isolate-running", gnu = "romacs:isolate", effect = Global,
fn isolate_running(cx, generation: Value) {
    cx.isolate_set_running(generation.integer_value().and_then(|n| u64::try_from(n).ok()));
    Ok(Value::NIL)
}}

// (romacs--isolate-interrupted-p): take the pending interrupt, if any.
defsubr! { name = "romacs--isolate-interrupted-p", gnu = "romacs:isolate", effect = Global,
fn isolate_interrupted_p(_cx) {
    Ok(if crate::host::take_interrupt() { Value::TRUE } else { Value::NIL })
}}

// (romacs--isolate-terminated-p): a termination signal was noted, or the
// spawner's socket has closed.
defsubr! { name = "romacs--isolate-terminated-p", gnu = "romacs:isolate", effect = Reads,
fn isolate_terminated_p(cx) {
    Ok(if crate::host::terminate_pending() || cx.isolate_parent_closed() { Value::TRUE } else { Value::NIL })
}}

// (romacs--isolate-hello-ok-p TEXT PROGRAM): whether a worker's hello
// TEXT names this protocol and the executable PROGRAM it was started as.
defsubr! { name = "romacs--isolate-hello-ok-p", gnu = "romacs:isolate", effect = Reads,
fn isolate_hello_ok_p(cx, text: Value, program: Value) {
    Ok(if cx.isolate_hello_ok(text, program)? { Value::TRUE } else { Value::NIL })
}}

// (romacs--isolate-hello BOOT): this process's hello text, for a worker.
defsubr! { name = "romacs--isolate-hello", gnu = "romacs:isolate", effect = Reads,
fn isolate_hello(cx, boot: Value) {
    cx.isolate_hello(boot)
}}

// (romacs--isolate-executable): this process's executable, a file name.
defsubr! { name = "romacs--isolate-executable", gnu = "romacs:isolate", effect = Reads,
fn isolate_executable(cx) {
    cx.isolate_executable()
}}

// (romacs--isolate-fork-request ID): ask zygote ID for a child, a new
// worker of this VM's; its id. Its pid comes with the zygote's report.
defsubr! { name = "romacs--isolate-fork-request", gnu = "romacs:isolate", effect = Global,
fn isolate_fork_request(cx, isolate: Value) {
    cx.isolate_fork_request(id(cx, isolate)?)
}}

// (romacs--isolate-fork GENERATION): in a zygote, fork the child the
// `fork' frame of GENERATION asked for: its pid in the zygote, nil in the
// child, which then serves the spawner as a worker of its own.
defsubr! { name = "romacs--isolate-fork", gnu = "romacs:isolate", effect = Global,
fn isolate_fork(cx, generation: Value) {
    cx.isolate_fork(id(cx, generation)?)
}}

// (romacs--isolate-release GENERATION): in a zygote, report the child
// forked for GENERATION to the spawner, then let it run.
defsubr! { name = "romacs--isolate-release", gnu = "romacs:isolate", effect = Global,
fn isolate_release(cx, generation: Value) {
    cx.isolate_release(id(cx, generation)?)
}}

// (romacs--isolate-fork-refusal): why this worker cannot fork now, or nil.
defsubr! { name = "romacs--isolate-fork-refusal", gnu = "romacs:isolate", effect = Reads,
fn isolate_fork_refusal(cx) {
    cx.isolate_fork_refusal()
}}

// (romacs--isolate-compact-next): the next collection compacts the heap.
defsubr! { name = "romacs--isolate-compact-next", gnu = "romacs:isolate", effect = Global,
fn isolate_compact_next(cx) {
    cx.isolate_compact_next();
    Ok(Value::NIL)
}}

// (romacs--isolate-reserve N): room in the heap's tables for N more entries.
defsubr! { name = "romacs--isolate-reserve", gnu = "romacs:isolate", effect = Global,
fn isolate_reserve(cx, needed: Value) {
    cx.isolate_reserve(id(cx, needed)? as usize);
    Ok(Value::NIL)
}}

// (romacs--isolate-stamp FILES): a fixnum digest of the size, change time
// and inode of each file in FILES (nil for a missing one), equal while
// none of them changed.
defsubr! { name = "romacs--isolate-stamp", gnu = "romacs:isolate", effect = Reads,
fn isolate_stamp(cx, files: Value) {
    cx.isolate_stamp(files)
}}

// (romacs--isolate-name-process NAME): this process's name, as `ps' shows it.
defsubr! { name = "romacs--isolate-name-process", gnu = "romacs:isolate", effect = Global,
fn isolate_name_process(cx, name: Value) {
    cx.isolate_name_process(name)
}}

// (romacs--isolate-forked): in a zygote, its children not yet reaped.
defsubr! { name = "romacs--isolate-forked", gnu = "romacs:isolate", effect = Reads,
fn isolate_forked(cx) {
    Value::integer(cx.isolate_forked() as i64).map_err(Fault::Error)
}}
