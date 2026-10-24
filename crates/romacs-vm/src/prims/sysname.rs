//! GNU editfns.c's host-name value and callable accessor.
use super::*;

pub(crate) fn initial_value(vm: &mut Vm) -> Value {
    // Linux exposes the same kernel nodename as GNU's gethostname, without an
    // unsafe syscall at the primitive boundary. Do not consult HOSTNAME or DNS.
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_else(|_| "unknown".into());
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
