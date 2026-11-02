//! Frame and INITIAL-terminal primitives; no TTY backend is implied.
use super::*; use crate::frame::ChildGeometry;

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
fn select_frame(cx, frame: Value; optional norecord: Value) { cx.windows().select_frame(frame, norecord) }}
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
    // The frame's font: the initial terminal's "tty", else the display's
    // font for the default face, named as GNU's pgtk names it.
    match super::font::frame_font(cx)? {
        Some(name) => {
            let name = cx.string(&name);
            store(&mut alist, cx.intern("font"), name);
            let backend = cx.intern(super::font::BACKEND);
            let backends = cx.cons(backend, Value::NIL);
            store(&mut alist, cx.intern("font-backend"), backends);
            let window_system = cx.windows().window_system(Value::NIL)?;
            store(&mut alist, cx.intern("window-system"), window_system);
        }
        None => { let tty = cx.string("tty"); store(&mut alist, cx.intern("font"), tty); }
    }
    store(&mut alist, name_key, name);
    let (cols, lines) = cx.windows().frame_text_size(frame)?;
    store(&mut alist, cx.intern("height"), Value::integer(lines).unwrap());
    store(&mut alist, cx.intern("width"), Value::integer(cols).unwrap());
    store(&mut alist, cx.intern("modeline"), Value::TRUE);
    if let Some(geometry) = cx.windows().child_geometry(frame)? {
        // A child frame's place in its parent and the parameters GNU
        // computes for it (pgtkfns.c x-create-frame).
        let child = cx.windows().decode_live_frame(frame)?;
        let parent = cx.windows().frame_parent(child)?;
        store(&mut alist, cx.intern("parent-frame"), parent);
        store(&mut alist, cx.intern("left"), Value::integer(i64::from(geometry.left)).unwrap());
        store(&mut alist, cx.intern("top"), Value::integer(i64::from(geometry.top)).unwrap());
        let mini = minibuffer_window(cx, child)?;
        store(&mut alist, cx.intern("minibuffer"), mini);
        let visible = if cx.windows().frame_visible(child)? { Value::TRUE } else { Value::NIL };
        store(&mut alist, cx.intern("visibility"), visible);
        for key in ["child-frame-border-width", "internal-border-width"] {
            let key = cx.intern(key);
            if !alist.iter().any(|(k, v)| *k == key && v.integer_value().is_some()) {
                store(&mut alist, key, Value::integer(i64::from(geometry.border)).unwrap());
            }
        }
        for key in ["no-accept-focus", "unsplittable"] {
            let key = cx.intern(key);
            if !alist.iter().any(|(k, _)| *k == key) { store(&mut alist, key, Value::NIL); }
        }
        store(&mut alist, cx.intern("menu-bar-lines"), Value::integer(0).unwrap());
        store(&mut alist, cx.intern("tab-bar-lines"), Value::integer(0).unwrap());
        store(&mut alist, cx.intern("buffer-list"), Value::NIL);
        store(&mut alist, cx.intern("buried-buffer-list"), Value::NIL);
        return Ok(alist);
    }
    store(&mut alist, cx.intern("unsplittable"), Value::NIL);
    // VM2 keeps no per-frame buffer selection history (VM2-FRAME1).
    store(&mut alist, cx.intern("buffer-list"), Value::NIL);
    store(&mut alist, cx.intern("buried-buffer-list"), Value::NIL);
    // make_initial_frame: one menu-bar line while menu-bar-mode is on.
    let menu = cx.intern("menu-bar-mode");
    let lines = if cx.variable_value(menu)?.is_some_and(|v| v != Value::NIL) { 1 } else { 0 };
    store(&mut alist, cx.intern("menu-bar-lines"), Value::integer(lines).unwrap());
    let ([left, right], tab_bar_lines) = cx.windows().frame_chrome(frame)?;
    store(&mut alist, cx.intern("tab-bar-lines"), Value::integer(i64::from(tab_bar_lines)).unwrap());
    store(&mut alist, cx.intern("visibility"), Value::TRUE);
    store(&mut alist, cx.intern("no-accept-focus"), Value::NIL);
    // A window-system frame stores every parameter it has a handler for,
    // nil when unset (pgtkfns.c:1599 gui_default_parameter), and reports
    // its fringes' widths (gui_report_frame_params).
    if cx.windows().window_system(frame)? != Value::NIL {
        store(&mut alist, cx.intern("left-fringe"), Value::integer(i64::from(left)).unwrap());
        store(&mut alist, cx.intern("right-fringe"), Value::integer(i64::from(right)).unwrap());
        for key in ["alpha", "alpha-background", "borders-respect-alpha-background"] {
            let key = cx.intern(key);
            if !alist.iter().any(|(k, _)| *k == key) { alist.insert(0, (key, Value::NIL)); }
        }
    }
    Ok(alist)
}

