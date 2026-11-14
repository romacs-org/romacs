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
defsubr! { name = "romacs--pointer", gnu = "romacs:display", effect = Reads,
fn pointer(cx) { cx.pointer() }}
defsubr! { name = "romacs--native-window", gnu = "romacs:display", effect = Global,
fn native_window(cx, action: Value) { cx.native_window(action) }}
