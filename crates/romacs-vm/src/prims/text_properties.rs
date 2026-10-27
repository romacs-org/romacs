//! Bulk edits reuse native annotation storage to preserve run ordering.
use super::*;
use crate::buffer::PropertyEdit;
use crate::prim::leaf::{PropertyBegin, Retry};

// Saved registers of a property change waiting on its hooks:
//   [KIND, BEG, END, ARG, BUFFER, CHARS, PLIST, START, END-MARKER, POSITION, LENGTH]
// KIND is 0 add, 1 set, 2 remove, 3 put, 4/5 face prepend/append, 6
// remove-list; BEG and END the validated positions (markers the hooks move
// do not move the edit); ARG the plist, the names to remove as a list, or
// the face; the rest is GNU's retry state (Retry). The hooks run in Lisp (romacs--before-change and
// romacs--after-change in rlisp/vm/prelude.el); these registers are traced.
fn edit_of(cx: &mut Leaf<'_>, kind: Value, arg: Value) -> std::result::Result<PropertyEdit, Fault> {
    Ok(match kind.integer_value() {
        Some(0 | 3) => PropertyEdit::Add(arg),
        Some(1) => PropertyEdit::Set(arg),
        Some(4) => PropertyEdit::Face { face: arg, append: false },
        Some(6) => {
            let mut names = Vec::new();
            let mut tail = arg;
            while let Some((name, rest)) = cx.pair(tail) { names.push(name); tail = rest; }
            PropertyEdit::Remove(names)
        }
        Some(5) => PropertyEdit::Face { face: arg, append: true },
        _ => {
            let mut names = Vec::new();
            let mut tail = arg;
            while let Some((name, rest)) = cx.pair(tail) { names.push(name); tail = rest; }
            PropertyEdit::Remove(names)
        }
    })
}

fn property_change(cx: &mut Leaf<'_>, kind: i64, start: Value, end: Value, arg: Value, object: Value) -> Next {
    let kind = Value::integer(kind).unwrap();
    let edit = edit_of(cx, kind, arg)?;
    let put = matches!(kind.integer_value(), Some(3..=5));
    match cx.property_begin(&edit, start, end, object)? {
        PropertyBegin::Done(value) => Ok(Step::Done(if put { Value::NIL } else { value })),
        PropertyBegin::Hooks { buffer, start: beg, end: stop, modification_hooks, retry } => {
            let hooks = modification_hooks.into_iter().rev().fold(Value::NIL, |tail, h| cx.cons(h, tail));
            let (b, e) = (Value::integer(beg).unwrap(), Value::integer(stop).unwrap());
            let chars = cx.bigint(retry.chars.into());
            Ok(Step::Call { f: cx.intern("romacs--before-change"), args: vec![buffer, b, e, hooks], then: changed,
                saved: vec![kind, b, e, arg, buffer, chars, retry.plist, retry.start, retry.end,
                    Value::integer(retry.position).unwrap(), Value::integer(retry.length).unwrap()] })
        }
    }
}

fn changed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let edit = edit_of(cx, saved[0], saved[3])?;
    let chars = cx.bigint_value(saved[5]).and_then(|n| u64::try_from(n).ok()).expect("chars register");
    let retry = Retry { chars, plist: saved[6], start: saved[7], end: saved[8],
        position: saved[9].integer_value().unwrap(), length: saved[10].integer_value().unwrap() };
    let watches = saved[0].integer_value() != Some(6);
    let Some((beg, end)) = cx.property_finish(&edit, saved[1], saved[2], saved[4], retry, watches)? else {
        return Ok(Step::Done(Value::NIL));
    };
    let args = vec![saved[4], Value::integer(beg).unwrap(), Value::integer(end).unwrap(), Value::integer(end - beg).unwrap()];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: finished, saved: vec![saved[0]] })
}

