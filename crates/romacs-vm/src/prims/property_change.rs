//! Sharing the native owners preserves overlay precedence during printer scans.
use super::*;

defsubr! { name = "next-property-change", gnu = "textprop.c:952", effect = Reads,
fn next_property_change(cx, position: Value; optional object: Value, limit: Value) {
    cx.next_property_change(position, object, limit)
}}

defsubr! { name = "next-single-char-property-change", gnu = "textprop.c:771", effect = Reads,
fn next_single_char_property_change(cx, position: Value, property: Value; optional object: Value, limit: Value) {
    cx.next_single_char_property_change(position, property, object, limit)
}}

defsubr! { name = "text-property-not-all", gnu = "textprop.c:1857", effect = Reads,
fn text_property_not_all(cx, start: Value, end: Value, property: Value, value: Value; optional object: Value) {
    cx.text_property_not_all(start, end, property, value, object)
}}
