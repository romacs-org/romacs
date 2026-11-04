//! profiler.c's primitives over the VM's profilers (`src/profiler.rs`).
use super::*;

defsubr! { name = "profiler-cpu-start", gnu = "profiler.c:459", effect = Global,
fn profiler_cpu_start(cx, sampling_interval: Value) {
    cx.profiler_cpu_start(sampling_interval.integer_value().unwrap_or(0))?;
    Ok(Value::TRUE)
}}
defsubr! { name = "profiler-cpu-stop", gnu = "profiler.c:489", effect = Global,
fn profiler_cpu_stop(cx) {
    Ok(truth(cx.profiler_cpu_stop()))
}}
defsubr! { name = "profiler-cpu-running-p", gnu = "profiler.c:524", effect = Reads,
fn profiler_cpu_running_p(cx) {
    Ok(truth(cx.profiler_cpu_running()))
}}
defsubr! { name = "profiler-cpu-log", gnu = "profiler.c:533", effect = Global,
fn profiler_cpu_log(cx) {
    cx.profiler_cpu_log()
}}
defsubr! { name = "profiler-memory-start", gnu = "profiler.c:602", effect = Global,
fn profiler_memory_start(cx) {
    cx.profiler_memory_start()?;
    Ok(Value::TRUE)
}}
defsubr! { name = "profiler-memory-stop", gnu = "profiler.c:622", effect = Global,
fn profiler_memory_stop(cx) {
    Ok(truth(cx.profiler_memory_stop()))
}}
defsubr! { name = "profiler-memory-running-p", gnu = "profiler.c:635", effect = Reads,
fn profiler_memory_running_p(cx) {
    Ok(truth(cx.profiler_memory_running()))
}}
defsubr! { name = "profiler-memory-log", gnu = "profiler.c:644", effect = Global,
fn profiler_memory_log(cx) {
    cx.profiler_memory_log()
}}
defsubr! { name = "function-equal", gnu = "profiler.c:680", effect = Reads,
fn function_equal(cx, f1: Value, f2: Value) {
    Ok(truth(cx.function_equal(f1, f2)))
}}

defsubr! { name = "romacs-instructions", gnu = "romacs:profiler", effect = Reads,
fn romacs_instructions(_cx) {
    // The evaluating thread's user-space instructions, nil without a counter.
    match crate::host::thread_instructions() {
        Some(count) => Value::integer(count as i64).map_err(Fault::Error),
        None => Ok(Value::NIL),
    }
}}

defsubr! { name = "romacs-native-stack-room", gnu = "romacs:profiler", effect = Reads,
fn romacs_native_stack_room(_cx) {
    // Bytes left on the evaluating thread's native stack segment, nil when
    // the bound is unknown: a loop that keeps native frames shows it shrinking.
    match stacker::remaining_stack() {
        Some(bytes) => Value::integer(bytes as i64).map_err(Fault::Error),
        None => Ok(Value::NIL),
    }
}}

defsubr! { name = "romacs--redisplay-since-mark", gnu = "romacs:profiler", effect = Global,
fn romacs_redisplay_since_mark(cx; optional mark: Value) {
    // Microseconds the first redisplay after the last mark took on the
    // evaluator: the frame's snapshot, its layout and its paint's submission
    // (a prompt's paint waits for its render too); nil before one ran, or
    // with no mark. MARK non-nil sets a new mark. No allocation: a fixnum.
    let took = cx.redisplay_since_mark().flatten();
    if mark != Value::NIL { cx.set_redisplay_since_mark(Some(None)); }
    match took {
        Some(micros) => Value::integer(i64::try_from(micros).unwrap_or(i64::MAX)).map_err(Fault::Error),
        None => Ok(Value::NIL),
    }
}}

fn truth(b: bool) -> Value {
    if b { Value::TRUE } else { Value::NIL }
}
