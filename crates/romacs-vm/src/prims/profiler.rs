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

fn truth(b: bool) -> Value {
    if b { Value::TRUE } else { Value::NIL }
}
