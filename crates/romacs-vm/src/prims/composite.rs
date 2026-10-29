//! GNU composite.c primitives.
use super::*;

// GNU replaces its gstring cache and clears the face cache. VM2 composes no
// characters and keeps neither cache (DIVERGENCES.md VM2-COMPOSITE1).
defsubr! { name = "clear-composition-cache", gnu = "composite.c:692", effect = Global,
fn clear_composition_cache(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}

// composite.c:624 compose_text: the `composition' text property
// ((LENGTH . COMPONENTS) . MODIFICATION-FUNC) over FROM..TO of OBJECT, which
// redisplay and `find-composition' read. VM2's display draws no
// compositions (VM2-COMPOSITE1); the property is GNU's.
fn compose_text(cx: &mut Leaf<'_>, from: i64, to: i64, components: Value, function: Value, object: Value, result: Value) -> Next {
    let length = Value::integer(to - from).map_err(Fault::Error)?;
    let head = cx.cons(length, components);
    let prop = cx.cons(head, function);
    let args = vec![Value::integer(from).unwrap(), Value::integer(to).unwrap(), cx.intern("composition"), prop, object];
    Ok(Step::Call { f: cx.intern("put-text-property"), args, then: composed, saved: vec![result] })
}

fn composed(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[0]))
}

defsubr! { name = "compose-region-internal", gnu = "composite.c:2009", effect = Global,
fn compose_region_internal(cx, start: Value, end: Value; optional components: Value, modification_func: Value) -> Step {
    let (from, to) = cx.validate_region(start, end)?;
    let kind = cx.kind(components);
    if components != Value::NIL && components.integer_value().is_none() && cx.pair(components).is_none()
        && !matches!(kind, crate::prim::leaf::Kind::String | crate::prim::leaf::Kind::Bytes | crate::prim::leaf::Kind::Vector) {
        return Err(Fault::Wrong("vectorp", components));
    }
    compose_text(cx, from, to, components, modification_func, Value::NIL, Value::NIL)
}}

defsubr! { name = "compose-string-internal", gnu = "composite.c:2029", effect = Global,
fn compose_string_internal(cx, string: Value, start: Value, end: Value; optional components: Value, modification_func: Value) -> Step {
    // CHECK_STRING, then validate_subarray (fns.c).
    let size = cx.string_view(string).map_err(|_| Fault::Wrong("stringp", string))?.chars as i64;
    let bound = |v: Value, default: i64| -> std::result::Result<i64, Fault> {
        if v == Value::NIL { return Ok(default); }
        let n = v.integer_value().ok_or(Fault::Wrong("integerp", v))?;
        Ok(if n < 0 { n + size } else { n })
    };
    let (from, to) = (bound(start, 0)?, bound(end, size)?);
    if !(0 <= from && from <= to && to <= size) {
        return Err(Fault::Range3(string, start, end));
    }
    compose_text(cx, from, to, components, modification_func, string, string)
}}
