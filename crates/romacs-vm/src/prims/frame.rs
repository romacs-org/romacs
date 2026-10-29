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
// terminal.c:536-585: the parameter alist of a live terminal.
fn assq(cx: &Leaf<'_>, key: Value, mut alist: Value) -> Value {
    while let Some((entry, rest)) = cx.pair(alist) {
        if cx.pair(entry).is_some_and(|(k, _)| k == key) { return entry; }
        alist = rest;
    }
    Value::NIL
}
defsubr! { name = "terminal-parameters", gnu = "terminal.c:553", effect = Reads,
fn terminal_parameters(cx; optional terminal: Value) {
    let (_, params) = cx.windows().terminal_params(terminal)?;
    // Fcopy_alist: fresh spine and fresh (PARM . VALUE) cells.
    let mut entries = Vec::new();
    let mut tail = params;
    while let Some((entry, rest)) = cx.pair(tail) {
        entries.push(match cx.pair(entry) { Some((k, v)) => cx.cons(k, v), None => entry });
        tail = rest;
    }
    Ok(entries.into_iter().rev().fold(Value::NIL, |tail, entry| cx.cons(entry, tail)))
}}
defsubr! { name = "terminal-parameter", gnu = "terminal.c:565", effect = Reads,
fn terminal_parameter(cx, terminal: Value, parameter: Value) {
    if cx.kind(parameter) != crate::prim::leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", parameter)); }
    let (_, params) = cx.windows().terminal_params(terminal)?;
    let entry = assq(cx, parameter, params);
    Ok(cx.pair(entry).map_or(Value::NIL, |(_, value)| value))
}}
defsubr! { name = "set-terminal-parameter", gnu = "terminal.c:575", effect = Global,
fn set_terminal_parameter(cx, terminal: Value, parameter: Value, value: Value) {
    let (terminal, params) = cx.windows().terminal_params(terminal)?;
    let entry = assq(cx, parameter, params);
    if let Some((_, old)) = cx.pair(entry) {
        cx.setcdr(entry, value)?;
        return Ok(old);
    }
    let entry = cx.cons(parameter, value);
    let params = cx.cons(entry, params);
    cx.windows().set_terminal_params(terminal, params)?;
    Ok(Value::NIL)
}}
// term.c:2284-2311: VM2's terminals are never text terminals
// (decode_tty_terminal is NULL for them), so no tty colours.
defsubr! { name = "tty-display-color-p", gnu = "term.c:2284", effect = Reads,
fn tty_display_color_p(cx; optional terminal: Value) {
    cx.windows().terminal_params(terminal)?;
    Ok(Value::NIL)
}}
defsubr! { name = "tty-display-color-cells", gnu = "term.c:2299", effect = Reads,
fn tty_display_color_cells(cx; optional terminal: Value) {
    cx.windows().terminal_params(terminal)?;
    Ok(Value::integer(0).unwrap())
}}
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
defsubr! { name = "set-window-combination-limit", gnu = "window.c:748", effect = Global,
fn set_window_combination_limit(cx, window: Value, limit: Value) { cx.windows().set_window_combination_limit(window, limit) }}
defsubr! { name = "window-hscroll", gnu = "window.c:1278", effect = Reads,
fn window_hscroll(cx; optional window: Value) { cx.windows().window_hscroll(window) }}
defsubr! { name = "set-window-hscroll", gnu = "window.c:1313", effect = Global,
fn set_window_hscroll(cx, window: Value, ncol: Value) { cx.windows().set_window_hscroll(window, ncol) }}
defsubr! { name = "select-frame", gnu = "frame.c:2103", effect = Global,
fn select_frame(cx, frame: Value; optional _norecord: Value) { cx.windows().select_frame(frame) }}
defsubr! { name = "recent-auto-save-p", gnu = "fileio.c:6541", effect = Reads,
fn recent_auto_save_p(cx) {
    let state = cx.buffer_modification(Value::NIL)?;
    Ok(if state.save_modiff < state.autosave_modiff { Value::TRUE } else { Value::NIL })
}}
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
    let mut seen = HashSet::default();
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

// GNU's internal state when VARIABLE is nil (dispnew.c frame_and_buffer_state).
const FRAME_BUFFER_STATE: &str = "#:vm2-frame-and-buffer-state";

