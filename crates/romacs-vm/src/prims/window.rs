//! The window primitives GNU's own `window.el` reaches when it drives the
//! window journey (C-x 2, C-x 3, C-x o, C-x 1, C-x 0), measured by advising
//! every C DEFUN on the pinned GNU (`tools/primh/window-specs/MEASURED.md`).
//! Each is a `DEFUN` at the pin, `gnu-emacs-ref` 45b0f7699df: window.c, frame.c,
//! and minibuf.c `active-minibuffer-window`.
//!
//! Tier: T1 leaves over `Leaf::windows`, which owns every read and write of
//! the window tree, except `run-window-scroll-functions`, which is T2: it runs
//! `window-scroll-functions` (Lisp) with one `Step::Call`. Nothing here
//! collects; every Value a body returns is rooted in a window, a slot, or a
//! fresh allocation.
use super::*;

defsubr! { name = "window-valid-p", gnu = "window.c:323", effect = Reads,
fn window_valid_p(cx, object: Value) {
    Ok(cx.windows().window_valid_p(object))
}}

defsubr! { name = "window-frame", gnu = "window.c:342", effect = Reads,
fn window_frame(cx; optional window: Value) {
    cx.windows().window_frame(window)
}}

defsubr! { name = "window-minibuffer-p", gnu = "window.c:380", effect = Reads,
fn window_minibuffer_p(cx; optional window: Value) {
    cx.windows().window_minibuffer_p(window)
}}

defsubr! { name = "set-frame-selected-window", gnu = "window.c:456", effect = Global,
fn set_frame_selected_window(cx, frame: Value, window: Value; optional norecord: Value) {
    cx.windows().set_frame_selected_window(frame, window, norecord)
}}

defsubr! { name = "window-top-child", gnu = "window.c:689", effect = Reads,
fn window_top_child(cx; optional window: Value) {
    cx.windows().first_child(window, false)
}}

defsubr! { name = "window-left-child", gnu = "window.c:701", effect = Reads,
fn window_left_child(cx; optional window: Value) {
    cx.windows().first_child(window, true)
}}

defsubr! { name = "window-use-time", gnu = "window.c:767", effect = Reads,
fn window_use_time(cx; optional window: Value) {
    cx.windows().window_use_time(window)
}}

defsubr! { name = "window-new-total", gnu = "window.c:934", effect = Reads,
fn window_new_total(cx; optional window: Value) {
    cx.windows().window_new_total(window)
}}

defsubr! { name = "window-normal-size", gnu = "window.c:947", effect = Reads,
fn window_normal_size(cx; optional window: Value, horizontal: Value) {
    cx.windows().window_normal_size(window, horizontal)
}}

defsubr! { name = "window-new-normal", gnu = "window.c:976", effect = Reads,
fn window_new_normal(cx; optional window: Value) {
    cx.windows().window_new_normal(window)
}}

defsubr! { name = "window-mode-line-height", gnu = "window.c:1215", effect = Reads,
fn window_mode_line_height(cx; optional window: Value) {
    cx.windows().window_mode_line_height(window)
}}

