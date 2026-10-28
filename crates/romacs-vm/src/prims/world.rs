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
