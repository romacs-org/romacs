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
/// Fframe_parameters for a frame without a window system: the stored alist
/// plus the entries GNU computes, each added as store_in_alist would.
fn frame_alist(cx: &mut Leaf<'_>, frame: Value) -> std::result::Result<Vec<(Value, Value)>, Fault> {
    let stored = cx.windows().frame_parameter_entries(frame)?;
    let name_key = cx.intern("name");
    let name = stored.iter().find(|(k, _)| *k == name_key).map_or(Value::NIL, |(_, v)| *v);
    let mut alist: Vec<(Value, Value)> = stored.into_iter().filter(|(k, _)| *k != name_key).collect();
    let mut store = |alist: &mut Vec<(Value, Value)>, key: Value, value: Value| match alist.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => alist.insert(0, (key, value)),
    };
    // tty_color_name of the initial terminal's unspecified default pixels.
    for (key, unspecified) in [("foreground-color", "unspecified-fg"), ("background-color", "unspecified-bg")] {
        let key = cx.intern(key);
        if !alist.iter().any(|(k, v)| *k == key && matches!(cx.kind(*v), leaf::Kind::String | leaf::Kind::Bytes)) {
            let value = cx.string(unspecified);
            store(&mut alist, key, value);
        }
    }
    let tty = cx.string("tty");
    store(&mut alist, cx.intern("font"), tty);
    store(&mut alist, name_key, name);
    let (cols, lines) = cx.windows().frame_text_size();
    store(&mut alist, cx.intern("height"), Value::integer(lines).unwrap());
    store(&mut alist, cx.intern("width"), Value::integer(cols).unwrap());
    store(&mut alist, cx.intern("modeline"), Value::TRUE);
    store(&mut alist, cx.intern("unsplittable"), Value::NIL);
    // VM2 keeps no per-frame buffer selection history (VM2-FRAME1).
    store(&mut alist, cx.intern("buffer-list"), Value::NIL);
    store(&mut alist, cx.intern("buried-buffer-list"), Value::NIL);
    // make_initial_frame: one menu-bar line while menu-bar-mode is on.
    let menu = cx.intern("menu-bar-mode");
    let lines = if cx.variable_value(menu)?.is_some_and(|v| v != Value::NIL) { 1 } else { 0 };
    store(&mut alist, cx.intern("menu-bar-lines"), Value::integer(lines).unwrap());
    store(&mut alist, cx.intern("tab-bar-lines"), Value::integer(0).unwrap());
    store(&mut alist, cx.intern("visibility"), Value::TRUE);
    store(&mut alist, cx.intern("no-accept-focus"), Value::NIL);
    Ok(alist)
}

defsubr! { name = "frame-parameters", gnu = "frame.c:4083", effect = Reads,
fn frame_parameters(cx; optional frame: Value) {
    // A fresh alist with fresh conses.
    let entries = frame_alist(cx, frame)?;
    Ok(entries.into_iter().rev().fold(Value::NIL, |tail, (k, v)| { let entry = cx.cons(k, v); cx.cons(entry, tail) }))
}}
defsubr! { name = "frame-parameter", gnu = "frame.c:4172", effect = Reads,
fn frame_parameter(cx, frame: Value, parameter: Value) {
    cx.windows().frame_parameter_entries(frame)?;
    if cx.kind(parameter) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", parameter)); }
    Ok(frame_alist(cx, frame)?.into_iter().find(|(k, _)| *k == parameter).map_or(Value::NIL, |(_, v)| v))
}}
defsubr! { name = "modify-frame-parameters", gnu = "frame.c:4243", effect = Global,
fn modify_frame_parameters(cx, frame: Value, alist: Value) {
    // A frame without a window system: list_length validates ALIST, Fcar and
    // Fcdr split each element, then entries are stored in reverse order.
    cx.windows().check_live_frame(frame)?;
    let mut elements = Vec::new();
    let mut seen = HashSet::new();
    let mut tail = alist;
    while let Some((element, rest)) = cx.pair(tail) {
        if !seen.insert(tail.0) { return Err(Fault::Circular(alist)); }
        elements.push(element);
        tail = rest;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", alist)); }
    let mut pairs = Vec::with_capacity(elements.len());
    for element in elements {
        pairs.push(match cx.pair(element) {
            Some(pair) => pair,
            None if element == Value::NIL => (Value::NIL, Value::NIL),
            None => return Err(Fault::Wrong("listp", element)),
        });
    }
    for (key, value) in pairs.into_iter().rev() {
        cx.windows().store_frame_parameter(frame, key, value)?;
    }
    Ok(Value::NIL)
}}
defsubr! { name = "window-system", gnu = "frame.c:415", effect = Reads,
fn window_system(cx; optional frame: Value) {
    // VM2 frames are initial-terminal frames (DIVERGENCES.md VM2-WIN5).
    cx.windows().check_live_frame(frame)?;
    Ok(Value::NIL)
}}
defsubr! { name = "current-message", gnu = "editfns.c:3288", effect = Reads,
fn current_message(cx) {
    // In batch, message writes to stderr and the echo area stays empty.
    let noninteractive = cx.intern("noninteractive");
    if cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) != Value::NIL { return Ok(Value::NIL); }
    let echo = cx.intern(crate::prim::ECHO_AREA_MESSAGE);
    Ok(cx.variable_value(echo)?.unwrap_or(Value::NIL))
}}
defsubr! { name = "internal-char-font", gnu = "font.c:4762", effect = Reads,
fn internal_char_font(cx, position: Value; optional ch: Value) {
    if position == Value::NIL {
        if !ch.integer_value().is_some_and(|c| (0..=0x3fffff).contains(&c)) { return Err(Fault::Wrong("characterp", ch)); }
    } else {
        let shape = cx.buffer_shape()?;
        let at = position.integer_value().or_else(|| cx.marker_position(position).flatten())
            .ok_or(Fault::Wrong("integer-or-marker-p", position))?;
        if !(shape.begv <= at && at < shape.zv) {
            return Err(Fault::Range3(position, Value::integer(shape.begv).unwrap(), Value::integer(shape.zv).unwrap()));
        }
        if ch != Value::NIL && !ch.integer_value().is_some_and(|c| c >= 0) { return Err(Fault::Wrong("wholenump", ch)); }
    }
    // A frame without a window system answers terminal_glyph_code; the
    // initial terminal has no glyph codes (VM2 frames, VM2-WIN5).
    Ok(Value::NIL)
}}
defsubr! { name = "internal-show-cursor", gnu = "dispnew.c:7428", effect = Global,
fn internal_show_cursor(cx, window: Value, show: Value) {
    cx.windows().set_cursor_off(window, show == Value::NIL)?;
    Ok(Value::NIL)
}}
defsubr! { name = "internal-show-cursor-p", gnu = "dispnew.c:7444", effect = Reads,
fn internal_show_cursor_p(cx; optional window: Value) {
    Ok(if cx.windows().cursor_off(window)? { Value::NIL } else { Value::TRUE })
}}
