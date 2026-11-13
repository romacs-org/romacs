//! The reverse keymap lookup's primitives (keymap.c); the walk is
//! `src/where_is.rs`.
use super::{Fault, Kind, Leaf, Next, Step, Value};
use crate::where_is::{Chain, Keyelt, Query, Reached, Stop, Wrappers};

// Fwhere_is_internal.  A key sequence found is looked up again in the
// keymaps, which applies menu filters as `lookup-key' does.  SAVED is then
// the query as it stands (`Query::values'), each (item . result) the
// lookup under way has, and the item whose filter runs.
defsubr! { name = "where-is-internal", gnu = "keymap.c:2588", effect = Global,
fn where_is_internal(cx, definition: Value; optional keymap: Value, firstonly: Value, noindirect: Value, no_remap: Value) -> Step {
    let query = Query::new(cx, definition, keymap, firstonly, noindirect != Value::NIL, no_remap != Value::NIL)?;
    where_is_step(cx, query, Vec::new())
}}
fn where_is_step(cx: &mut Leaf<'_>, mut query: Query, mut filters: Vec<(Value, Value)>) -> Next {
    match query.advance(cx, &mut filters) {
        Ok(found) => Ok(Step::Done(found)),
        Err(Stop::Fault(fault)) => Err(fault),
        Err(Stop::Filter { item, filter, object }) => {
            let mut saved = query.values()?.to_vec();
            saved.extend(filters.into_iter().flat_map(|(item, result)| [item, result]));
            saved.push(item);
            Ok(super::keymap::filter_call(cx, filter, object, where_is_filtered, saved))
        }
    }
}
fn where_is_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let (query, filters) = saved.split_at(Query::VALUES);
    let mut filters: Vec<(Value, Value)> = filters.chunks_exact(2).map(|pair| (pair[0], pair[1])).collect();
    filters.push((saved[saved.len() - 1], result));
    where_is_step(cx, Query::from_values(query), filters)
}

// Faccessible_keymaps.  PREFIX's keymap comes from `lookup-key', the one
// step that may apply a menu filter.  SAVED is that walk's: KEYMAP, PREFIX
// and t, then the filters' results.
defsubr! { name = "accessible-keymaps", gnu = "keymap.c:2010", effect = Global,
fn accessible_keymaps(cx, keymap: Value; optional prefix: Value) -> Step {
    accessible_step(cx, &[keymap, prefix, Value::TRUE])
}}
fn accessible_step(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let (keymap, prefix) = (saved[0], saved[1]);
    let prefix_length = sequence_length(cx, prefix)?;
    let start = if prefix == Value::NIL {
        let map = crate::where_is::keymap_noload(cx, keymap).filter(|map| cx.pair(*map).is_some()).ok_or(Fault::Wrong("keymapp", keymap))?;
        Some(Reached { keys: Vec::new(), map })
    } else {
        let binding = match super::keymap::walk(cx, saved, 0, accessible_filtered)? {
            Ok(binding) => binding,
            Err(call) => return Ok(call),
        };
        match crate::where_is::keymap_noload(cx, binding) {
            Some(map) => Some(Reached { keys: crate::where_is::prefix_events(cx, prefix)?, map }),
            None => None,
        }
    };
    let reached = crate::where_is::accessible(cx, start, prefix_length)?;
    let mut maps = Value::NIL;
    for (at, Reached { keys, map }) in reached.into_iter().enumerate().rev() {
        // A vector PREFIX is itself the first key sequence.
        let keys = if at == 0 && cx.kind(prefix) == Kind::Vector { prefix } else { cx.vector_from(keys) };
        let entry = cx.cons(keys, map);
        maps = cx.cons(entry, maps);
    }
    Ok(Step::Done(maps))
}
fn accessible_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    saved.push(result);
    accessible_step(cx, &saved)
}
// Flength of a key sequence argument.
fn sequence_length(cx: &Leaf<'_>, sequence: Value) -> std::result::Result<usize, Fault> {
    if sequence == Value::NIL { return Ok(0); }
    match cx.kind(sequence) {
        Kind::Vector => cx.vector_len(sequence),
        Kind::String | Kind::Bytes => Ok(cx.string_chars(sequence).unwrap_or(0)),
        Kind::Cons => {
            let (mut tail, mut length) = (sequence, 0);
            while let Some((_, rest)) = cx.pair(tail) {
                if length > cx.list_limit() { return Err(Fault::Circular(sequence)); }
                (tail, length) = (rest, length + 1);
            }
            if tail == Value::NIL { Ok(length) } else { Err(Fault::Wrong("listp", tail)) }
        }
        _ => Err(Fault::Wrong("sequencep", sequence)),
    }
}

// Fkeymap__get_keyelt.  SAVED is AUTOLOAD and the cycle detection's state
// (`Chain'), kept while a menu item's `:filter' runs.
defsubr! { name = "keymap--get-keyelt", gnu = "keymap.c:646", effect = Global,
fn keymap_get_keyelt(cx, object: Value, autoload: Value) -> Step {
    keyelt_step(cx, object, autoload, Chain::new(object))
}}
fn keyelt_step(cx: &mut Leaf<'_>, object: Value, autoload: Value, chain: Chain) -> Next {
    Ok(match crate::where_is::keyelt_from(cx, Wrappers::new(cx), object, autoload != Value::NIL, chain)? {
        Keyelt::Is(definition) => Step::Done(definition),
        Keyelt::Filter { filter, object, chain } => {
            let [mark, taken, limit] = chain.values()?;
            super::keymap::filter_call(cx, filter, object, keyelt_filtered, vec![autoload, mark, taken, limit])
        }
    })
}
fn keyelt_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let mut chain = Chain::from_values(&saved[1..]);
    chain.advance(result)?;
    keyelt_step(cx, result, saved[0], chain)
}
