//! Bulk edits reuse native annotation storage to preserve run ordering.
use super::*;

defsubr! { name = "add-text-properties", gnu = "textprop.c:1296", effect = Global,
fn add_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) {
    cx.add_text_properties(start, end, properties, object)
}}

defsubr! { name = "set-text-properties", gnu = "textprop.c:1330", effect = Global,
fn set_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) {
    cx.set_text_properties(start, end, properties, object)
}}

// The names are PROPERTIES' keys: every other element, walked by cddr.
defsubr! { name = "remove-text-properties", gnu = "textprop.c:1554", effect = Global,
fn remove_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) {
    let mut names = Vec::new();
    let mut tail = properties;
    for _ in 0..=cx.list_limit() {
        let Some((name, rest)) = cx.pair(tail) else { break };
        names.push(name);
        tail = cx.pair(rest).map_or(Value::NIL, |(_, next)| next);
    }
    cx.remove_text_properties(start, end, &names, object)
}}

defsubr! { name = "remove-list-of-text-properties", gnu = "textprop.c:1683", effect = Global,
fn remove_list_of_text_properties(cx, start: Value, end: Value, list_of_properties: Value; optional object: Value) {
    let mut names = Vec::new();
    let mut tail = list_of_properties;
    for _ in 0..=cx.list_limit() {
        let Some((name, rest)) = cx.pair(tail) else { break };
        names.push(name);
        tail = rest;
    }
    cx.remove_text_properties(start, end, &names, object)
}}
