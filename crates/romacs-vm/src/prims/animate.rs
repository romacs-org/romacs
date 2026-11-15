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
// showed it, its counts and failure as a bar of its own.
defsubr! { name = "romacs-startup-progress", gnu = "romacs:display", effect = Global,
fn startup_progress(cx, stage: Value; optional label: Value, state: Value) { cx.startup_progress(stage, label, state) }}
defsubr! { name = "romacs--pointer", gnu = "romacs:display", effect = Reads,
fn pointer(cx) { cx.pointer() }}
defsubr! { name = "romacs--native-window", gnu = "romacs:display", effect = Global,
fn native_window(cx, action: Value) { cx.native_window(action) }}
