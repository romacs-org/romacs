//! GNU undo.c's boundary and buffer.c's buffer-enable-undo
//! (`prim/leaf/undo.rs'); the undo commands are GNU's simple.el.
use super::*;

defsubr! { name = "undo-boundary", gnu = "undo.c:251", effect = Global,
fn undo_boundary(cx) {
    cx.undo_boundary()
}}

defsubr! { name = "buffer-enable-undo", gnu = "buffer.c:1843", effect = Global,
fn buffer_enable_undo(cx; optional buffer: Value) {
    cx.buffer_enable_undo(buffer)
}}
