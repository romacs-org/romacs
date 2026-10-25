use super::*;

defsubr! { name = "mark-marker", gnu = "editfns.c:265", effect = Global,
fn mark_marker(cx) { cx.editor_mark_marker() }}

defsubr! { name = "region-beginning", gnu = "editfns.c:251", effect = Reads,
fn region_beginning(cx) { cx.editor_region_limit(true) }}

defsubr! { name = "region-end", gnu = "editfns.c:258", effect = Reads,
fn region_end(cx) { cx.editor_region_limit(false) }}

defsubr! { name = "point-marker", gnu = "editfns.c:198", effect = Global,
fn point_marker(cx) { cx.editor_point_marker() }}

defsubr! { name = "point-min-marker", gnu = "editfns.c:887", effect = Global,
fn point_min_marker(cx) {
    let position = cx.buffer_shape()?.begv;
    cx.editor_position_marker(position)
}}

defsubr! { name = "point-max-marker", gnu = "editfns.c:906", effect = Global,
fn point_max_marker(cx) {
    let position = cx.buffer_shape()?.zv;
    cx.editor_position_marker(position)
}}
