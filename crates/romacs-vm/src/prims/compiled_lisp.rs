//! Background compiles of compiled Lisp (`crate::compiled_lisp`), the
//! switch lisp/session/romacs-precompile.el turns. Not in GNU, whose `.elc` files
//! an installation compiles ahead (ledger/divergences VM2-LOADPRECOMPILE).
use super::*;

// (romacs--compiled-lisp-background ON DELAY): with ON, compile in the
// background whenever the session has been idle DELAY seconds; without,
// start no more compiles. Non-nil if this process compiles Lisp at all.
defsubr! { name = "romacs--compiled-lisp-background", gnu = "romacs:compiled-lisp", effect = Global,
fn compiled_lisp_background(cx, on: Value, delay: Value) {
    cx.compiled_lisp_background(on != Value::NIL, delay)
}}

// Return what this process's loads that could run compiled Lisp took.
// The value is a plist.  `:taken' counts the loads that ran a stored
// artifact, `:compiled' those that ran one compiled for the load,
// `:source-on-miss' those that read source where a compile was due, and
// `:source-off' those that read source because this process compiles
// nothing: compiled Lisp is off, or the store is read-only.
//
// `:misses' is the latest loads that read source with compiled Lisp on,
// newest first, each (FILE . REASON).  REASON is `deferred' (a background
// compile will make it), `read-only', `failed' (the compile published
// nothing), `not-compiled' (its compile refused the file or signalled),
// `chain' (the file is being compiled up this compile's chain) or
// `unreadable'.
//
// `:store' is the store and `:identity' the compile identity that keys
// it, each nil while unknown.  Not in GNU.
defsubr! { name = "romacs-compiled-lisp-stats", gnu = "romacs:compiled-lisp", effect = Reads,
fn compiled_lisp_stats(cx) {
    cx.compiled_lisp_stats()
}}
