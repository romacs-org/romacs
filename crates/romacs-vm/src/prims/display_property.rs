//! xdisp.c's `get-display-property`: one specification of a `display`
//! property, read as redisplay reads it.
use super::{Fault, Leaf, Next, Step, Value};

// Fget_display_property: SPEC's value in the `display' property at POSITION
// of OBJECT, or in PROPERTIES when that is given.
defsubr! { name = "get-display-property", gnu = "xdisp.c:5877", effect = Reads,
fn get_display_property(cx, position: Value, spec: Value; optional object: Value, properties: Value) -> Step {
    if properties != Value::NIL {
        if cx.pair(properties).is_none() { return Err(Fault::Wrong("listp", properties)); }
        return Ok(Step::Done(find_display_property(cx, properties, spec)?));
    }
    let (f, display) = (cx.intern("get-text-property"), cx.intern("display"));
    Ok(Step::Call { f, args: vec![position, display, object], then: display_read, saved: vec![spec] })
}}

fn display_read(cx: &mut Leaf<'_>, properties: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(find_display_property(cx, properties, saved[0])?))
}

/// xdisp.c `find_display_property`: in a vector or list of display
/// specifications, or one specification, the first `(SPEC VALUE...)`; its
/// VALUE when it has one, else its VALUE list. A list stops at an improper
/// tail, as GNU's walk does.
fn find_display_property(cx: &mut Leaf<'_>, display: Value, spec: Value) -> std::result::Result<Value, Fault> {
    let named = |cx: &Leaf<'_>, element: Value| cx.pair(element).is_some_and(|(car, cdr)| car == spec && cx.pair(cdr).is_some());
    let found = if cx.kind(display) == crate::prim::leaf::Kind::Vector {
        let mut found = None;
        for i in 0..cx.vector_len(display)? {
            let element = cx.vector_get(display, Value::integer(i64::try_from(i).expect("a vector index is a fixnum")).expect("vector index"))?;
            if named(cx, element) { found = Some(element); break; }
        }
        found
    } else if cx.pair(display).is_some_and(|(car, _)| cx.pair(car).is_some()) {
        let mut tail = display;
        let mut found = None;
        while let Some((element, rest)) = cx.pair(tail) {
            if named(cx, element) { found = Some(element); break; }
            tail = rest;
        }
        found
    } else {
        named(cx, display).then_some(display)
    };
    let Some(element) = found else { return Ok(Value::NIL) };
    let values = cx.pair(element).expect("a specification is a cons").1;
    let (value, more) = cx.pair(values).expect("a specification has a value");
    Ok(if more == Value::NIL { value } else { values })
}
