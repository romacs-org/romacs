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
    cx.note_keymap_change();
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

// A `lookup-key' walk that may stop for a menu item's `:filter', which runs
// as get_keyelt runs it (menu_item_eval_property: errors give nil), through
// the Lisp helper `romacs--menu-filter'.  SAVED is [KEYMAP KEY ACCEPT-DEFAULT],
// FIXED values of the caller's, then each (item . result) the filters gave:
// each result joins SAVED and the walk restarts, so the pairs stay rooted
// across the call.  Err is the call to make, THEN its continuation.
fn walk(cx: &mut Leaf<'_>, saved: &[Value], fixed: usize, then: crate::prim::Then) -> std::result::Result<std::result::Result<Value, Step>, Fault> {
    let filters = saved[3 + fixed..].chunks_exact(2).map(|pair| (pair[0], pair[1])).collect();
    match cx.lookup_key_walk(saved[0], saved[1], saved[2] != Value::NIL, filters)? {
        Ok(binding) => Ok(Ok(binding)),
        Err((item, filter, object)) => {
            let helper = cx.intern("romacs--menu-filter");
            let (f, args) = if cx.fboundp(helper) { (helper, vec![filter, object]) } else { (filter, vec![object]) };
            let mut saved = saved.to_vec();
            saved.push(item);
            Ok(Err(Step::Call { f, args, then, saved }))
        }
    }
}
// SAVED with a filter's RESULT, for STEP to walk again.
fn filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value], step: fn(&mut Leaf<'_>, &[Value]) -> Next) -> Next {
    let mut saved = saved.to_vec();
    saved.push(result);
    step(cx, &saved)
}
fn symbolp(cx: &Leaf<'_>, object: Value) -> bool {
    object == Value::NIL || object == Value::TRUE || cx.kind(object) == Kind::Symbol
}
fn unbound(binding: Value) -> bool {
    binding == Value::NIL || binding.integer_value().is_some()
}

// Fcommand_remapping: the binding of [remap COMMAND] in the active keymaps
// at POSITION, or in KEYMAPS.
defsubr! { name = "command-remapping", gnu = "keymap.c:1216", effect = Reads,
fn command_remapping(cx, command: Value; optional position: Value, keymaps: Value) -> Step {
    if !symbolp(cx, command) { return Ok(Step::Done(Value::NIL)); }
    let maps = if keymaps == Value::NIL { cx.current_active_maps(true, position)? } else { keymaps };
    remapping(cx, maps, command, Value::NIL)
}}
// The binding of [remap COMMAND] in MAPS, or UNMAPPED.
fn remapping(cx: &mut Leaf<'_>, maps: Value, command: Value, unmapped: Value) -> Next {
    let remap = cx.intern("remap");
    let key = cx.vector(&[remap, command]);
    remapping_step(cx, &[maps, key, Value::NIL, unmapped])
}
fn remapping_step(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    Ok(match walk(cx, saved, 1, remapping_filtered)? {
        Ok(binding) => Step::Done(if unbound(binding) { saved[3] } else { binding }),
        Err(call) => call,
    })
}
fn remapping_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    filtered(cx, result, saved, remapping_step)
}

// Fcurrent_active_maps: the policy is src/active_keymaps.rs.
defsubr! { name = "current-active-maps", gnu = "keymap.c:1649", effect = Reads,
fn current_active_maps(cx; optional olp: Value, position: Value) {
    cx.current_active_maps(olp != Value::NIL, position)
}}

defsubr! { name = "current-minor-mode-maps", gnu = "keymap.c:1928", effect = Reads,
fn current_minor_mode_maps(cx) {
    cx.current_minor_mode_maps(false)
}}

// `current-minor-mode-maps' with each mode: ((MODE . KEYMAP) ...), for a
// listing that names the modes.
defsubr! { name = "romacs--minor-mode-maps", gnu = "keymap.c:1531", effect = Reads,
fn romacs_minor_mode_maps(cx) {
    cx.current_minor_mode_maps(true)
}}

