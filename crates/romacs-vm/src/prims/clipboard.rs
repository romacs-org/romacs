use super::*;

// Native CLIPBOARD text only. Lisp owns kill-ring and interprogram policy.
defsubr! { name = "romacs--clipboard-text", gnu = "romacs:clipboard", effect = Global,
fn clipboard_text(cx) { cx.clipboard_text() }}

// Put the string TEXT on the native clipboard of the display the host is attached to.
// Signals an error with no host attached, or when the host cannot set it; a write that
// timed out signals and may still complete.  Returns nil.
defsubr! { name = "romacs--clipboard-set", gnu = "romacs:clipboard", effect = Global,
fn clipboard_set(cx, text: Value) { cx.clipboard_set(text) }}
