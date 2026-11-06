//! GNU window.c `recenter', `scroll-up', `scroll-down' and
//! `move-to-window-line'. They move by screen lines and set the window's
//! start, calling `vertical-motion' and Lisp hooks at every step, so they
//! are Lisp here (rlisp/vm/window-scroll.el), installed by one `Step::Call'
//! of `eval' the first time one runs, as message.rs installs its half.
use super::*;

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/window-scroll.el");

/// Call HELPER of the Lisp half with ARGS, installing the half first;
/// then, when FOLLOW is a window, move its old point to its point
/// (`finish').
fn enter(cx: &mut Leaf<'_>, helper: &str, args: Vec<Value>, follow: Value) -> Next {
    let f = cx.intern(helper);
    if cx.symbol_function(f)? != Value::NIL {
        return Ok(Step::Call { f, args, then: finish, saved: vec![follow] });
    }
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    let saved = [follow, f].into_iter().chain(args).collect();
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then: installed, saved })
}

fn installed(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: saved[1], args: saved[2..].to_vec(), then: finish, saved: vec![saved[0]] })
}

/// GNU window_scroll (window.c:6324, :6806): a window whose point was its
/// old point before the scroll has its old point follow point, so that
/// scrolling does not end an explicit hscroll's hold. SAVED holds that
/// window, or nil.
fn finish(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    if saved[0] != Value::NIL { cx.windows().old_point_to_point(saved[0])?; }
    Ok(Step::Done(value))
}

/// The selected window when its point is its old point, else nil.
fn following(cx: &mut Leaf<'_>) -> std::result::Result<Value, Fault> {
    let window = cx.windows().selected_window()?;
    Ok(if cx.windows().old_point_is_point(window)? { window } else { Value::NIL })
}

defsubr! { name = "recenter", gnu = "window.c:7219", effect = Global,
fn recenter(cx; optional arg: Value, redisplay: Value) -> Step {
    enter(cx, "romacs--recenter", vec![arg, redisplay], Value::NIL)
}}

// The pixel branches of Frecenter and window_scroll_pixel_based, over rows
// a host lays out: nil where the Lisp half's lines serve.
defsubr! { name = "romacs--recenter-start", gnu = "romacs:display", effect = Reads,
fn recenter_start(cx, arg: Value, margin: Value, height: Value) {
    let window = cx.vm_selected_window()?;
    let lines = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    cx.window_recenter_start(window, arg.integer_value(), (lines(margin)?, lines(height)?))
}}

defsubr! { name = "romacs--scroll-start", gnu = "romacs:display", effect = Reads,
fn scroll_start(cx, start: Value, forward: Value, context: Value) {
    let window = cx.vm_selected_window()?;
    let number = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    cx.window_scroll_start(window, number(start)?, forward != Value::NIL, number(context)?)
}}

defsubr! { name = "romacs--rows-shown-from", gnu = "romacs:display", effect = Reads,
fn rows_shown_from(cx, start: Value) {
    let window = cx.vm_selected_window()?;
    cx.window_rows_shown_from(window, start.integer_value().ok_or(Fault::Wrong("fixnump", start))?)
}}

defsubr! { name = "scroll-up", gnu = "window.c:7026", effect = Global,
fn scroll_up(cx; optional arg: Value) -> Step {
    let follow = following(cx)?;
    enter(cx, "romacs--scroll-command", vec![Value::NIL, arg, Value::integer(1).expect("fixnum")], follow)
}}

defsubr! { name = "scroll-down", gnu = "window.c:7039", effect = Global,
fn scroll_down(cx; optional arg: Value) -> Step {
    let follow = following(cx)?;
    enter(cx, "romacs--scroll-command", vec![Value::NIL, arg, Value::integer(-1).expect("fixnum")], follow)
}}

defsubr! { name = "move-to-window-line", gnu = "window.c:7502", effect = Global,
fn move_to_window_line(cx, arg: Value) -> Step {
    enter(cx, "romacs--move-to-window-line", vec![arg], Value::NIL)
}}
