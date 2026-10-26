//! Dense keymap constructor and its character-table type boundary.
use super::*;

defsubr! { name = "romacs--keymap-at-point", gnu = "intervals.c:2176", effect = Reads,
fn keymap_at_point(cx, property: Value) { cx.keymap_at_point(property) }}

defsubr! { name = "current-local-map", gnu = "keymap.c:1910", effect = Reads,
fn current_local_map(cx) {
    cx.current_local_map()
}}

defsubr! { name = "make-keymap", gnu = "keymap.c:93", effect = Global,
fn make_keymap(cx; optional prompt: Value) {
    Ok(cx.dense_keymap(prompt))
}}

defsubr! { name = "char-table-p", gnu = "data.c:435", effect = Pure,
fn char_table_p(cx, object: Value) {
    Ok(if cx.kind(object) == crate::prim::leaf::Kind::CharTable { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "char-table-subtype", gnu = "chartab.c:499", effect = Pure,
fn char_table_subtype(cx, table: Value) {
    cx.char_table_subtype(table)
}}

/// GNU `keymap_parent` (keymap.c:235) after `get_keymap (KEYMAP, 1, ...)`:
/// the first tail of KEYMAP past its `keymap' head that is itself a keymap.
fn keymap_parent(cx: &Leaf<'_>, keymap: Value) -> std::result::Result<Value, Fault> {
    let keymap = cx.keymap(keymap).ok_or(Fault::Wrong("keymapp", keymap))?;
    let mut list = cx.pair(keymap).map_or(Value::NIL, |(_, tail)| tail);
    // A cyclic keymap hangs GNU here; VM2 stops at the list limit.
    for _ in 0..=cx.list_limit() {
        let Some((_, tail)) = cx.pair(list) else { break };
        if cx.keymap(list).is_some() {
            return Ok(list);
        }
        list = tail;
    }
    Ok(cx.keymap(list).unwrap_or(Value::NIL))
}

defsubr! { name = "keymap-parent", gnu = "keymap.c:253", effect = Global,
fn keymap_parent_subr(cx, keymap: Value) {
    keymap_parent(cx, keymap)
}}

// GNU keymap.c:273. Both arguments go through get_keymap (KEYMAP must be a
// keymap, PARENT nil or a keymap; a symbol stands for its function's keymap),
// a parent that already inherits from KEYMAP is refused, and the parent is
// spliced in at the first tail that is nil, an atom or an existing parent.
defsubr! { name = "set-keymap-parent", gnu = "keymap.c:273", effect = Global,
fn set_keymap_parent(cx, keymap: Value, parent: Value) {
    let keymap = cx.keymap(keymap).ok_or(Fault::Wrong("keymapp", keymap))?;
    let parent = if parent == Value::NIL {
        Value::NIL
    } else {
        let parent = cx.keymap(parent).ok_or(Fault::Wrong("keymapp", parent))?;
        // keymap_memberp (keymap.c:262): is KEYMAP PARENT or an ancestor?
        let mut maps = parent;
        for _ in 0..=cx.list_limit() {
            if maps == keymap || cx.keymap(maps).is_none() { break; }
            maps = keymap_parent(cx, maps)?;
        }
        if maps == keymap {
            return Err(Fault::Error("Cyclic keymap inheritance".into()));
        }
        parent
    };
    let mut prev = keymap;
    for _ in 0..=cx.list_limit() {
        let list = cx.pair(prev).map_or(Value::NIL, |(_, tail)| tail);
        if cx.pair(list).is_none() || cx.keymap(list).is_some() {
            cx.setcdr(prev, parent)?;
            return Ok(parent);
        }
        prev = list;
    }
    Err(Fault::Circular(keymap))
}}

defsubr! { name = "copy-keymap", gnu = "keymap.c:1018", effect = Global,
fn copy_keymap(cx, keymap: Value) { cx.copy_keymap(keymap, 0) }}

// Fcommand_remapping: the binding of [remap COMMAND], through key-binding
// (NO-REMAP t, POSITION) or, given KEYMAPS, lookup-key.
defsubr! { name = "command-remapping", gnu = "keymap.c:1216", effect = Reads,
fn command_remapping(cx, command: Value; optional position: Value, keymaps: Value) -> Step {
    if !(command == Value::NIL || command == Value::TRUE || cx.kind(command) == Kind::Symbol) {
        return Ok(Step::Done(Value::NIL));
    }
    let remap = cx.intern("remap");
    let key = cx.vector(&[remap, command]);
    let (f, args) = if keymaps == Value::NIL {
        (cx.intern("key-binding"), vec![key, Value::NIL, Value::TRUE, position])
    } else {
        (cx.intern("lookup-key"), vec![keymaps, key, Value::NIL])
    };
    Ok(Step::Call { f, args, then: remapping_found, saved: vec![] })
}}
fn remapping_found(_: &mut Leaf<'_>, binding: Value, _: &[Value]) -> Next {
    Ok(Step::Done(if binding.integer_value().is_some() { Value::NIL } else { binding }))
}