/// The entries dispnew.c records: each frame and its name, then each listed
/// buffer (its name not starting with a space) with its read-only and
/// modified flags; and the count of all live buffers.
fn frame_buffer_entries(cx: &mut Leaf<'_>) -> std::result::Result<(Vec<Value>, usize), Fault> {
    let mut entries = Vec::new();
    let name_key = cx.intern("name");
    let mut frames = cx.windows().frame_list()?;
    while let Some((frame, rest)) = cx.pair(frames) {
        let name = cx.windows().frame_parameter_entries(frame)?.into_iter().find(|(k, _)| *k == name_key).map_or(Value::NIL, |(_, v)| v);
        entries.extend([frame, name]);
        frames = rest;
    }
    let read_only = cx.intern("buffer-read-only");
    let (mut buffers, mut count) = (cx.lifecycle_list(), 0);
    while let Some((buffer, rest)) = cx.pair(buffers) {
        count += 1;
        if !cx.buffer_state(buffer).unwrap().name.as_deref().is_some_and(|name| name.starts_with(' ')) {
            let flag = cx.buffer_local_read(read_only, buffer)?;
            let state = cx.buffer_modification(buffer)?;
            let modified = if state.save_modiff >= state.modiff { Value::NIL }
                else if state.autosave_modiff == state.modiff { cx.intern("autosaved") } else { Value::TRUE };
            entries.extend([buffer, flag, modified]);
        }
        buffers = rest;
    }
    Ok((entries, count))
}

defsubr! { name = "frame-or-buffer-changed-p", gnu = "dispnew.c:7048", effect = Global,
fn frame_or_buffer_changed_p(cx; optional variable: Value) {
    let lambda = cx.intern("lambda");
    let mut state = if variable == Value::NIL {
        match cx.hidden_value(FRAME_BUFFER_STATE) {
            Value::NIL => { let fresh = cx.vector(&[lambda; 20]); cx.set_hidden_value(FRAME_BUFFER_STATE, fresh); fresh }
            state => state,
        }
    } else {
        if cx.kind(variable) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", variable)); }
        cx.variable_value(variable)?.ok_or(Fault::Signal(cx.intern("void-variable"), cx.cons(variable, Value::NIL)))?
    };
    let (entries, buffers) = frame_buffer_entries(cx)?;
    let old = if cx.kind(state) == leaf::Kind::Vector { Some(cx.vector_values(state)?) } else { None };
    // Unchanged: every entry matches by eq and a lambda follows them.
    if let Some(old) = &old
        && old.len() > entries.len() && old.iter().zip(&entries).all(|(a, b)| a == b) && old[entries.len()] == lambda {
        return Ok(Value::NIL);
    }
    let frame_count = { let mut n = 0; let mut f = cx.windows().frame_list()?; while let Some((_, r)) = cx.pair(f) { n += 1; f = r; } n };
    let needed = 1 + 2 * frame_count + 3 * buffers;
    let size = old.as_ref().map_or(0, Vec::len);
    if old.is_none() || needed > size || needed + 20 < size / 2 {
        state = cx.vector(&vec![lambda; needed + 20]);
        if variable == Value::NIL { cx.set_hidden_value(FRAME_BUFFER_STATE, state); }
        else { cx.variable_set(variable, state)?; }
    }
    for (i, &entry) in entries.iter().enumerate() {
        cx.vector_set(state, Value::integer(i as i64).unwrap(), entry)?;
    }
    for i in entries.len()..cx.vector_len(state)? {
        cx.vector_set(state, Value::integer(i as i64).unwrap(), lambda)?;
    }
    Ok(Value::TRUE)
}}

defsubr! { name = "frame-initial-p", gnu = "terminal.c:482", effect = Reads,
fn frame_initial_p(cx; optional frame: Value) { cx.windows().frame_initial_p(frame) }}

defsubr! { name = "font-info", gnu = "font.c:5594", effect = Reads,
fn font_info(cx, name: Value; optional frame: Value) {
    // A VM2 frame's one font is the initial terminal's "tty" (VM2-WIN5),
    // whose metrics are the frame's canonical character, which the host
    // lays out: [OPENED-NAME FULL-NAME SIZE HEIGHT BASELINE-OFFSET
    // RELATIVE-COMPOSE DEFAULT-ASCENT MAX-WIDTH ASCENT DESCENT SPACE-WIDTH
    // AVERAGE-WIDTH FILENAME CAPABILITY].  Any other font does not open,
    // so its information is nil, as Ffont_info answers then.
    if !cx.string_view(name).is_ok_and(|n| n.bytes() == b"tty") {
        if cx.kind(name) != leaf::Kind::String { return Err(Fault::Wrong("stringp", name)); }
        return Ok(Value::NIL);
    }
    let width = cx.windows().frame_char_size(frame, true)?;
    let height = cx.windows().frame_char_size(frame, false)?;
    let zero = Value::integer(0).map_err(Fault::Error)?;
    let format = cx.intern("x");
    let capability = cx.cons(format, Value::NIL);
    Ok(cx.vector(&[name, name, height, height, zero, zero, zero, width, height, zero, width, width, Value::NIL, capability]))
}}
