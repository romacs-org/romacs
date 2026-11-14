//! The evaluator's calls out of the VM, counted (`crate::host_calls`). Not in GNU.
use crate::Value;

// (romacs--host-calls): ((NAME SIDE COUNT)...), the calls this evaluator
// has made on its hosts since it started: NAME a host trait's method
// (`layout.measure'), a host.rs result (`read-file') or a call of the
// paint sender (`sender.submit_scene'), SIDE `local' when the call stays
// with the evaluator, `cross' when it reaches the display side, another
// thread or a file, and `through' for a host method whose crossings are
// the paint sender's calls it makes.
defsubr! { name = "romacs--host-calls", gnu = "romacs:host-calls", effect = Reads,
fn host_calls(cx) {
    cx.host_calls()
}}
