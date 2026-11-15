//! The loads under way, for a caller that reports where an error came
//! from (lisp/romacs/romacs-init-errors.el, the user's init).
use super::*;

// Each load under way, outermost first: (FILE LINE ORDINAL BYTE), the
// top-level form of FILE being evaluated now, its line, its ordinal among
// the file's forms and its byte.  Only the loads the loader reads as
// source; one from compiled code or a world names no form.
defsubr! { name = "romacs--loads-in-progress", gnu = "romacs:load", effect = Reads,
fn loads_in_progress(cx) { cx.loads_in_progress() }}
