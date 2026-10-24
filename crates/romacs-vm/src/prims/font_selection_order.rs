//! GNU xfaces.c font-sort state; scoring limitations live in DIVERGENCES.md.
use super::*;
use romacs_layout_input::FontSelectionAttribute as A;

defsubr! { name = "internal-set-font-selection-order", gnu = "xfaces.c:5850", effect = Global,
fn internal_set_font_selection_order(cx, order: Value) {
    if order != Value::NIL && cx.pair(order).is_none() {
        return Err(Fault::Wrong("listp", order));
    }
    let invalid = |cx: &mut Leaf<'_>| {
        let condition = cx.intern("error");
        let text = cx.string("Invalid font sort order");
        // GNU eval.c:2110 signal_error splices a proper list but wraps
        // dotted/circular data. Determine properness without looping on cycles.
        let mut slow = order;
        let mut fast = order;
        let proper = loop {
            if fast == Value::NIL { break true; }
            let Some((_, rest)) = cx.pair(fast) else { break false; };
            fast = rest;
            if fast == Value::NIL { break true; }
            let Some((_, rest)) = cx.pair(fast) else { break false; };
            fast = rest;
            slow = cx.pair(slow).expect("fast cursor passed slow").1;
            if slow == fast { break false; }
        };
        let rest = if proper { order } else { cx.cons(order, Value::NIL) };
        let data = cx.cons(text, rest);
        Fault::Signal(condition, data)
    };
    let mut cursor = order;
    let mut parsed = [A::Width; 4];
    for item in &mut parsed {
        let Some((attr, rest)) = cx.pair(cursor) else { return Err(invalid(cx)); };
        *item = if attr == cx.intern(":width") { A::Width }
            else if attr == cx.intern(":height") { A::Height }
            else if attr == cx.intern(":weight") { A::Weight }
            else if attr == cx.intern(":slant") { A::Slant }
            else { return Err(invalid(cx)); };
        cursor = rest;
    }
    if cursor != Value::NIL { return Err(invalid(cx)); }
    cx.set_font_selection_order(parsed);
    Ok(Value::NIL)
}}