fn finished(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(if matches!(saved[0].integer_value(), Some(3..=5)) { Value::NIL } else { Value::TRUE }))
}

fn list(cx: &mut Leaf<'_>, values: Vec<Value>) -> Value {
    values.into_iter().rev().fold(Value::NIL, |tail, v| cx.cons(v, tail))
}

defsubr! { name = "add-text-properties", gnu = "textprop.c:1296", effect = Global,
fn add_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) -> Step {
    property_change(cx, 0, start, end, properties, object)
}}

defsubr! { name = "set-text-properties", gnu = "textprop.c:1330", effect = Global,
fn set_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) -> Step {
    property_change(cx, 1, start, end, properties, object)
}}

// The names are PROPERTIES' keys: every other element, walked by cddr.
defsubr! { name = "remove-text-properties", gnu = "textprop.c:1554", effect = Global,
fn remove_text_properties(cx, start: Value, end: Value, properties: Value; optional object: Value) -> Step {
    let mut names = Vec::new();
    let mut tail = properties;
    for _ in 0..=cx.list_limit() {
        let Some((name, rest)) = cx.pair(tail) else { break };
        names.push(name);
        tail = cx.pair(rest).map_or(Value::NIL, |(_, next)| next);
    }
    let names = list(cx, names);
    property_change(cx, 2, start, end, names, object)
}}

defsubr! { name = "remove-list-of-text-properties", gnu = "textprop.c:1683", effect = Global,
fn remove_list_of_text_properties(cx, start: Value, end: Value, list_of_properties: Value; optional object: Value) -> Step {
    let mut names = Vec::new();
    let mut tail = list_of_properties;
    for _ in 0..=cx.list_limit() {
        let Some((name, rest)) = cx.pair(tail) else { break };
        names.push(name);
        tail = rest;
    }
    let names = list(cx, names);
    property_change(cx, 6, start, end, names, object)
}}

defsubr! { name = "get-pos-property", gnu = "editfns.c:275", effect = Reads,
fn get_pos_property(cx, position: Value, prop: Value; optional object: Value) {
    cx.get_pos_property(position, prop, object)
}}

// Fput_text_property: add_text_properties_1 with (PROPERTY VALUE), always nil.
defsubr! { name = "put-text-property", gnu = "textprop.c:1314", effect = Global,
fn put_text_property(cx, start: Value, end: Value, property: Value, value: Value; optional object: Value) -> Step {
    let tail = cx.cons(value, Value::NIL);
    let plist = cx.cons(property, tail);
    property_change(cx, 3, start, end, plist, object)
}}

// Fadd_face_text_property: FACE prepended (or with APPENDP appended) to each
// character's face, nondestructively; always nil.
defsubr! { name = "add-face-text-property", gnu = "textprop.c:1345", effect = Global,
fn add_face_text_property(cx, start: Value, end: Value, face: Value; optional appendp: Value, object: Value) -> Step {
    property_change(cx, if appendp == Value::NIL { 4 } else { 5 }, start, end, face, object)
}}

defsubr! { name = "next-single-property-change", gnu = "textprop.c:1017", effect = Reads,
fn next_single_property_change(cx, position: Value, prop: Value; optional object: Value, limit: Value) {
    cx.next_single_property_change(position, prop, object, limit)
}}

defsubr! { name = "previous-property-change", gnu = "textprop.c:1066", effect = Reads,
fn previous_property_change(cx, position: Value; optional object: Value, limit: Value) {
    if limit == Value::TRUE {
        return Err(Fault::Error("raw text-property interval boundaries (LIMIT t) are not implemented".into()));
    }
    cx.previous_property_change(position, None, object, limit)
}}

defsubr! { name = "previous-single-property-change", gnu = "textprop.c:1114", effect = Reads,
fn previous_single_property_change(cx, position: Value, prop: Value; optional object: Value, limit: Value) {
    cx.previous_property_change(position, Some(prop), object, limit)
}}
