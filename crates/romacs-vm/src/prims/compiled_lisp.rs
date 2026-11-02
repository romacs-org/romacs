//! Background compiles of compiled Lisp (`crate::compiled_lisp`), the
//! substrate of rlisp/romacs-precompile.el. Not in GNU, whose `.elc` files
//! an installation compiles ahead (DIVERGENCES.md VM2-LOADPRECOMPILE).
use super::*;

// (romacs--compiled-lisp-candidates): the source files a background
// compile looks at, most likely first; nil in a process that compiles
// nothing.
defsubr! { name = "romacs--compiled-lisp-candidates", gnu = "romacs:compiled-lisp", effect = Reads,
fn compiled_lisp_candidates(cx) {
    cx.compiled_lisp_candidates()
}}

// (romacs--compiled-lisp-compile-command FILE): (PROGRAM ARGS ENVIRONMENT)
// that compiles FILE at the idle priority, ENVIRONMENT a list of
// "NAME=VALUE"; nil when a load of FILE would compile nothing now.
defsubr! { name = "romacs--compiled-lisp-compile-command", gnu = "romacs:compiled-lisp", effect = Reads,
fn compiled_lisp_compile_command(cx, file: Value) {
    cx.compiled_lisp_compile_command(file)
}}
