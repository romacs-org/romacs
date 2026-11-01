//! What the display tells Lisp's animations (rlisp/romacs-animate.el).
use super::*;
use super::leaf_scalars::number;

// Whether this display shows animation drawn frame by frame (Lisp's,
// rlisp/romacs-animate.el): t on a GPU, nil on a software rasterizer (every
// headless harness) unless ROMACS_LAYER_ANIMATION is 1 or more, nil with no
// window, `unknown' until the window has chosen its adapter.
defsubr! { name = "romacs--display-animates", gnu = "romacs:display", effect = Reads,
fn display_animates(cx) { cx.display_animates() }}

// Have the display draw text animation ID over WINDOW's text by itself,
// frame after frame, with no Lisp per frame: ROWS by COLS cells from
// column 0 of WINDOW's visual row TOP (negative when scrolled above it),
// at FPS while MOVING, its cells taking SHADES shades, dark to light, each
// in the face WINDOW's text shows that shade in.  KIND says what draws the
// cells: (armillary PITCH YAW ZOOM CELL-ASPECT EPOCH), the sphere turned
// to that view and then by its motion, which a new EPOCH restarts.  A
// change keeps the motion; with WINDOW nil, ID is removed.  Signals where
// the display draws no text animation (`romacs--display-animates').
defsubr! { name = "romacs--text-animation", gnu = "romacs:display", effect = Global,
fn text_animation(cx, id: Value; optional window: Value, top: Value, cols: Value, rows: Value, fps: Value, moving: Value, shades: Value, kind: Value) {
    use romacs_display_protocol::text_animation::{TextAnimation, TextAnimationKind};
    let id = natural(id)?;
    if window == Value::NIL { return cx.text_animation(id, None); }
    let top = top.integer_value().and_then(|n| i32::try_from(n).ok()).ok_or(Fault::Wrong("fixnump", top))?;
    let shades = natural(shades)? as u32;
    let mut fields = vec![];
    let mut rest = kind;
    while let Some((head, tail)) = cx.pair(rest) { fields.push(head); rest = tail; }
    let armillary = cx.intern("armillary");
    let kind = match fields.as_slice() {
        &[head, pitch, yaw, zoom, cell_aspect, epoch] if head == armillary => TextAnimationKind::Armillary {
            pitch: number(cx, pitch)?, yaw: number(cx, yaw)?, zoom: number(cx, zoom)?,
            cell_aspect: number(cx, cell_aspect)?, epoch: natural(epoch)?,
        },
        _ => return Err(Fault::Wrong("romacs-text-animation-kind-p", kind)),
    };
    let animation = TextAnimation {
        window: cx.display_window_id(window)?, top, cols: natural(cols)? as u32, rows: natural(rows)? as u32,
        fps: natural(fps)? as u32, moving: moving != Value::NIL, shades, kind,
    };
    cx.text_animation(id, Some(animation))
}}

fn natural(value: Value) -> std::result::Result<u64, Fault> {
    value.integer_value().and_then(|n| u64::try_from(n).ok()).filter(|&n| n <= u64::from(u32::MAX)).ok_or(Fault::Wrong("natnump", value))
}
