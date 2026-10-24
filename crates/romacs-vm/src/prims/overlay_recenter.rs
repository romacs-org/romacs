//! GNU buffer.c:4051-4064: overlay-recenter has been a checked no-op
//! since Emacs 29.1. Neither implementation needs a movable overlay centre.
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
