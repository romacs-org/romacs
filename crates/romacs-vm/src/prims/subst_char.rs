//! GNU editfns.c subst-char-in-region.
use super::*;

defsubr! { name = "subst-char-in-region", gnu = "editfns.c:2294", effect = Global,
fn subst_char_in_region(cx, start: Value, end: Value, fromchar: Value, tochar: Value; optional noundo: Value) {
    cx.subst_char_in_region(start, end, fromchar, tochar, noundo)
}}
