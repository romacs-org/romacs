use super::*;

// Native CLIPBOARD text only. Lisp owns kill-ring and interprogram policy.
defsubr! { name = "romacs--clipboard-text", gnu = "romacs:clipboard", effect = Global,
fn clipboard_text(cx) { cx.clipboard_text() }}

defsubr! { name = "romacs--clipboard-set", gnu = "romacs:clipboard", effect = Global,
fn clipboard_set(cx, text: Value) { cx.clipboard_set(text) }}
