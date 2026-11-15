//! What the display tells Lisp's animations (lisp/romacs/romacs-animate.el).
//! The animations themselves travel with each frame (`Vm::text_animations').
use super::*;

// Whether this display shows animation drawn frame by frame (Lisp's,
// lisp/romacs/romacs-animate.el): t on a GPU, nil on a software rasterizer (every
// headless harness) unless ROMACS_LAYER_ANIMATION is 1 or more, nil with no
// window, `unknown' until the window has chosen its adapter and while no
// host is attached.
defsubr! { name = "romacs--display-animates", gnu = "romacs:display", effect = Reads,
fn display_animates(cx) { cx.display_animates() }}

// The host frame's native window, for a server session whose window comes
// and goes (lisp/romacs/romacs-server.el): ACTION `show' makes it anew when it was
// closed and asks for the focus, `hide' closes it and keeps the session,
// `query' only asks. t when a window shows after the request.
// What startup is doing: STAGE a symbol or string naming the step (`init',
// `agents', `restore'), LABEL what it is at, or nil, for the screen a
// launch shows until its first frame (nothing once that frame shows, or
// with no display); and STATE, a plist (:status, :done, :of, :name,
// :error, :seconds), kept with LABEL as STAGE's latest in
// `romacs-progress-state' whatever the display shows, where a restart's
// supervisor reads it over the server's socket and the progress card after
// the first frame does (romacs-progress-view.el).  t when the launch screen
// showed it, its counts and failure as a bar of its own; with a STATE,
// `romacs-progress-view-update' is told, when it is defined (the card past
// the screen).
defsubr! { name = "romacs-startup-progress", gnu = "romacs:display", effect = Global,
fn startup_progress(cx, stage: Value; optional label: Value, state: Value) -> Step {
    let shown = cx.startup_progress(stage, label, state)?;
    // The view runs past the screen, and for a step the screen shows that
    // goes on (it moves to the card once the first frame is up); never for
    // a stage with no state, as the init's loads are.
    let running = { let key = cx.intern(":status"); let status = cx.plist_value(state, key); cx.kind(status) == Kind::Symbol && cx.symbol_name(status).is_ok_and(|s| s == "running") };
    let view = cx.intern("romacs-progress-view-update");
    if (shown != Value::NIL && !running) || state == Value::NIL || !cx.fboundp(view) { return Ok(Step::Done(shown)); }
    Ok(Step::Call { f: view, args: vec![shown], then: shown_by_view, saved: vec![shown] })
}}
fn shown_by_view(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { Ok(Step::Done(saved[0])) }
defsubr! { name = "romacs--pointer", gnu = "romacs:display", effect = Reads,
fn pointer(cx) { cx.pointer() }}
defsubr! { name = "romacs--native-window", gnu = "romacs:display", effect = Global,
fn native_window(cx, action: Value) { cx.native_window(action) }}
