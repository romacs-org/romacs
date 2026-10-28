//! GNU editfns.c's host-name value and callable accessor.
use super::*;

pub(crate) fn initial_value(vm: &mut Vm) -> Value {
    let host = crate::host::system_name().unwrap_or_else(|_| "unknown".into());
    let host = host.strip_suffix('\n').unwrap_or(&host).replace([' ', '\t'], "-");
    vm.allocate_object(data::Object::String(Box::new(
        data::LispString::from_bytes(host.into_bytes(), false))))
}

defsubr! { name = "system-name", gnu = "editfns.c:1283", effect = Reads,
fn system_name(cx) {
    let symbol = cx.intern("system-name");
    // GNU returns overrides verbatim, including nil and non-string objects.
    // Host-cache refresh is not implemented yet; see VM2-SYSNAME1.
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}}

defsubr! { name = "emacs-pid", gnu = "editfns.c:1292", effect = Reads,
fn emacs_pid(_cx) {
    Ok(Value::integer(i64::from(crate::host::pid())).expect("pid is a fixnum"))
}}

// Fsleep_for: GNU waits in wait_reading_process_output, which also runs
// timers and reads subprocess output. With the process prelude loaded that
// wait is rlisp/vm/subprocess.el's; a VM without it sleeps only when no timer
// is pending, since nothing else would run them.
defsubr! { name = "sleep-for", gnu = "dispnew.c:6894", effect = Global,
fn sleep_for(cx, seconds: Value; optional milliseconds: Value) -> Step {
    // extract_float: any integer or float.
    let mut duration = if let Some(n) = seconds.integer_value() { n as f64 }
        else if let Some(n) = cx.bigint_value(seconds) { if n.sign() == num_bigint::Sign::Minus { f64::NEG_INFINITY } else { f64::INFINITY } }
        else { cx.float_value(seconds).map_err(|_| Fault::Wrong("numberp", seconds))? };
    if milliseconds != Value::NIL {
        duration += milliseconds.integer_value().ok_or(Fault::Wrong("fixnump", milliseconds))? as f64 / 1000.0;
    }
    if duration > 0.0 {
        let wait = cx.intern("romacs--wait-reading-process-output");
        if cx.fboundp(wait) {
            let end = cx.float(now() + duration.min(1e9));
            return slept(cx, Value::NIL, &[wait, end]);
        }
        for name in ["timer-list", "timer-idle-list"] {
            let symbol = cx.intern(name);
            if cx.variable_value(symbol)?.is_some_and(|timers| timers != Value::NIL) {
                return Err(Fault::Error("VM2 sleep-for with pending timers is unsupported (timers do not run while it waits)".into()));
            }
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(duration.min(1e9)));
    }
    Ok(Step::Done(Value::NIL))
}}

/// The wait returns as soon as it reads output, so wait again until the
/// end time saved after the wait function, as Fsleep_for loops.
fn slept(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (wait, end) = (saved[0], saved[1]);
    let remaining = cx.float_value(end)? - now();
    if remaining <= 0.0 {
        return Ok(Step::Done(Value::NIL));
    }
    let limit = cx.float(remaining);
    let zero = Value::integer(0).expect("fixnum");
    Ok(Step::Call { f: wait, args: vec![limit, Value::NIL, zero], then: slept, saved: saved.to_vec() })
}

/// current_timespec, as `float-time' reads it.
fn now() -> f64 {
    crate::host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}
