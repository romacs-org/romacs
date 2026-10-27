//! GNU editfns.c translate-region-internal.
use super::*;

defsubr! { name = "translate-region-internal", gnu = "editfns.c:2509", effect = Global,
fn translate_region_internal(cx, start: Value, end: Value, table: Value) {
    cx.translate_region_internal(start, end, table)
}}
