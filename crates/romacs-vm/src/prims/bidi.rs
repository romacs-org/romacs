//! bidi.c and xdisp.c's paragraph direction (crate::bidi).
use super::*;

defsubr! { name = "current-bidi-paragraph-direction", gnu = "xdisp.c:26932", effect = Reads,
fn current_bidi_paragraph_direction(cx; optional buffer: Value) {
    cx.current_bidi_paragraph_direction(buffer)
}}
