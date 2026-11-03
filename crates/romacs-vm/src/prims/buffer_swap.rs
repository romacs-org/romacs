//! GNU buffer.c buffer-swap-text over Vm::buffer_swap_text.
use super::*;

defsubr! { name = "buffer-swap-text", gnu = "buffer.c:2573", effect = Global,
fn buffer_swap_text(cx, buffer: Value) { cx.swap_buffer_text(buffer) }}
