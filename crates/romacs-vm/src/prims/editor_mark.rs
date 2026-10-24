use super::*;

defsubr! { name = "mark-marker", gnu = "editfns.c:265", effect = Global,
fn mark_marker(cx) { cx.editor_mark_marker() }}

defsubr! { name = "region-beginning", gnu = "editfns.c:251", effect = Reads,
fn region_beginning(cx) { cx.editor_region_limit(true) }}

defsubr! { name = "region-end", gnu = "editfns.c:258", effect = Reads,
fn region_end(cx) { cx.editor_region_limit(false) }}