// Fkey_binding.  With no POSITION, a key that starts with a mouse click
// (past a symbolic prefix such as `mode-line') is looked up where it
// clicked.  A command found is remapped as Fcommand_remapping would, in the
// same active keymaps unless a menu filter ran Lisp since they were read.
// SAVED is the walk's, then NO-REMAP and POSITION.
defsubr! { name = "key-binding", gnu = "keymap.c:1788", effect = Global,
fn key_binding(cx, key: Value; optional accept_default: Value, no_remap: Value, position: Value) -> Step {
    let position = if position == Value::NIL && cx.kind(key) == Kind::Vector {
        let events = cx.vector_values(key)?;
        let event = match events.as_slice() {
            [] => return Ok(Step::Done(Value::NIL)),
            [prefix, event, ..] if symbolp(cx, *prefix) => *event,
            [event, ..] => *event,
        };
        click_start(cx, event)?
    } else { position };
    let maps = cx.current_active_maps(true, position)?;
    key_binding_step(cx, &[maps, key, accept_default, no_remap, position])
}}
// EVENT_START of EVENT if it is a mouse click with a position, else nil.
fn click_start(cx: &mut Leaf<'_>, event: Value) -> Out {
    let Some((head, Some((start, _)))) = cx.pair(event).map(|(head, rest)| (head, cx.pair(rest))) else { return Ok(Value::NIL) };
    let kind = cx.intern("event-kind");
    Ok(if cx.symbol_property(head, kind)? == cx.intern("mouse-click") { start } else { Value::NIL })
}
fn key_binding_step(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let binding = match walk(cx, saved, 2, key_binding_filtered)? {
        Ok(binding) => binding,
        Err(call) => return Ok(call),
    };
    if unbound(binding) { return Ok(Step::Done(Value::NIL)); }
    if saved[3] != Value::NIL || !symbolp(cx, binding) { return Ok(Step::Done(binding)); }
    let maps = if saved.len() == 5 { saved[0] } else { cx.current_active_maps(true, saved[4])? };
    remapping(cx, maps, binding, binding)
}
fn key_binding_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    filtered(cx, result, saved, key_binding_step)
}

// Fminor_mode_key_binding: KEY's binding in each active minor mode's map,
// highest priority first: the prefix bindings, or the first binding alone
// when it is no prefix; a non-prefix after a prefix is ignored.  SAVED is
// the walk's (the map in hand first), then the (MODE . KEYMAP) still to
// look in, the map's mode and the prefixes found, newest first.
defsubr! { name = "minor-mode-key-binding", gnu = "keymap.c:1857", effect = Global,
fn minor_mode_key_binding(cx, key: Value; optional accept_default: Value) -> Step {
    let rest = cx.current_minor_mode_maps(true)?;
    let mut saved = vec![Value::NIL, key, accept_default, rest, Value::NIL, Value::NIL];
    if !next_minor_map(cx, &mut saved) { return Ok(Step::Done(Value::NIL)); }
    minor_binding_step(cx, &saved)
}}
// Take the next minor mode's map in hand, if one is left.
fn next_minor_map(cx: &Leaf<'_>, saved: &mut Vec<Value>) -> bool {
    let Some(((mode, map), rest)) = cx.pair(saved[3]).and_then(|(entry, rest)| Some((cx.pair(entry)?, rest))) else { return false };
    saved.truncate(6);
    (saved[0], saved[3], saved[4]) = (map, rest, mode);
    true
}
fn minor_binding_step(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    loop {
        let binding = match walk(cx, &saved, 3, minor_binding_filtered)? {
            Ok(binding) => binding,
            Err(call) => return Ok(call),
        };
        if !unbound(binding) {
            let entry = cx.cons(saved[4], binding);
            if cx.keymapp(binding) {
                saved[5] = cx.cons(entry, saved[5]);
            } else if saved[5] == Value::NIL {
                return Ok(Step::Done(cx.cons(entry, Value::NIL)));
            }
        }
        if !next_minor_map(cx, &mut saved) { break; }
    }
    let (mut prefixes, mut found) = (saved[5], Value::NIL);
    while let Some((entry, rest)) = cx.pair(prefixes) {
        found = cx.cons(entry, found);
        prefixes = rest;
    }
    Ok(Step::Done(found))
}
fn minor_binding_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    filtered(cx, result, saved, minor_binding_step)
}

