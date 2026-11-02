//! What the display tells Lisp's animations (rlisp/romacs-animate.el).
//! The animations themselves travel with each frame (`Vm::text_animations').
use super::*;

// Whether this display shows animation drawn frame by frame (Lisp's,
// rlisp/romacs-animate.el): t on a GPU, nil on a software rasterizer (every
// headless harness) unless ROMACS_LAYER_ANIMATION is 1 or more, nil with no
// window, `unknown' until the window has chosen its adapter and while no
// host is attached.
defsubr! { name = "romacs--display-animates", gnu = "romacs:display", effect = Reads,
fn display_animates(cx) { cx.display_animates() }}
