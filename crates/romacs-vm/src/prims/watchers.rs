//! GNU data.c add-variable-watcher, remove-variable-watcher and
//! get-variable-watchers. Watchers are a list on the base variable's
//! `watchers' property; the VM traps writes while it is non-empty and calls
//! them through notify_variable_watchers (watchers.rs).
use super::*;

fn base_variable(cx: &mut Leaf<'_>, symbol: Value) -> Out {
    if cx.kind(symbol) != leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", symbol)); }
    let mut base = symbol;
    let mut seen = Vec::new();
    while let Some(next) = cx.variable_alias(base)? {
        if seen.contains(&next) { break; }
        seen.push(next);
        base = next;
    }
    Ok(base)
}

fn watchers(cx: &mut Leaf<'_>, symbol: Value) -> Out {
    let property = cx.intern("watchers");
    cx.face_property(symbol, property)
}

fn member(cx: &mut Leaf<'_>, watch_function: Value, mut tail: Value) -> std::result::Result<bool, Fault> {
    while let Some((w, rest)) = cx.pair(tail) {
        if cx.equal(w, watch_function).map_err(|_| Fault::Circular(w))? { return Ok(true); }
        tail = rest;
    }
    Ok(false)
}

fn store(cx: &mut Leaf<'_>, symbol: Value, list: Value) -> std::result::Result<(), Fault> {
    let property = cx.intern("watchers");
    cx.put_symbol_property(symbol, property, list)?;
    cx.trap_variable(symbol, list != Value::NIL)
}

defsubr! { name = "add-variable-watcher", gnu = "data.c:1857", effect = Global,
fn add_variable_watcher(cx, symbol: Value, watch_function: Value) {
    let symbol = base_variable(cx, symbol)?;
    let list = watchers(cx, symbol)?;
    let list = if member(cx, watch_function, list)? { list } else { cx.cons(watch_function, list) };
    store(cx, symbol, list)?;
    Ok(Value::NIL)
}}

// Fdelete splices the list in place, so a notification walking it skips a
// watcher removed before it is reached.
defsubr! { name = "remove-variable-watcher", gnu = "data.c:1885", effect = Global,
fn remove_variable_watcher(cx, symbol: Value, watch_function: Value) {
    let symbol = base_variable(cx, symbol)?;
    let mut list = watchers(cx, symbol)?;
    let (mut previous, mut tail) = (Value::NIL, list);
    while let Some((w, rest)) = cx.pair(tail) {
        if !cx.equal(w, watch_function).map_err(|_| Fault::Circular(w))? { previous = tail; }
        else if previous == Value::NIL { list = rest; }
        else { cx.setcdr(previous, rest)?; }
        tail = rest;
    }
    store(cx, symbol, list)?;
    Ok(Value::NIL)
}}

defsubr! { name = "get-variable-watchers", gnu = "data.c:1904", effect = Reads,
fn get_variable_watchers(cx, symbol: Value) {
    let symbol = base_variable(cx, symbol)?;
    if !cx.trapped_variable(symbol)? { return Ok(Value::NIL); }
    let property = cx.intern("watchers");
    cx.face_property(symbol, property)
}}

defsubr! { name = "set-buffer-redisplay", gnu = "xdisp.c:922", effect = Global,
fn set_buffer_redisplay(cx, _symbol: Value, _newval: Value, _op: Value, _place: Value) {
    // bset_redisplay: the next redisplay republishes the current buffer.
    cx.request_redisplay();
    Ok(Value::NIL)
}}
