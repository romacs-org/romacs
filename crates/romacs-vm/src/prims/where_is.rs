//! The reverse keymap lookup's primitives (keymap.c); the walk is
//! `src/where_is.rs`.
use super::{Fault, Kind, Leaf, Next, Step, Value};
use crate::keymap::Walked;
use crate::where_is::{Chain, Keyelt, Query, Reached, Stop, Wrappers};

// Fwhere_is_internal.  A key sequence found is looked up again in the
// keymaps, which applies menu filters as `lookup-key' does: SAVED is then
// the query as it stands (`Query::values'), then the walk that met the
// filter.
defsubr! { name = "where-is-internal", gnu = "keymap.c:2588", effect = Global,
fn where_is_internal(cx, definition: Value; optional keymap: Value, firstonly: Value, noindirect: Value, no_remap: Value) -> Step {
    let query = Query::new(cx, definition, keymap, firstonly, noindirect != Value::NIL, no_remap != Value::NIL)?;
    where_is_step(cx, query)
}}
fn where_is_step(cx: &mut Leaf<'_>, mut query: Query) -> Next {
    match query.advance(cx) {
        Ok(found) => Ok(Step::Done(found)),
        Err(Stop::Fault(fault)) => Err(fault),
        Err(Stop::Filter { walk, filter, object }) => Ok(where_is_filter(cx, &query.values()?, &walk, filter, object)),
    }
}
fn where_is_filter(cx: &mut Leaf<'_>, query: &[Value], walk: &crate::keymap::KeyWalk, filter: Value, object: Value) -> Step {
    let mut saved = query.to_vec();
    saved.extend(cx.key_walk_values(walk));
    super::keymap::filter_call(cx, filter, object, where_is_filtered, saved)
}
// The walk under way goes on with the filter's RESULT; once it has its
// answer, so does the query, from the lookup the walk was for.
fn where_is_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let (query, walk) = saved.split_at(Query::VALUES);
    let mut walk = cx.key_walk_from(walk)?;
    match cx.key_walk_resume(&mut walk, result)? {
        Walked::Filter { filter, object } => Ok(where_is_filter(cx, query, &walk, filter, object)),
        Walked::Done(binding) => {
            let mut query = Query::from_values(query);
            match query.looked_up(cx, binding)? {
                Some(answer) => Ok(Step::Done(answer)),
                None => where_is_step(cx, query),
            }
        }
    }
}

// Faccessible_keymaps.  PREFIX's keymap comes from `lookup-key', the one
// step that may apply a menu filter: SAVED is KEYMAP and PREFIX, then that
// walk's.
defsubr! { name = "accessible-keymaps", gnu = "keymap.c:2010", effect = Global,
fn accessible_keymaps(cx, keymap: Value; optional prefix: Value) -> Step {
    sequence_length(cx, prefix)?;
    if prefix == Value::NIL {
        let map = crate::where_is::keymap_noload(cx, keymap).filter(|map| cx.pair(*map).is_some()).ok_or_else(|| Fault::Wrong("keymapp", keymap))?;
        return accessible_from(cx, prefix, Some(Reached { keys: Vec::new(), map }));
    }
    let walked = super::keymap::lookup(cx, &[keymap, prefix], keymap, prefix, true, accessible_filtered)?;
    accessible_prefix(cx, prefix, walked)
}}
fn accessible_filtered(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let walked = super::keymap::resume(cx, result, saved, 2, accessible_filtered)?;
    accessible_prefix(cx, saved[1], walked)
}
// The keymaps reachable from PREFIX's binding, once the walk has it.
fn accessible_prefix(cx: &mut Leaf<'_>, prefix: Value, walked: super::keymap::Found) -> Next {
    let binding = match walked {
        Ok(binding) => binding,
        Err(call) => return Ok(call),
    };
    let start = match crate::where_is::keymap_noload(cx, binding) {
        Some(map) => Some(Reached { keys: crate::where_is::prefix_events(cx, prefix)?, map }),
        None => None,
    };
    accessible_from(cx, prefix, start)
}
// The keymaps reachable from START, each with its key sequence, as a list.
fn accessible_from(cx: &mut Leaf<'_>, prefix: Value, start: Option<Reached>) -> Next {
    let prefix_length = sequence_length(cx, prefix)?;
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
