//! GNU window.c `recenter', `scroll-up', `scroll-down' and
//! `move-to-window-line'. They move by screen lines and set the window's
//! start, calling `vertical-motion' and Lisp hooks at every step, so they
//! are Lisp here (rlisp/vm/window-scroll.el), installed by one `Step::Call'
//! of `eval' the first time one runs, as message.rs installs its half.
use super::*;

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/window-scroll.el");

/// Call HELPER of the Lisp half with ARGS, installing the half first.
fn enter(cx: &mut Leaf<'_>, helper: &str, args: Vec<Value>) -> Next {
    let f = cx.intern(helper);
    if cx.symbol_function(f)? != Value::NIL {
        return Ok(Step::Call { f, args, then: done, saved: vec![] });
    }
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    let saved = std::iter::once(f).chain(args).collect();
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then: installed, saved })
}

fn installed(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: saved[0], args: saved[1..].to_vec(), then: done, saved: vec![] })
}

fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

defsubr! { name = "recenter", gnu = "window.c:7219", effect = Global,
fn recenter(cx; optional arg: Value, redisplay: Value) -> Step {
    enter(cx, "romacs--recenter", vec![arg, redisplay])
}}

defsubr! { name = "scroll-up", gnu = "window.c:7026", effect = Global,
fn scroll_up(cx; optional arg: Value) -> Step {
    enter(cx, "romacs--scroll-command", vec![Value::NIL, arg, Value::integer(1).expect("fixnum")])
}}

defsubr! { name = "scroll-down", gnu = "window.c:7039", effect = Global,
fn scroll_down(cx; optional arg: Value) -> Step {
    enter(cx, "romacs--scroll-command", vec![Value::NIL, arg, Value::integer(-1).expect("fixnum")])
}}

defsubr! { name = "move-to-window-line", gnu = "window.c:7502", effect = Global,
fn move_to_window_line(cx, arg: Value) -> Step {
    enter(cx, "romacs--move-to-window-line", vec![arg])
}}
