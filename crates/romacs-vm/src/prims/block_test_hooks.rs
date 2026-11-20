//! Test-only, compiled under the `test-hooks` feature alone (build.rs
//! `source_gate`) and never shipped. Not in GNU.
use super::*;

// (romacs--test-block SECONDS READY): write the file READY, then sleep
// SECONDS in Rust, polling nothing and retrying a signal's EINTR. It exists
// only to give the termination-alarm test (tests/isolate_zygotes.rs) a wait
// no signal can end, entered once READY exists: every real wait in a worker
// hears its interrupt and termination.
defsubr! { name = "romacs--test-block", gnu = "romacs:test", effect = Global,
fn romacs_test_block(cx, seconds: Integer, ready: Value) {
    let ready = cx.string_text(ready)?;
    crate::host::test_block(std::path::Path::new(&ready), u64::try_from(seconds.0).unwrap_or(0))
        .map_err(|e| Fault::Error(format!("romacs--test-block: {ready}: {e}")))?;
    Ok(Value::NIL)
}}
