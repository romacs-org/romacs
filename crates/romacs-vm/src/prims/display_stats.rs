//! Redisplay's counts (`crate::display_stats`). Not in GNU.
use crate::Value;

// (romacs--display-stats &optional RESET): ((NAME . COUNT)...), what
// redisplay did on this evaluator since the last reset: `cycles' it ran
// (a frame frozen, compared with the one on screen and, where it changed,
// laid out and submitted); the `instructions' and `conses' spent inside
// them (`instructions' nil without a counter, as `romacs-instructions');
// the windows a frozen frame visited, `windows-reused' from their last
// snapshot or `windows-rebuilt' in full; `band-expansions' (mode lines,
// header lines, the status bar) and the evaluator entries they made
// (`band-evals'); `layouts' and `measures' asked of the layout host
// besides each paint's own layout; `frames-submitted' to the renderer; and
// `check-mismatches', the reused windows `romacs-display-check' found to
// differ from a full build.  RESET non-nil starts every count over after
// reading it.
defsubr! { name = "romacs--display-stats", gnu = "romacs:display", effect = Global,
fn display_stats(cx; optional reset: Value) {
    cx.display_stats(reset != Value::NIL)
}}
