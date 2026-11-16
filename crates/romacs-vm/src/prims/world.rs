//! World builds (DESIGN-M1 §3): the startup function's extent.
use super::*;

// Romacs-only. romacs-loadup-bootstrap.el wraps the startup steps of GNU's
// `command-line' in (romacs--startup-phase t) ... (romacs--startup-phase nil):
// a world build may read per-process host facts only inside them. A no-op,
// returning nil, outside a build.
defsubr! { name = "romacs--startup-phase", gnu = "startup.el:1479", effect = Global,
fn startup_phase(cx, on: Value) {
    cx.startup_phase(on != Value::NIL);
    Ok(Value::NIL)
}}

// Romacs-only. Whether this process builds a world that ends at GNU's
// dump point (`host::end_at_dump_point'): romacs-loadup-bootstrap.el then
// stops before the startup steps, as loadup.el does under `dump-mode'.
defsubr! { name = "romacs--dumping", gnu = "loadup.el:607", effect = Reads,
fn dumping(_cx) {
    Ok(if crate::host::dumping() { Value::TRUE } else { Value::NIL })
}}
