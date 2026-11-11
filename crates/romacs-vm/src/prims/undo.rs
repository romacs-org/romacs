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

// The command loop's Lisp half (lisp/romacs/vm/keyboard.el) notes point before
// each command, as command_loop_1 does.
defsubr! { name = "romacs--note-command-point", gnu = "keyboard.c:1536", effect = Global,
fn note_command_point(cx) {
    cx.note_command_point()
}}

// The next buffer `undo-outer-limit-function' is to be asked about, with the
// size to ask, (BUFFER . SIZE), as things stand now, for
// `romacs--offer-undo-lists' (lisp/romacs/vm/prelude.el). ASKED is the list
// of buffers the run has asked about already, none of which is offered
// again; with it nil, nil too when no collection has found one since the
// last run of them (VM2-UNDO-OUTER).
defsubr! { name = "romacs--take-undo-offer", gnu = "undo.c:361", effect = Global,
fn take_undo_offer(cx; optional asked: Value) {
    cx.take_undo_offer(asked)
}}

// The rest of truncate_undo_list and compact_buffer for BUFFER, on ANSWER, what
// `undo-outer-limit-function' said about it: nil makes the cut.
defsubr! { name = "romacs--answer-undo-offer", gnu = "undo.c:371", effect = Global,
fn answer_undo_offer(cx, buffer: Value, answer: Value) {
    cx.answer_undo_offer(buffer, answer)
}}
