//! The flight recorder's step marks (`crate::flight`). Not in GNU.
use crate::prim::{Fault, Leaf};
use crate::Value;

// (romacs--flight-step LABEL): the command LABEL has run. A recording
// session logs the VM's fingerprint after it; a replaying one checks it.
defsubr! { name = "romacs--flight-step", gnu = "romacs:flight", effect = Global,
fn flight_step(cx, label: Value) {
    cx.flight_step(label)?;
    Ok(Value::NIL)
}}

// (romacs--flight-fingerprint): the VM's state as a flight step logs it,
// one field per part: heap tables, symbols, the collector's counts, the
// buffers' texts and the current buffer.
defsubr! { name = "romacs--flight-fingerprint", gnu = "romacs:flight", effect = Reads,
fn flight_fingerprint(cx) {
    cx.flight_fingerprint()
}}