defsubr! { name = "window-header-line-height", gnu = "window.c:1224", effect = Reads,
fn window_header_line_height(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-tab-line-height", gnu = "window.c:1233", effect = Reads,
fn window_tab_line_height(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-right-divider-width", gnu = "window.c:1242", effect = Reads,
fn window_right_divider_width(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-bottom-divider-width", gnu = "window.c:1251", effect = Reads,
fn window_bottom_divider_width(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-scroll-bar-width", gnu = "window.c:1260", effect = Reads,
fn window_scroll_bar_width(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-scroll-bar-height", gnu = "window.c:1269", effect = Reads,
fn window_scroll_bar_height(cx; optional window: Value) {
    cx.windows().live_zero(window)
}}

defsubr! { name = "window-dedicated-p", gnu = "window.c:2274", effect = Reads,
fn window_dedicated_p(cx; optional window: Value) {
    cx.windows().window_dedicated_p(window)
}}

defsubr! { name = "next-window", gnu = "window.c:2941", effect = Reads,
fn next_window(cx; optional window: Value, minibuf: Value, all_frames: Value) {
    cx.windows().next_window(window, minibuf, all_frames, true)
}}

defsubr! { name = "window-list-1", gnu = "window.c:3079", effect = Reads,
fn window_list_1(cx; optional window: Value, minibuf: Value, all_frames: Value) {
    cx.windows().window_list_1(window, minibuf, all_frames)
}}

defsubr! { name = "delete-other-windows-internal", gnu = "window.c:3467", effect = Global,
fn delete_other_windows_internal(cx; optional window: Value, root: Value) {
    cx.windows().delete_other_windows_internal(window, root)
}}

defsubr! { name = "run-window-scroll-functions", gnu = "window.c:3852", effect = Global,
fn run_window_scroll_functions(cx; optional window: Value) -> Step {
    // GNU passes the hook WINDOW as given, nil when omitted, not the
    // decoded window (window.c:3867).
    let (_, buffer, start) = cx.windows().scroll_functions_call(window)?;
    // GNU switches to WINDOW's buffer, then runs the hook only when
    // `window-scroll-functions` is non-nil there (window.c:3862), where it
    // may be buffer-local; with it nil the switch is unobservable.
    let hook = cx.intern("window-scroll-functions");
    if cx.buffer_local_read(hook, buffer)? == Value::NIL {
        return Ok(Step::Done(Value::NIL));
    }
    // (let ((old (current-buffer)))
    //   (unwind-protect (progn (set-buffer BUFFER)
    //                          (run-hook-with-args 'window-scroll-functions WINDOW START))
    //     (set-buffer old)))
    let old = cx.intern("romacs--window-scroll-old-buffer");
    let list = |cx: &mut Leaf<'_>, items: &[Value]| items.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail));
    let quote = cx.intern("quote");
    let quoted_hook = list(cx, &[quote, hook]);
    let (run, set_buffer, current) = (cx.intern("run-hook-with-args"), cx.intern("set-buffer"), cx.intern("current-buffer"));
    let (progn, unwind, let_) = (cx.intern("progn"), cx.intern("unwind-protect"), cx.intern("let"));
    let quoted = |cx: &mut Leaf<'_>, v: Value| { let q = cx.intern("quote"); list(cx, &[q, v]) };
    let (window_q, buffer_q, start_q) = (quoted(cx, window), quoted(cx, buffer), quoted(cx, start));
    let run_form = list(cx, &[run, quoted_hook, window_q, start_q]);
    let set_form = list(cx, &[set_buffer, buffer_q]);
    let body = list(cx, &[progn, set_form, run_form]);
    let restore = list(cx, &[set_buffer, old]);
    let protected = list(cx, &[unwind, body, restore]);
    let current_form = list(cx, &[current]);
    let binding = list(cx, &[old, current_form]);
    let bindings = list(cx, &[binding]);
    let form = list(cx, &[let_, bindings, protected]);
    let eval = cx.intern("eval");
    Ok(Step::Call { f: eval, args: vec![form, Value::TRUE], then: scrolled, saved: vec![] })
}}

/// Frun_window_scroll_functions returns nil whatever the hook returned.
fn scrolled(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}

defsubr! { name = "set-window-new-total", gnu = "window.c:4667", effect = Global,
fn set_window_new_total(cx, window: Value, size: Value; optional add: Value) {
    cx.windows().set_window_new_total(window, size, add)
}}

defsubr! { name = "set-window-new-normal", gnu = "window.c:4693", effect = Global,
fn set_window_new_normal(cx, window: Value; optional size: Value) {
    cx.windows().set_window_new_normal(window, size)
}}

defsubr! { name = "window-resize-apply-total", gnu = "window.c:5003", effect = Global,
fn window_resize_apply_total(cx; optional frame: Value, horizontal: Value) {
    let _ = horizontal;
    cx.windows().window_resize_apply_total(frame)
}}

defsubr! { name = "window-fringes", gnu = "window.c:8639", effect = Reads,
fn window_fringes(cx; optional window: Value) {
    cx.windows().window_fringes(window)
}}

defsubr! { name = "frame-live-p", gnu = "frame.c:326", effect = Reads,
fn frame_live_p(cx, object: Value) {
    Ok(cx.windows().frame_live_p(object))
}}

defsubr! { name = "frame-char-height", gnu = "frame.c:4345", effect = Reads,
fn frame_char_height(cx; optional frame: Value) {
    cx.windows().frame_char_size(frame, false)
}}

defsubr! { name = "frame-char-width", gnu = "frame.c:4363", effect = Reads,
fn frame_char_width(cx; optional frame: Value) {
    cx.windows().frame_char_size(frame, true)
}}

defsubr! { name = "frame-right-divider-width", gnu = "frame.c:4531", effect = Reads,
fn frame_right_divider_width(cx; optional frame: Value) {
    cx.windows().frame_divider_width(frame)
}}

defsubr! { name = "frame-bottom-divider-width", gnu = "frame.c:4538", effect = Reads,
fn frame_bottom_divider_width(cx; optional frame: Value) {
    cx.windows().frame_divider_width(frame)
}}

defsubr! { name = "active-minibuffer-window", gnu = "minibuf.c:215", effect = Reads,
fn active_minibuffer_window(cx) {
    cx.windows().active_minibuffer_window()
}}

defsubr! { name = "frame-first-window", gnu = "window.c:390", effect = Reads,
fn frame_first_window(cx; optional frame_or_window: Value) {
    cx.windows().frame_first_window(frame_or_window)
}}

defsubr! { name = "window-prev-buffers", gnu = "window.c:2322", effect = Reads,
fn window_prev_buffers(cx; optional window: Value) {
    cx.windows().window_buffers(window, false)
}}

defsubr! { name = "set-window-prev-buffers", gnu = "window.c:2339", effect = Global,
fn set_window_prev_buffers(cx, window: Value, prev_buffers: Value) {
    cx.windows().set_window_buffers(window, prev_buffers, false)
}}

defsubr! { name = "window-next-buffers", gnu = "window.c:2353", effect = Reads,
fn window_next_buffers(cx; optional window: Value) {
    cx.windows().window_buffers(window, true)
}}

defsubr! { name = "set-window-next-buffers", gnu = "window.c:2362", effect = Global,
fn set_window_next_buffers(cx, window: Value, next_buffers: Value) {
    cx.windows().set_window_buffers(window, next_buffers, true)
}}

// Fset_window_buffer: a window that gets another buffer first records the
// old one (record-window-buffer, window.el); set_window_buffer then runs
// window-scroll-functions in the new buffer.
defsubr! { name = "set-window-buffer", gnu = "window.c:4432", effect = Global,
fn set_window_buffer(cx, window: Value, buffer_or_name: Value; optional keep_margins: Value) -> Step {
    let (window, buffer, changed) = cx.windows().set_window_buffer_check(window, buffer_or_name)?;
    let record = cx.intern("record-window-buffer");
    // Bootstrap regimes that have not loaded window.el have nothing to record.
    if changed && cx.fboundp(record) {
        return Ok(Step::Call { f: record, args: vec![window], then: show_buffer, saved: vec![window, buffer, keep_margins] });
    }
    show_buffer(cx, Value::NIL, &[window, buffer, keep_margins])
}}
fn show_buffer(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    cx.windows().set_window_buffer(saved[0], saved[1], saved[2])?;
    let run = cx.intern("run-window-scroll-functions");
    Ok(Step::Call { f: run, args: vec![saved[0]], then: shown, saved: vec![] })
}
fn shown(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }
