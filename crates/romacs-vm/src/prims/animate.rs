//! What the display tells Lisp's animations (lisp/ui/romacs-animate.el).
//! The animations themselves travel with each frame (`Vm::text_animations').
use super::*;

// Whether this display shows animation drawn frame by frame (Lisp's,
// lisp/ui/romacs-animate.el): t on a GPU, nil on a software rasterizer (every
// headless harness) unless ROMACS_LAYER_ANIMATION is 1 or more, nil with no
// window, `unknown' until the window has chosen its adapter and while no
// host is attached.
defsubr! { name = "romacs--display-animates", gnu = "romacs:display", effect = Reads,
fn display_animates(cx) { cx.display_animates() }}

// The host frame's native window, for a server session whose window comes
// and goes (lisp/session/romacs-server.el): ACTION `show' makes it anew when it was
// closed and asks for the focus, `hide' closes it and keeps the session,
// `query' only asks. t when a window shows after the request.
// What startup is doing, for the screen a launch shows until its first
// frame: STAGE a symbol or string naming the step (`init', `agents',
// `restore'), LABEL what it is at, or nil.  Nothing once the session's
// first frame shows, or with no display.
defsubr! { name = "romacs-startup-progress", gnu = "romacs:display", effect = Global,
fn startup_progress(cx, stage: Value; optional label: Value) { cx.startup_progress(stage, label) }}
defsubr! { name = "romacs--pointer", gnu = "romacs:display", effect = Reads,
fn pointer(cx) { cx.pointer() }}
defsubr! { name = "romacs--native-window", gnu = "romacs:display", effect = Global,
fn native_window(cx, action: Value) { cx.native_window(action) }}
