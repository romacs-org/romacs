//! Frame and INITIAL-terminal primitives; no TTY backend is implied.
use super::*;

defsubr! { name = "framep", gnu = "frame.c:286", effect = Reads,
fn framep(cx, object: Value) { Ok(cx.windows().frame_p(object)) }}
defsubr! { name = "frame-list", gnu = "frame.c:2161", effect = Reads,
fn frame_list(cx) { cx.windows().frame_list() }}
defsubr! { name = "frame-terminal", gnu = "terminal.c:424", effect = Reads,
fn frame_terminal(cx; optional frame: Value) { cx.windows().frame_terminal(frame) }}
defsubr! { name = "terminal-live-p", gnu = "terminal.c:443", effect = Reads,
fn terminal_live_p(cx, object: Value) { Ok(cx.windows().terminal_live_p(object)) }}
defsubr! { name = "last-nonminibuffer-frame", gnu = "frame.c:2467", effect = Reads,
fn last_nonminibuffer_frame(cx) { cx.windows().last_nonminibuffer_frame() }}
defsubr! { name = "terminal-list", gnu = "terminal.c:501", effect = Reads,
fn terminal_list(cx) { cx.windows().terminal_list() }}
defsubr! { name = "tty-top-frame", gnu = "term.c:2517", effect = Reads,
fn tty_top_frame(cx; optional terminal: Value) { cx.windows().tty_top_frame(terminal) }}
defsubr! { name = "window-list", gnu = "window.c:3056", effect = Reads,
fn window_list(cx; optional frame: Value, minibuf: Value, window: Value) {
    cx.windows().window_list(frame, minibuf, window)
}}
defsubr! { name = "get-buffer-window", gnu = "window.c:3309", effect = Reads,
fn get_buffer_window(cx; optional buffer: Value, all_frames: Value) {
    cx.windows().get_buffer_window(buffer, all_frames)
}}
defsubr! { name = "window-combination-limit", gnu = "window.c:731", effect = Reads,
fn window_combination_limit(cx, window: Value) { cx.windows().window_combination_limit(window) }}
