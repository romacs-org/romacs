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

// Romacs-only. rlisp/romacs-loadup-bootstrap.el's `romacs--init-eval-after-load':
// the user's init gave `eval-after-load' PRINTED for FILE (a feature, or a
// file named by a string), not loaded yet: noted for its layer's plan
// (`world::layer'). A no-op, returning nil, unless a recording is on and the
// init itself made the call.
defsubr! { name = "romacs--note-init-after-load", gnu = "romacs:world", effect = Global,
fn note_init_after_load(cx, file: Value, printed: Value) {
    cx.note_init_after_load(file, printed)?;
    Ok(Value::NIL)
}}

// Romacs-only. `romacs-world-restart': SYMBOL's re-seed definition, entry
// INDEX of `romacs--world-pending', waits for the variable's first access
// (`world::pending'). Nil when SYMBOL cannot wait (a world build, a
// buffer-local, forwarded or let-bound variable): restart evaluates it now.
defsubr! { name = "romacs--world-pend", gnu = "romacs:world", effect = Global,
fn world_pend(cx, symbol: Value, index: Integer) {
    let index = u32::try_from(index.0).map_err(|_| Fault::Error("romacs--world-pend: index out of range".into()))?;
    Ok(if cx.pend_variable(symbol, index)? { Value::TRUE } else { Value::NIL })
}}

// Romacs-only. The init layer builder's question (`world::layer`
// `deferred_by`): for each of LOADED, a list (FEATURES FILES) of what an
// entry loaded (files by their true names), the library of DEFERRED, lists
// (NAME FILE) of the libraries whose configuration is refused (FILE the one
// `eval-after-load` named, or nil), that it loaded, or nil.
defsubr! { name = "romacs--world-layer-deferred-by", gnu = "romacs:world", effect = Global,
fn world_layer_deferred_by(cx, deferred: Value, loaded: Value) {
    cx.world_layer_deferred_by(deferred, loaded)
}}

// Romacs-only. `romacs--init-eval-after-load': ON holds the init's
// recording while an after-load form it gave runs at once, nil releases the
// hold (`world::layer'). A no-op without a recording.
defsubr! { name = "romacs--init-recording-hold", gnu = "romacs:world", effect = Global,
fn init_recording_hold(cx, on: Value) {
    cx.hold_init_recording(on != Value::NIL);
    Ok(Value::NIL)
}}

// Romacs-only. `romacs--init-eval-after-load': the position the user's
// init's next `eval-after-load' form for FILE takes among that library's
// (`world::layer'), or nil unless a recording is on and the init itself
// makes the call. A layer keys the forms it made by it.
defsubr! { name = "romacs--init-after-load-ordinal", gnu = "romacs:world", effect = Global,
fn init_after_load_ordinal(cx, file: Value) {
    cx.init_after_load_ordinal(file)
}}
