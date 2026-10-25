//! Bulk edits reuse native annotation storage to preserve run ordering.
use super::*;

defsubr! { name = "add-text-properties", gnu = "textprop.c:1296", effect = Global,
fn add_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) {
    cx.add_text_properties(start, end, properties, object)
}}
