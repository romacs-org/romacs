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

fn watchers(cx: &mut Leaf<'_>, symbol: Value) -> std::result::Result<Vec<Value>, Fault> {
    let property = cx.intern("watchers");
    let mut list = Vec::new();
    let mut tail = cx.face_property(symbol, property)?;
    while let Some((watcher, rest)) = cx.pair(tail) { list.push(watcher); tail = rest; }
    Ok(list)
}

fn store(cx: &mut Leaf<'_>, symbol: Value, list: &[Value]) -> std::result::Result<(), Fault> {
    let property = cx.intern("watchers");
    let value = list.iter().rev().fold(Value::NIL, |tail, &w| cx.cons(w, tail));
    cx.put_symbol_property(symbol, property, value)?;
    cx.trap_variable(symbol, !list.is_empty())
}

defsubr! { name = "add-variable-watcher", gnu = "data.c:1857", effect = Global,
fn add_variable_watcher(cx, symbol: Value, watch_function: Value) {
    let symbol = base_variable(cx, symbol)?;
    let mut list = watchers(cx, symbol)?;
    let mut present = false;
    for &w in &list { if cx.equal(w, watch_function).map_err(|_| Fault::Circular(w))? { present = true; } }
    if !present { list.insert(0, watch_function); }
    store(cx, symbol, &list)?;
    Ok(Value::NIL)
}}

defsubr! { name = "remove-variable-watcher", gnu = "data.c:1885", effect = Global,
fn remove_variable_watcher(cx, symbol: Value, watch_function: Value) {
    let symbol = base_variable(cx, symbol)?;
    let mut list = Vec::new();
    for w in watchers(cx, symbol)? {
        if !cx.equal(w, watch_function).map_err(|_| Fault::Circular(w))? { list.push(w); }
    }
    store(cx, symbol, &list)?;
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
