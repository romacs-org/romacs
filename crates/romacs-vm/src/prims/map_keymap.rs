//! GNU keymap.c:498-644; callbacks cross only the T2 traced boundary.
use super::*;
fn resume(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    match cx.map_cursor_next(saved[1])? {
        crate::map_keymap::Advance::Load(definition, symbol) => {
            let f = cx.intern("autoload-do-load");
            Ok(Step::Call {
                f,
                args: vec![definition, symbol, Value::NIL],
                then: resume,
                saved: saved.to_vec(),
            })
        }
        crate::map_keymap::Advance::Binding(key, value) => Ok(Step::Call {
            f: saved[0],
            args: vec![
                key,
                if value == Value::TRUE {
                    Value::NIL
                } else {
                    value
                },
            ],
            then: resume,
            saved: saved.to_vec(),
        }),
        crate::map_keymap::Advance::Done(result) => Ok(Step::Done(result)),
    }
}
fn sorted_done(_: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    Ok(Step::Done(result))
}
defsubr! { name = "map-keymap", gnu = "keymap.c:625", effect = Global,
fn map_keymap(cx, function: Value, keymap: Value; optional sort_first: Value) -> Step {
    if sort_first != Value::NIL {
        let f = cx.intern("map-keymap-sorted");
        return Ok(Step::Call { f, args: vec![function, keymap], then: sorted_done, saved: vec![] });
    }
    let cursor = cx.map_cursor(keymap, false);
    resume(cx, Value::NIL, &[function, cursor])
}}
defsubr! { name = "map-keymap-internal", gnu = "keymap.c:613", effect = Global,
fn map_keymap_internal(cx, function: Value, keymap: Value) -> Step {
    let cursor = cx.map_cursor(keymap, true);
    resume(cx, Value::NIL, &[function, cursor])
}}
