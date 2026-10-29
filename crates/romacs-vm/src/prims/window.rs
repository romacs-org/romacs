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

defsubr! { name = "force-window-update", gnu = "window.c:4492", effect = Global,
fn force_window_update(cx; optional object: Value) {
    cx.windows().force_window_update(object)
}}

defsubr! { name = "redraw-display", gnu = "dispnew.c:3252", effect = Global,
fn redraw_display(cx) {
    cx.windows().redraw_display()
}}

defsubr! { name = "window-mode-line-height", gnu = "window.c:1215", effect = Reads,
fn window_mode_line_height(cx; optional window: Value) {
    cx.windows().window_mode_line_height(window)
}}

defsubr! { name = "window-header-line-height", gnu = "window.c:1224", effect = Reads,
fn window_header_line_height(cx; optional window: Value) {
    cx.windows().window_header_line_height(window)
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
    cx.windows().frame_absent_width(frame)
}}

defsubr! { name = "frame-bottom-divider-width", gnu = "frame.c:4538", effect = Reads,
fn frame_bottom_divider_width(cx; optional frame: Value) {
    cx.windows().frame_absent_width(frame)
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

defsubr! { name = "window-discard-buffer-from-window", gnu = "window.c:3445", effect = Global,
fn window_discard_buffer_from_window(cx, buffer: Value, window: Value; optional all: Value) {
    cx.windows().discard_buffer(buffer, window, all != Value::NIL)
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
    // A window strongly dedicated to its buffer keeps it; a weakly
    // dedicated one stops being dedicated.
    if changed {
        if let Some(name) = cx.windows().strongly_dedicated(window)? {
            let template = cx.string("Window is dedicated to `%s'");
            let name = cx.string(&name);
            let message = cx.format_message(template, &[name])?;
            let data = cx.cons(message, Value::NIL);
            return Err(Fault::Signal(cx.intern("error"), data));
        }
        cx.windows().set_window_dedicated_p(window, Value::NIL)?;
    }
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

defsubr! { name = "set-window-dedicated-p", gnu = "window.c:2297", effect = Global,
fn set_window_dedicated_p(cx, window: Value, flag: Value) {
    cx.windows().set_window_dedicated_p(window, flag)
}}

defsubr! { name = "window-body-width", gnu = "window.c:1140", effect = Reads,
fn window_body_width(cx; optional window: Value, pixelwise: Value) {
    cx.windows().window_body_size(window, pixelwise, true)
}}

defsubr! { name = "window-body-height", gnu = "window.c:1165", effect = Reads,
fn window_body_height(cx; optional window: Value, pixelwise: Value) {
    cx.windows().window_body_size(window, pixelwise, false)
}}

// Frame visibility (frame.c). VM2's one frame is always visible, as a
// terminal frame is in GNU batch: visibility requests change nothing, and
// making the sole frame invisible needs FORCE.
defsubr! { name = "make-frame-visible", gnu = "frame.c:3505", effect = Global,
fn make_frame_visible(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)
}}

defsubr! { name = "make-frame-invisible", gnu = "frame.c:3549", effect = Global,
fn make_frame_invisible(cx; optional frame: Value, force: Value) {
    cx.windows().decode_live_frame(frame)?;
    if force == Value::NIL {
        return Err(Fault::Error("Attempt to make invisible the sole visible or iconified frame".into()));
    }
    Ok(Value::NIL)
}}

defsubr! { name = "iconify-frame", gnu = "frame.c:3591", effect = Global,
fn iconify_frame(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(Value::NIL)
}}

defsubr! { name = "frame-visible-p", gnu = "frame.c:3633", effect = Reads,
fn frame_visible_p(cx, frame: Value) {
    if cx.windows().frame_live_p(frame) == Value::NIL { return Err(Fault::Wrong("frame-live-p", frame)); }
    Ok(Value::TRUE)
}}

defsubr! { name = "visible-frame-list", gnu = "frame.c:3652", effect = Reads,
fn visible_frame_list(cx) {
    let frame = cx.windows().decode_live_frame(Value::NIL)?;
    Ok(cx.cons(frame, Value::NIL))
}}

defsubr! { name = "raise-frame", gnu = "frame.c:3667", effect = Global,
fn raise_frame(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(Value::NIL)
}}

defsubr! { name = "lower-frame", gnu = "frame.c:3686", effect = Global,
fn lower_frame(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(Value::NIL)
}}

// Fredisplay: nil while a keyboard macro runs, else t. VM2 cannot redraw
// synchronously (VM2-REDISPLAY1); the host redraws after the command.
defsubr! { name = "redisplay", gnu = "dispnew.c:7020", effect = Global,
fn redisplay(cx; optional _force: Value) {
    let macro_ = cx.intern("executing-kbd-macro");
    Ok(if cx.variable_value(macro_)?.is_some_and(|m| m != Value::NIL) { Value::NIL } else { Value::TRUE })
}}

defsubr! { name = "pos-visible-in-window-p", gnu = "window.c:1998", effect = Reads,
fn pos_visible_in_window_p(cx; optional pos: Value, window: Value, partially: Value) {
    cx.pos_visible_in_window_p(pos, window, partially)
}}

defsubr! { name = "window-pixel-left", gnu = "window.c:1001", effect = Reads,
fn window_pixel_left(cx; optional window: Value) {
    cx.windows().window_pixel_edge(window, false)
}}

defsubr! { name = "window-pixel-top", gnu = "window.c:1009", effect = Reads,
fn window_pixel_top(cx; optional window: Value) {
    cx.windows().window_pixel_edge(window, true)
}}

// GNU's motion over the initial terminal frame's display, or over the host's
// layout once a host lays the frame out (DIVERGENCES.md VM2-WTPS1).
defsubr! { name = "window-text-pixel-size", gnu = "xdisp.c:12078", effect = Reads,
fn window_text_pixel_size(cx; optional window: Value, from: Value, to: Value, x_limit: Value, y_limit: Value, mode_lines: Value, ignore_line_at_end: Value) {
    cx.window_text_pixel_size(window, crate::prim::TextSizeArgs { from, to, x_limit, y_limit, mode_lines, ignore_line_at_end })
}}

defsubr! { name = "buffer-text-pixel-size", gnu = "xdisp.c:12158", effect = Reads,
fn buffer_text_pixel_size(cx; optional buffer_or_name: Value, window: Value, x_limit: Value, y_limit: Value, from: Value, to: Value) {
    cx.buffer_text_pixel_size(buffer_or_name, window, x_limit, y_limit, from, to)
}}

defsubr! { name = "posn-at-point", gnu = "keyboard.c:13054", effect = Reads,
fn posn_at_point(cx; optional pos: Value, window: Value) {
    cx.posn_at_point(pos, window)
}}

defsubr! { name = "frame-native-width", gnu = "frame.c:4381", effect = Reads,
fn frame_native_width(cx; optional frame: Value) {
    cx.windows().frame_native_size(frame, true)
}}

defsubr! { name = "frame-native-height", gnu = "frame.c:4401", effect = Reads,
fn frame_native_height(cx; optional frame: Value) {
    cx.windows().frame_native_size(frame, false)
}}

// GNU's iterator over the window's display, or over the host's layout once
// a host lays the frame out (DIVERGENCES.md VM2-VMOTION1).
defsubr! { name = "vertical-motion", gnu = "indent.c:2207", effect = Global,
fn vertical_motion(cx, lines: Value; optional window: Value, cur_col: Value) {
    cx.vertical_motion(lines, window, cur_col)
}}

defsubr! { name = "line-pixel-height", gnu = "xdisp.c:1545", effect = Reads,
fn line_pixel_height(cx) {
    cx.line_pixel_height()
}}

defsubr! { name = "window-scroll-bars", gnu = "window.c:8898", effect = Reads,
fn window_scroll_bars(cx; optional window: Value) {
    cx.windows().window_scroll_bars(window)
}}

defsubr! { name = "window-left-column", gnu = "window.c:1017", effect = Reads,
fn window_left_column(cx; optional window: Value) {
    cx.windows().window_char_edge(window, false)
}}

defsubr! { name = "window-top-line", gnu = "window.c:1029", effect = Reads,
fn window_top_line(cx; optional window: Value) {
    cx.windows().window_char_edge(window, true)
}}

defsubr! { name = "frame-internal-border-width", gnu = "frame.c:4524", effect = Reads,
fn frame_internal_border_width(cx; optional frame: Value) {
    cx.windows().frame_absent_width(frame)
}}

// xdisp.c sets a buffer's long_line_optimizations_p when redisplay meets a
// line past `long-line-threshold'; Romacs's redisplay has no long-line
// optimizations, so none is ever in effect (DIVERGENCES.md VM2-LONGLINE1).
defsubr! { name = "long-line-optimizations-p", gnu = "xdisp.c:20409", effect = Reads,
fn long_line_optimizations_p(_cx) {
    Ok(Value::NIL)
}}

// window.c Fselect_window; record_buffer (buffer.c:2218) runs
// buffer-list-update-hook unless the buffer inhibits its hooks.
defsubr! { name = "select-window", gnu = "window.c:604", effect = Global,
fn select_window(cx, window: Value; optional norecord: Value) -> Step {
    let (window, buffer) = cx.windows().select_window(window, norecord != Value::NIL)?;
    if norecord != Value::NIL || cx.buffer_state(buffer).is_none_or(|s| s.inhibit_buffer_hooks) {
        return Ok(Step::Done(window));
    }
    let hook = cx.intern("buffer-list-update-hook");
    Ok(Step::Call { f: cx.intern("run-hooks"), args: vec![hook], then: selected, saved: vec![window] })
}}
fn selected(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { Ok(Step::Done(saved[0])) }

defsubr! { name = "window-end", gnu = "window.c:1782", effect = Reads,
fn window_end(cx; optional window: Value, update: Value) {
    cx.window_end(window, update)
}}

defsubr! { name = "previous-window", gnu = "window.c:2980", effect = Reads,
fn previous_window(cx; optional window: Value, minibuf: Value, all_frames: Value) {
    cx.windows().next_window(window, minibuf, all_frames, false)
}}
