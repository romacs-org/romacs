//! Background compiles of compiled Lisp (`crate::compiled_lisp`), the
//! switch rlisp/romacs-precompile.el turns. Not in GNU, whose `.elc` files
//! an installation compiles ahead (DIVERGENCES.md VM2-LOADPRECOMPILE).
use super::*;

// (romacs--compiled-lisp-background ON DELAY): with ON, compile in the
// background whenever the session has been idle DELAY seconds; without,
// start no more compiles. Non-nil if this process compiles Lisp at all.
defsubr! { name = "romacs--compiled-lisp-background", gnu = "romacs:compiled-lisp", effect = Global,
fn compiled_lisp_background(cx, on: Value, delay: Value) {
    cx.compiled_lisp_background(on != Value::NIL, delay)
}}
