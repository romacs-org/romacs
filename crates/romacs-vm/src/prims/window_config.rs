//! GNU window.c window configurations (`window_config.rs`).
use super::*;

defsubr! { name = "current-window-configuration", gnu = "window.c:8395", effect = Reads,
fn current_window_configuration(cx; optional frame: Value) {
    let config = cx.windows().current_window_configuration(frame)?;
    // save_window_save: a saved point advances on insertion where its
    // buffer's `window-point-insertion-type' says so.
    let insertion = cx.intern("window-point-insertion-type");
    for (marker, buffer) in cx.windows().configuration_points(config)? {
        if cx.buffer_local_read(insertion, buffer).is_ok_and(|v| v != Value::NIL) {
            cx.windows().set_marker_advances(marker)?;
        }
    }
    Ok(config)
}}

// DONT-SET-FRAME keeps the frame selected at entry selected (window.c:8076).
// A window that will show another buffer first records its current one.
defsubr! { name = "set-window-configuration", gnu = "window.c:7661", effect = Global,
fn set_window_configuration(cx, configuration: Value; optional dont_set_frame: Value, dont_set_miniwindow: Value) -> Step {
    let windows = cx.windows().window_configuration_records(configuration)?;
    let list = windows.iter().rev().fold(Value::NIL, |tail, &w| cx.cons(w, tail));
    record_next(cx, Value::NIL, &[configuration, dont_set_miniwindow, list, dont_set_frame])
}}
fn record_next(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (configuration, dont_set_miniwindow, windows, dont_set_frame) = (saved[0], saved[1], saved[2], saved[3]);
    let record = cx.intern("record-window-buffer");
    if let Some((window, rest)) = cx.pair(windows) && cx.fboundp(record) {
        return Ok(Step::Call { f: record, args: vec![window], then: record_next, saved: vec![configuration, dont_set_miniwindow, rest, dont_set_frame] });
    }
    let Some(restored) = cx.windows().set_window_configuration(configuration, dont_set_frame, dont_set_miniwindow)? else {
        return Ok(Step::Done(Value::NIL));
    };
    let saved = vec![restored.reselect, restored.new_current, restored.old_point];
    // The restored selected window's buffer was recorded (record_buffer,
    // buffer.c:2218): buffer-list-update-hook runs there, in that buffer,
    // unless it inhibits hooks; the saved frame and buffer come back after.
    match restored.recorded {
        Some(buffer) if cx.buffer_state(buffer).is_some_and(|s| !s.inhibit_buffer_hooks) => {
            let hook = cx.intern("buffer-list-update-hook");
            Ok(Step::Call { f: cx.intern("run-hooks"), args: vec![hook], then: finish, saved })
        }
        _ => finish(cx, Value::NIL, &saved),
    }
}
fn finish(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(cx.windows().finish_window_configuration(saved[0], saved[1], saved[2])?))
}

defsubr! { name = "window-configuration-p", gnu = "window.c:7640", effect = Pure,
fn window_configuration_p(cx, object: Value) {
    Ok(cx.windows().window_configuration_p(object))
}}

defsubr! { name = "window-configuration-equal-p", gnu = "window.c:9135", effect = Reads,
fn window_configuration_equal_p(cx, x: Value, y: Value) {
    cx.windows().window_configuration_equal_p(x, y)
}}

defsubr! { name = "window-configuration-frame", gnu = "window.c:7647", effect = Reads,
fn window_configuration_frame(cx, config: Value) {
    cx.windows().window_configuration_frame(config)
}}