/// One opacity as gui_set_alpha_background reads it (frame.c:6098): nil is
/// None, else a float in [0, 1] or a percentage fixnum in [0, 100].
fn opacity(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<Option<f32>, Fault> {
    if value == Value::NIL { return Ok(None); }
    if let Ok(alpha) = cx.float_value(value) {
        if !(0.0..=1.0).contains(&alpha) { return Err(Fault::Range(cx.float(0.0), cx.float(1.0))); }
        return Ok(Some(alpha as f32));
    }
    match value.as_integer() {
        Ok(percent) if (0..=100).contains(&percent) => Ok(Some(percent as f32 / 100.0)),
        Ok(_) => Err(Fault::Range(Value::integer(0).unwrap(), Value::integer(100).unwrap())),
        Err(_) => Err(Fault::Wrong("numberp", value)),
    }
}

/// The opacity `frame-alpha-lower-limit' lets `alpha' reach, as
/// pgtk_set_frame_alpha reads it (pgtkterm.c:4677): above 1 none applies.
fn alpha_lower_limit(cx: &mut Leaf<'_>) -> std::result::Result<f32, Fault> {
    let symbol = cx.intern("frame-alpha-lower-limit");
    let limit = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
    Ok(match (cx.float_value(limit), limit.as_integer()) {
        (Ok(limit), _) => limit as f32,
        (_, Ok(percent)) => percent as f32 / 100.0,
        _ => 1.0,
    })
}

/// The window system's handler for KEY, run after FRAME stores VALUE
/// (frame.c gui_set_frame_parameters_1): `alpha' (gui_set_alpha,
/// frame.c:6047) is an opacity or a cons of the active and inactive ones,
/// nil leaving the window's alone; `alpha-background' (frame.c:6098) the
/// opacity of the backgrounds the frame paints; `left-fringe' and
/// `right-fringe' its fringes; `right-divider-width', `bottom-divider-width'
/// and `internal-border-width' the gaps between and around its windows;
/// `tab-bar-lines' its tab bar; `romacs-status-bar-lines' its status bar
/// and `romacs-pane-label' its windows' mode-line bands (VM2-STATUS-BAR). Other parameters, and
/// every parameter of a frame without a window system, are only stored.
fn handle_frame_parameter(cx: &mut Leaf<'_>, frame: Value, key: Value, value: Value) -> std::result::Result<(), Fault> {
    if cx.kind(key) != leaf::Kind::Symbol || cx.windows().window_system(frame)? == Value::NIL { return Ok(()); }
    let fringe = [cx.intern("left-fringe"), cx.intern("right-fringe")].iter().position(|&k| k == key);
    if let Some(side) = fringe {
        // gui_set_left_fringe (frame.c:5800): a fixnum's magnitude, else 8.
        let width = value.integer_value().map_or(crate::frame::DEFAULT_FRINGE, |n| n.unsigned_abs().min(i32::MAX as u64) as u32);
        cx.windows().set_frame_fringe(frame, side, width)
    } else if let Some(side) = [cx.intern("right-divider-width"), cx.intern("bottom-divider-width")].iter().position(|&k| k == key) {
        cx.windows().set_frame_divider(frame, side, natnum(value)?)
    } else if key == cx.intern("internal-border-width") {
        cx.windows().set_frame_internal_border(frame, natnum(value)?)
    } else if key == cx.intern("tab-bar-lines") {
        // pgtk_set_tab_bar_lines (pgtkfns.c:444): an integer >= 0, else 0.
        let lines = value.integer_value().filter(|&n| n >= 0).map_or(0, |n| n.min(i64::from(i32::MAX)) as u32);
        cx.windows().set_frame_tab_bar_lines(frame, lines)
    } else if key == cx.intern("romacs-status-bar-lines") {
        // Like `tab-bar-lines': an integer >= 1 shows the bar, else none.
        let lines = u32::from(value.integer_value().is_some_and(|n| n > 0));
        cx.windows().set_frame_status_bar_lines(frame, lines)
    } else if key == cx.intern("romacs-pane-label") {
        let label = if value == Value::NIL { crate::frame::PaneLabel::None }
            else if value == cx.intern("name") { crate::frame::PaneLabel::Name }
            else { crate::frame::PaneLabel::Full };
        cx.windows().set_frame_pane_label(frame, label)
    } else if key == cx.intern("alpha-background") {
        let background = opacity(cx, value)?.unwrap_or(1.0);
        cx.windows().set_frame_background_alpha(frame, background)
    } else if key == cx.intern("alpha") {
        let alpha = alpha_pair(cx, value)?;
        let limit = alpha_lower_limit(cx)?;
        cx.windows().set_frame_alpha(frame, alpha, limit)
    } else { Ok(()) }
}

/// check_int_nonnegative (bignum.c:506): an integer, 0 when negative.
fn natnum(value: Value) -> std::result::Result<u32, Fault> {
    let n = value.integer_value().ok_or(Fault::Wrong("integerp", value))?;
    u32::try_from(n.max(0)).ok().filter(|&n| n <= i32::MAX as u32)
        .ok_or_else(|| Fault::Range(Value::integer(0).unwrap(), Value::integer(i64::from(i32::MAX)).unwrap()))
}

/// `alpha' VALUE as gui_set_alpha reads it: the opacity while active and
/// inactive, from a number or a cons's car and cdr, -1 for nil.
fn alpha_pair(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<[f32; 2], Fault> {
    let (mut rest, mut alpha) = (value, [-1.0; 2]);
    for slot in &mut alpha {
        let item = match cx.pair(rest) { Some((car, cdr)) => { rest = cdr; car } None => rest };
        *slot = opacity(cx, item)?.unwrap_or(-1.0);
    }
    Ok(alpha)
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
    // list_length validates ALIST, Fcar and Fcdr split each element, then
    // entries are stored in reverse order, each before its handler runs.
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
    for &(key, value) in pairs.iter().rev() {
        cx.windows().store_frame_parameter(frame, key, value)?;
        handle_frame_parameter(cx, frame, key, value)?;
    }
    // A child frame applies its geometry and visibility (frame.c
    // gui_set_frame_parameters); the host frame's are the host window's.
    if cx.windows().is_child_frame(frame)? {
        apply_child_parameters(cx, frame, &pairs)?;
    }
    Ok(Value::NIL)
}}

/// The minibuffer window FRAME uses.
fn minibuffer_window(cx: &mut Leaf<'_>, frame: Value) -> std::result::Result<Value, Fault> {
    cx.windows().frame_minibuffer_window(frame)
}

/// A size parameter (frame.c gui_figure_window_size): an integer in
/// UNITs, `(text-pixels . N)' in pixels, a float a fraction of PARENT.
fn size_param(cx: &Leaf<'_>, value: Value, unit: u32, parent: u32) -> Option<u32> {
    let (head, tail) = cx.pair(value).unzip();
    let text_pixels = head.filter(|&h| cx.symbol_name(h).is_ok_and(|name| name == "text-pixels")).and(tail);
    if let Some(n) = text_pixels.and_then(|n| n.integer_value()) { return u32::try_from(n).ok(); }
    if let Some(n) = value.integer_value() { return u32::try_from(n).ok().map(|n| n * unit); }
    cx.float_value(value).ok().filter(|f| (0.0..=1.0).contains(f)).map(|f| (f * f64::from(parent)) as u32)
}

/// A position parameter (frame.c gui_figure_window_size): pixels from
/// the parent's left (top) edge, a negative integer or `(- N)' from its
/// right (bottom) edge, `(+ N)' from the left even when negative, a float
/// a fraction of the room the parent leaves.
fn position_param(cx: &Leaf<'_>, value: Value, room: i64) -> Option<i32> {
    let from_far = |n: i64| room + n;
    let n = if let Some(n) = value.integer_value() { if n < 0 { from_far(n) } else { n } }
    else if let Some((head, rest)) = cx.pair(value) {
        let n = cx.pair(rest)?.0.integer_value()?;
        match cx.symbol_name(head).ok()?.as_str() { "+" => n, "-" => from_far(-n), _ => return None }
    } else { (cx.float_value(value).ok().filter(|f| (0.0..=1.0).contains(f))? * room as f64) as i64 };
    i32::try_from(n).ok()
}

/// The geometry PAIRS ask for, from GEOMETRY.
fn requested_geometry(cx: &mut Leaf<'_>, parent: Value, mut geometry: ChildGeometry, pairs: &[(Value, Value)]) -> std::result::Result<ChildGeometry, Fault> {
    let lookup = |cx: &mut Leaf<'_>, name: &str| { let key = cx.intern(name); pairs.iter().find(|(k, _)| *k == key).map(|&(_, v)| v) };
    let (column, line) = (char_size(cx, true)?, char_size(cx, false)?);
    let (parent_width, parent_height) = (native(cx, parent, true)?, native(cx, parent, false)?);
    let border = ["child-frame-border-width", "internal-border-width"].iter()
        .find_map(|name| lookup(cx, name).and_then(|v| v.integer_value()).and_then(|n| u32::try_from(n).ok()));
    if let Some(border) = border { geometry.border = border; }
    if let Some(width) = lookup(cx, "width").and_then(|v| size_param(cx, v, column, parent_width)) { geometry.width = width; }
    if let Some(height) = lookup(cx, "height").and_then(|v| size_param(cx, v, line, parent_height)) { geometry.height = height; }
    let outer = |extent: u32| i64::from(extent + 2 * geometry.border);
    if let Some(left) = lookup(cx, "left").and_then(|v| position_param(cx, v, i64::from(parent_width) - outer(geometry.width))) { geometry.left = left; }
    if let Some(top) = lookup(cx, "top").and_then(|v| position_param(cx, v, i64::from(parent_height) - outer(geometry.height))) { geometry.top = top; }
    Ok(geometry)
}

fn char_size(cx: &mut Leaf<'_>, horizontal: bool) -> std::result::Result<u32, Fault> {
    Ok(cx.windows().frame_char_size(Value::NIL, horizontal)?.integer_value().and_then(|n| u32::try_from(n).ok()).unwrap_or(1).max(1))
}
fn native(cx: &mut Leaf<'_>, frame: Value, horizontal: bool) -> std::result::Result<u32, Fault> {
    Ok(cx.windows().frame_native_size(frame, horizontal)?.integer_value().and_then(|n| u32::try_from(n).ok()).unwrap_or(0))
}

/// Apply PAIRS to child FRAME: geometry, then visibility.
fn apply_child_parameters(cx: &mut Leaf<'_>, frame: Value, pairs: &[(Value, Value)]) -> std::result::Result<(), Fault> {
    let parent = cx.windows().frame_parent(frame)?;
    let geometry = cx.windows().child_geometry(frame)?.expect("child frame");
    let next = requested_geometry(cx, parent, geometry, pairs)?;
    if next != geometry { cx.windows().set_child_geometry(frame, next)?; }
    let visibility = cx.intern("visibility");
    if let Some(&(_, value)) = pairs.iter().find(|(k, _)| *k == visibility) {
        // `icon' iconifies, which a child frame shows as hidden.
        let icon = cx.intern("icon");
        cx.windows().set_frame_visible(frame, value != Value::NIL && value != icon)?;
    }
    Ok(())
}

// What pgtkfns.c x-create-frame does for a `parent-frame': Romacs's
// display makes child frames, painted over their parent (DIVERGENCES.md
// VM2-CHILD-FRAME); a top-level frame is refused (VM2-WIN-SUBSTRATE).
// Not named `x-create-frame': loadup.el and cus-start.el read that name
// as a window system's whole frame surface. romacs-pgtk.el's
// `frame-creation-function' method calls it for `make-frame'.
defsubr! { name = "romacs--create-frame", gnu = "pgtkfns.c:1197", effect = Global,
fn romacs_create_frame(cx, parms: Value) {
    let mut pairs = Vec::new();
    let mut tail = parms;
    while let Some((entry, rest)) = cx.pair(tail) {
        if let Some(pair) = cx.pair(entry) { pairs.push(pair); }
        tail = rest;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", parms)); }
    let lookup = |cx: &mut Leaf<'_>, name: &str| { let key = cx.intern(name); pairs.iter().find(|(k, _)| *k == key).map(|&(_, v)| v) };
    let parent = lookup(cx, "parent-frame").unwrap_or(Value::NIL);
    if parent == Value::NIL {
        return Err(Fault::Error("Romacs creates child frames only (a `parent-frame' parameter)".into()));
    }
    if cx.windows().frame_live_p(parent) == Value::NIL { return Err(Fault::Wrong("frame-live-p", parent)); }
    let (column, line) = (char_size(cx, true)?, char_size(cx, false)?);
    // GNU's default frame size, 80 columns by 36 lines.
    let start = ChildGeometry { left: 0, top: 0, width: 80 * column, height: 36 * line, border: 0 };
    let geometry = requested_geometry(cx, parent, start, &pairs)?;
    // x_create_frame stores every parameter it was given, then the name.
    let mut stored: Vec<(Value, Value)> = Vec::new();
    for &(key, value) in &pairs {
        if key != Value::NIL && !stored.iter().any(|(k, _)| *k == key) { stored.insert(0, (key, value)); }
    }
    let name_key = cx.intern("name");
    if !stored.iter().any(|(k, v)| *k == name_key && matches!(cx.kind(*v), leaf::Kind::String)) {
        let name = cx.string("romacs");
        stored.retain(|(k, _)| *k != name_key);
        stored.insert(0, (name_key, name));
    }
    let frame = cx.windows().create_child_frame(parent, geometry, stored)?;
    cx.face_copy_frame(parent, frame)?;
    // gui_default_parameter (pgtkfns.c:1599) on the frame already listed:
    // a refused value leaves it, not yet visible, as x_create_frame does.
    for name in ["alpha", "alpha-background", "left-fringe", "right-fringe"] {
        let key = cx.intern(name);
        if let Some(value) = lookup(cx, name) { handle_frame_parameter(cx, frame, key, value)?; }
    }
    let visibility = lookup(cx, "visibility");
    let icon = cx.intern("icon");
    cx.windows().set_frame_visible(frame, visibility.is_none_or(|v| v != Value::NIL && v != icon))?;
    Ok(frame)
}}

// The frame reserves no line for a minibuffer window a child frame hosts
// (DIVERGENCES.md VM2-MINI-COMPLETION-LAYER); romacs-complete.el places it.
defsubr! { name = "romacs--host-minibuffer", gnu = "romacs:frame", effect = Global,
fn romacs_host_minibuffer(cx, frame: Value) {
    cx.windows().host_minibuffer(frame)
}}
defsubr! { name = "romacs-status-bar-window", gnu = "romacs:frame", effect = Reads,
fn romacs_status_bar_window(cx) {
    cx.windows().status_bar_window()
}}
defsubr! { name = "romacs-status-bar-height", gnu = "romacs:frame", effect = Reads,
fn romacs_status_bar_height(cx) {
    Ok(Value::integer(i64::from(cx.windows().status_bar_height())).unwrap())
}}
defsubr! { name = "romacs--status-bar-text", gnu = "romacs:frame", effect = Reads,
fn romacs_status_bar_text(cx) {
    Ok(cx.windows().status_bar_text())
}}
defsubr! { name = "romacs--minibuffer-host", gnu = "romacs:frame", effect = Reads,
fn romacs_minibuffer_host(cx) {
    cx.windows().minibuffer_host()
}}

// frame.c Fdelete_frame: the hooks around delete_frame's deletion.
// Descendants are deleted without running theirs (VM2-CHILD-FRAME).
defsubr! { name = "delete-frame", gnu = "frame.c:3150", effect = Global,
fn delete_frame(cx; optional frame: Value, force: Value) -> Step {
    let target = if frame == Value::NIL { cx.windows().decode_live_frame(Value::NIL)? } else { frame };
    if cx.windows().frame_p(target) == Value::NIL { return Err(Fault::Wrong("framep", target)); }
    if cx.windows().frame_live_p(target) == Value::NIL { return Ok(Step::Done(Value::NIL)); }
    if !cx.windows().is_child_frame(target)? { cx.windows().delete_frame(target, force)?; }
    run_frame_hook(cx, "delete-frame-functions", target, delete_after_hook, vec![target, force])
}}
fn run_frame_hook(cx: &mut Leaf<'_>, hook: &str, frame: Value, then: fn(&mut Leaf<'_>, Value, &[Value]) -> Next, saved: Vec<Value>) -> Next {
    let run = cx.intern("run-hook-with-args");
    if !cx.fboundp(run) { return then(cx, Value::NIL, &saved); }
    let hook = cx.intern(hook);
    Ok(Step::Call { f: run, args: vec![hook, frame], then, saved })
}
fn delete_after_hook(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    // The hook may have deleted the frame already.
    if cx.windows().frame_live_p(saved[0]) == Value::NIL { return Ok(Step::Done(Value::NIL)); }
    cx.windows().delete_frame(saved[0], saved[1])?;
    run_frame_hook(cx, "after-delete-frame-functions", saved[0], deleted, vec![])
}
fn deleted(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }

defsubr! { name = "frame-parent", gnu = "frame.c:2180", effect = Reads,
fn frame_parent(cx; optional frame: Value) { cx.windows().frame_parent(frame) }}
defsubr! { name = "frame-ancestor-p", gnu = "frame.c:2242", effect = Reads,
fn frame_ancestor_p(cx, ancestor: Value, descendant: Value) { cx.windows().frame_ancestor_p(ancestor, descendant) }}
defsubr! { name = "redirect-frame-focus", gnu = "frame.c:3702", effect = Global,
fn redirect_frame_focus(cx, frame: Value; optional focus_frame: Value) { cx.windows().redirect_frame_focus(frame, focus_frame) }}
defsubr! { name = "frame-focus", gnu = "frame.c:3746", effect = Reads,
fn frame_focus(cx; optional frame: Value) { cx.windows().frame_focus(frame) }}
// frame.c:7042: the flag only matters to an X window manager's map event.
defsubr! { name = "frame--set-was-invisible", gnu = "frame.c:7042", effect = Global,
fn frame_set_was_invisible(cx, frame: Value, was_invisible: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(if was_invisible == Value::NIL { Value::NIL } else { Value::TRUE })
}}

// frame.c Fframe_position: a child frame's outer position in its
// parent; the host frame has none on a Wayland display.
defsubr! { name = "frame-position", gnu = "frame.c:4644", effect = Reads,
fn frame_position(cx; optional frame: Value) {
    let (left, top) = cx.windows().child_geometry(frame)?.map_or((0, 0), |g| (g.left, g.top));
    Ok(cx.cons(Value::integer(i64::from(left)).unwrap(), Value::integer(i64::from(top)).unwrap()))
}}
defsubr! { name = "set-frame-position", gnu = "frame.c:4662", effect = Global,
fn set_frame_position(cx, frame: Value, x: Value, y: Value) {
    let frame = cx.windows().decode_live_frame(frame)?;
    let (x, y) = (x.integer_value().ok_or(Fault::Wrong("fixnump", x))?, y.integer_value().ok_or(Fault::Wrong("fixnump", y))?);
    if let Some(mut geometry) = cx.windows().child_geometry(frame)? {
        let parent = cx.windows().frame_parent(frame)?;
        let outer = |extent: u32| i64::from(extent + 2 * geometry.border);
        let (pw, ph) = (native(cx, parent, true)?, native(cx, parent, false)?);
        // A negative X or Y counts from the parent's right or bottom edge.
        let place = |n: i64, room: i64| i32::try_from(if n < 0 { room + n } else { n }).unwrap_or(0);
        geometry.left = place(x, i64::from(pw) - outer(geometry.width));
        geometry.top = place(y, i64::from(ph) - outer(geometry.height));
        cx.windows().set_child_geometry(frame, geometry)?;
    }
    Ok(Value::TRUE)
}}
/// Resize FRAME's text area to WIDTH by HEIGHT columns and lines, or
/// pixels with PIXELWISE; nil keeps that extent.
fn resize_frame(cx: &mut Leaf<'_>, frame: Value, width: Option<Value>, height: Option<Value>, pixelwise: bool) -> std::result::Result<(), Fault> {
    let frame = cx.windows().decode_live_frame(frame)?;
    let size = |cx: &mut Leaf<'_>, v: Value, horizontal: bool| -> std::result::Result<u32, Fault> {
        let n = v.integer_value().ok_or(Fault::Wrong("fixnump", v))?;
        let unit = if pixelwise { 1 } else { char_size(cx, horizontal)? };
        Ok(u32::try_from(n.max(0)).unwrap_or(0) * unit)
    };
    let width = width.map(|v| size(cx, v, true)).transpose()?;
    let height = height.map(|v| size(cx, v, false)).transpose()?;
    if let Some(mut geometry) = cx.windows().child_geometry(frame)? {
        if let Some(width) = width { geometry.width = width; }
        if let Some(height) = height { geometry.height = height; }
        cx.windows().set_child_geometry(frame, geometry)?;
    }
    Ok(())
}
defsubr! { name = "set-frame-size", gnu = "frame.c:4621", effect = Global,
fn set_frame_size(cx, frame: Value, width: Value, height: Value; optional pixelwise: Value) {
    resize_frame(cx, frame, Some(width), Some(height), pixelwise != Value::NIL)?;
    Ok(Value::NIL)
}}
defsubr! { name = "set-frame-width", gnu = "frame.c:4592", effect = Global,
fn set_frame_width(cx, frame: Value, width: Value; optional _pretend: Value, pixelwise: Value) {
    resize_frame(cx, frame, Some(width), None, pixelwise != Value::NIL)?;
    Ok(Value::NIL)
}}
defsubr! { name = "set-frame-height", gnu = "frame.c:4563", effect = Global,
fn set_frame_height(cx, frame: Value, height: Value; optional _pretend: Value, pixelwise: Value) {
    resize_frame(cx, frame, None, Some(height), pixelwise != Value::NIL)?;
    Ok(Value::NIL)
}}
// frame.c:4447-4490: the text area in pixels and in characters, without
// the fringes, the tab bar and the internal border; the total columns and
// lines are the native size's (Romacs draws no scroll bars).
fn text_extent(cx: &mut Leaf<'_>, frame: Value, horizontal: bool, chars: bool) -> super::Out {
    let (width, height) = cx.windows().frame_text_area(frame)?;
    let pixels = if horizontal { width } else { height };
    let unit = if chars { char_size(cx, horizontal)? } else { 1 };
    Ok(Value::integer(i64::from(pixels / unit)).unwrap())
}
defsubr! { name = "frame-text-width", gnu = "frame.c:4475", effect = Reads,
fn frame_text_width(cx; optional frame: Value) { text_extent(cx, frame, true, false) }}
defsubr! { name = "frame-text-height", gnu = "frame.c:4482", effect = Reads,
fn frame_text_height(cx; optional frame: Value) { text_extent(cx, frame, false, false) }}
defsubr! { name = "frame-text-cols", gnu = "frame.c:4447", effect = Reads,
fn frame_text_cols(cx; optional frame: Value) { text_extent(cx, frame, true, true) }}
defsubr! { name = "frame-text-lines", gnu = "frame.c:4454", effect = Reads,
fn frame_text_lines(cx; optional frame: Value) { text_extent(cx, frame, false, true) }}
defsubr! { name = "frame-total-cols", gnu = "frame.c:4461", effect = Reads,
fn frame_total_cols(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(Value::integer(i64::from(native(cx, frame, true)? / char_size(cx, true)?)).unwrap())
}}
defsubr! { name = "frame-total-lines", gnu = "frame.c:4468", effect = Reads,
fn frame_total_lines(cx; optional frame: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(Value::integer(i64::from(native(cx, frame, false)? / char_size(cx, false)?)).unwrap())
}}
defsubr! { name = "window-system", gnu = "frame.c:415", effect = Reads,
fn window_system(cx; optional frame: Value) { cx.windows().window_system(frame) }}
defsubr! { name = "current-message", gnu = "editfns.c:3288", effect = Reads,
fn current_message(cx) {
    // In batch, message writes to stderr and the echo area stays empty.
    let noninteractive = cx.intern("noninteractive");
    if cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) != Value::NIL { return Ok(Value::NIL); }
    let echo = cx.intern(crate::prim::ECHO_AREA_MESSAGE);
    Ok(cx.variable_value(echo)?.unwrap_or(Value::NIL))
}}
defsubr! { name = "internal-handle-focus-in", gnu = "keyboard.c:7891", effect = Reads,
fn internal_handle_focus_in(cx, event: Value) {
    // GNU queues a switch-frame event when the focused frame is not the
    // selected one; a VM2 session has one frame (`frame-list'), always
    // selected, so there is never one to queue.
    let focus_in = cx.intern("focus-in");
    let frame = cx.pair(event).filter(|&(head, _)| head == focus_in).and_then(|(_, rest)| cx.pair(rest)).map(|(frame, _)| frame);
    if !frame.is_some_and(|frame| cx.windows().frame_p(frame) != Value::NIL) {
        return Err(Fault::Error("Invalid focus-in event".into()));
    }
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

// frame.c:3772 Fframe_after_make_frame: FRAME may run its window change
// functions from now on; Romacs runs none for a frame (VM2-CHILD-FRAME).
defsubr! { name = "frame-after-make-frame", gnu = "frame.c:3772", effect = Global,
fn frame_after_make_frame(cx, frame: Value, made: Value) {
    cx.windows().decode_live_frame(frame)?;
    Ok(made)
}}

// xdisp.c:15013: the lines, or with PIXELWISE the pixels, of FRAME's tab
// bar; a child frame shows none.
defsubr! { name = "tab-bar-height", gnu = "xdisp.c:15013", effect = Reads,
fn tab_bar_height(cx; optional frame: Value, pixelwise: Value) {
    let frame = cx.windows().decode_live_frame(frame)?;
    let lines = if cx.windows().is_child_frame(frame)? { 0 } else { i64::from(cx.windows().tab_bar_lines()) };
    let unit = if pixelwise != Value::NIL { i64::from(char_size(cx, false)?) } else { 1 };
    Ok(Value::integer(lines * unit).unwrap())
}}

// frame.c:4517: FRAME_TOTAL_FRINGE_WIDTH, none on a frame without a
// window system.
defsubr! { name = "frame-fringe-width", gnu = "frame.c:4517", effect = Reads,
fn frame_fringe_width(cx; optional frame: Value) {
    let ([left, right], _) = cx.windows().frame_chrome(frame)?;
    let shown = cx.windows().window_system(frame)? != Value::NIL;
    Ok(Value::integer(if shown { i64::from(left + right) } else { 0 }).unwrap())
}}