// Fkeymap_prompt: the first string in MAP's list, searching embedded
// keymaps (not parents' autoloads) in order.
defsubr! { name = "keymap-prompt", gnu = "keymap.c:148", effect = Reads,
fn keymap_prompt(cx, map: Value) {
    keymap_prompt_in(cx, map, 0)
}}
fn keymap_prompt_in(cx: &mut Leaf<'_>, map: Value, depth: usize) -> Out {
    let Some(mut map) = cx.keymap(map) else { return Ok(Value::NIL) };
    for _ in 0..=cx.list_limit() {
        let Some((item, rest)) = cx.pair(map) else { return Ok(Value::NIL) };
        if cx.string_view(item).is_ok() { return Ok(item); }
        if depth < cx.list_limit() && cx.keymap(item).is_some() {
            let prompt = keymap_prompt_in(cx, item, depth + 1)?;
            if prompt != Value::NIL { return Ok(prompt); }
        }
        map = rest;
    }
    Err(Fault::Circular(map))
}

// Flookup_key.
defsubr! { name = "lookup-key", gnu = "keymap.c:1290", effect = Global,
fn lookup_key(cx, keymap: Value, key: Value; optional accept_default: Value) -> Step {
    lookup_key_step(cx, &[keymap, key, accept_default])
}}
fn lookup_key_step(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    Ok(match walk(cx, saved, 0, lookup_key_filtered)? {
        Ok(binding) => Step::Done(binding),
        Err(call) => call,
    })
}
fn lookup_key_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    filtered(cx, result, saved, lookup_key_step)
}

// Frecent_keys over the events the command loop recorded; a batch session
// reads no input, so it is [] as in GNU.
defsubr! { name = "recent-keys", gnu = "keyboard.c:12213", effect = Reads,
fn recent_keys(cx; optional include_cmds: Value) {
    Ok(cx.recent_keys(include_cmds != Value::NIL))
}}

// Flossage_size: the number of recent keys kept, or with ARG a new number
// from 100 (MIN_NUM_RECENT_KEYS) to MAX_NUM_RECENT_KEYS, keeping the
// newest; anything else is a `user-error' with GNU's message.
defsubr! { name = "lossage-size", gnu = "keyboard.c:12173", effect = Global,
fn lossage_size(cx; optional arg: Value) {
    const MIN: i64 = 100;
    const MAX: i64 = i32::MAX as i64 / 64;
    if arg != Value::NIL {
        let message = match arg.integer_value() {
            Some(n) if n < 0 => Some("Value must be a positive integer".to_owned()),
            None => Some("Value must be a positive integer".to_owned()),
            Some(n) if n as usize == cx.lossage_limit() => None,
            Some(n) if n < MIN => Some(format!("Value must be >= {MIN}")),
            Some(n) if n > MAX => Some(format!("Value must be <= {MAX}")),
            Some(n) => { cx.set_lossage_limit(n as usize); None }
        };
        if let Some(message) = message {
            let message = cx.string(&message);
            let data = cx.cons(message, Value::NIL);
            return Err(Fault::Signal(cx.intern("user-error"), data));
        }
    }
    Value::integer(cx.lossage_limit() as i64).map_err(Fault::Error)
}}

// The count of keymap changes, which keymap.c keeps implicitly as the
// validity of its where_is_cache: `where-is-internal' (rlisp/vm/prelude.el)
// rebuilds its reverse map when this moves.
defsubr! { name = "romacs--keymap-generation", gnu = "keymap.c:2505", effect = Reads,
fn romacs_keymap_generation(cx) {
    Value::integer((cx.keymap_generation() & 0x0fff_ffff_ffff_ffff) as i64).map_err(Fault::Error)
}}
