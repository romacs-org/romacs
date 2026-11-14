//! Where redisplay calls `fontification-functions` (`src/prim/leaf/fontify_stops.rs`).
use super::{Fault, Value};

// (romacs--fontify-next WINDOW POS END &optional PAST): the first position
// from POS, before END, of the current buffer as WINDOW (nil: the selected
// window) shows it, at which GNU's display iterator would call
// `fontification-functions` (xdisp.c handle_fontified_prop): shown text whose
// `fontified` char property is nil, or the first position of a hidden
// stretch whose is; nil when there is none. PAST non-nil means POS was just
// called at: the walk first leaves its hidden stretch, or the run of
// `fontified` a call left nil. Redisplay's walk, `romacs--redisplay-fontify`.
defsubr! { name = "romacs--fontify-next", gnu = "romacs:display", effect = Reads,
fn fontify_next(cx, window: Value, pos: Value, end: Value; optional past: Value) {
    let number = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    let found = cx.fontify_next(window, number(pos)?, number(end)?, past != Value::NIL)?;
    found.map_or(Ok(Value::NIL), |at| Value::integer(at).map_err(Fault::Error))
}}
