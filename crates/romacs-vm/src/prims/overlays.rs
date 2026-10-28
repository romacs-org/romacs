//! GNU buffer.c overlay ownership and backwards boundary queries.
use super::*;

defsubr! { name = "delete-all-overlays", gnu = "buffer.c:3847", effect = Global,
fn delete_all_overlays(cx; optional buffer: Value) {
    cx.delete_all_overlays(buffer)
}}

defsubr! { name = "overlay-buffer", gnu = "buffer.c:3881", effect = Reads,
fn overlay_buffer(cx, overlay: Value) {
    cx.overlay_buffer(overlay)
}}

defsubr! { name = "overlay-properties", gnu = "buffer.c:3898", effect = Reads,
fn overlay_properties(cx, overlay: Value) {
    cx.overlay_plist(overlay)
}}

defsubr! { name = "previous-overlay-change", gnu = "buffer.c:4014", effect = Reads,
fn previous_overlay_change(cx, position: Value) {
    cx.previous_overlay_change(position)
}}
