//! Primitive domain modules. build.rs writes the module list from the files
//! in this directory, so adding a file edits no shared line.
//!
//! Everything under `src/prims/` is safe Rust: a body cannot forge a `&mut Vm`
//! from a raw pointer, and build.rs rejects global and thread-local storage
//! here, so no Value can outlive its call in a place the collector cannot see.
#![forbid(unsafe_code)]
use super::*;
include!(concat!(env!("OUT_DIR"), "/prims_mods.rs"));

/// The VM's own Lisp, which `Vm::install_lisp_prelude` evaluates from the
/// runtime tree's lisp/, in this order:
pub(crate) const PRELUDES: [&str; 6] = [
    // `safe_run_hooks`, for the prelude and the keyboard half alike.
    "vm/hooks.el",
    // T3 tier: Lisp loops that call Lisp per element, over leaf cursors.
    "vm/prelude.el",
    // T3 tier: process.c's waits, filters, sentinels and timers over prims/process.rs.
    "vm/subprocess.el",
    // T3 tier: inotify.c's watch list and event dispatch over prims/inotify.rs.
    "vm/inotify.el",
    // T3 tier: isolates' spawning, futures, delivery and the worker's loop over prims/isolate.rs.
    "vm/isolate.el",
    // T3 tier: window.c's change functions and its old-state accessors
    // (`run-window-configuration-change-hook`, `window-old-buffer`, ...),
    // which redisplay runs (host_input.rs) and any session may call.
    "vm/window-change.el",
];

/// The form of the runtime tree's lisp/PATH, a Lisp half's `(progn ...)`,
/// which its primitive evaluates on first use: the file as the tree holds
/// it now, never a copy compiled into the executable.
pub(crate) fn lisp_half(cx: &mut Leaf<'_>, path: &str) -> std::result::Result<Value, Fault> {
    let file = crate::host::runtime_lisp(path).map_err(Fault::Error)?;
    let text = crate::host::read_to_string(&file).map_err(|e| Fault::Error(format!("{}: {e}", file.display())))?;
    let source = cx.string(&text);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    Ok(cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)").0)
}

// Compile-fail probes for the T1 boundary. `tests/primitive_gc_boundary.rs`
// checks this crate with `--cfg vm2_gc_probe="..."`, which places a fixture
// body exactly where a declared primitive lives, and requires rustc to refuse.
// The cfg is set only by that test, never by Cargo, hence the narrow allow.
#[allow(unexpected_cfgs)]
const _: () = {
    #[cfg(any(vm2_gc_probe = "leaf_field", vm2_gc_probe = "unsafe_body"))]
    #[path = "../../tests/fixtures/gc_boundary/probe.rs"]
    mod gc_boundary_probe;
};
