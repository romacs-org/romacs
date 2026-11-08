//! Debugging the display from Lisp.
use super::*;

// Make the display lose its GPU device as a driver reset would, so its
// device-loss recovery rebuilds the GPU state while the session runs: t when
// the request reached a display with a device, nil without one.
defsubr! { name = "romacs--debug-lose-device", gnu = "romacs:display", effect = Global,
fn debug_lose_device(cx) { cx.debug_lose_device() }}
