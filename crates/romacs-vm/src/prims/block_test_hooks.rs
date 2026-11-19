//! Test-only, compiled under the `test-hooks` feature alone (build.rs
//! `source_gate`) and never shipped. Not in GNU.
use super::*;

// (romacs--test-block SECONDS): sleep SECONDS in Rust, polling nothing and
// retrying a signal's EINTR. It exists only to give the termination-alarm
// test (tests/isolate_zygotes.rs) a wait no signal can end: every real wait
// in a worker now hears its interrupt and termination.
defsubr! { name = "romacs--test-block", gnu = "romacs:test", effect = Global,
fn romacs_test_block(cx, seconds: Integer) {
    let _ = &cx;
    std::thread::sleep(std::time::Duration::from_secs(u64::try_from(seconds.0).unwrap_or(0)));
    Ok(Value::NIL)
}}
