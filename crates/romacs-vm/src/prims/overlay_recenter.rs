//! GNU buffer.c:4033-4064: overlay-lists and overlay-recenter, which has
//! been a checked no-op since Emacs 29.1. Neither implementation needs a
//! movable overlay centre, so overlay-lists keeps no "after" list.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "overlay-recenter", gnu = "buffer.c:4051", effect = Pure,
fn overlay_recenter(cx, position: Value) {
    if position.integer_value().is_some() || cx.kind(position) == Kind::Bignum {
        return Ok(Value::NIL);
    }
    match cx.marker_position(position) {
        Some(Some(_)) => Ok(Value::NIL),
        Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
        None => Err(Fault::Wrong("integer-or-marker-p", position)),
    }
}}

defsubr! { name = "overlay-lists", gnu = "buffer.c:4033", effect = Reads,
fn overlay_lists(cx) {
    // Every overlay of the buffer, narrowing ignored, in a fresh list.
    let overlays = cx.current_overlays()?;
    let inner = overlays.into_iter().rev().fold(Value::NIL, |tail, overlay| cx.cons(overlay, tail));
    Ok(cx.cons(inner, Value::NIL))
}}
