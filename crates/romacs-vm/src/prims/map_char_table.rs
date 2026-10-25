use super::*;

fn resume(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    match cx.map_cursor_next(saved[1])? {
        crate::map_keymap::Advance::Binding(key, value) => Ok(Step::Call {
            f: saved[0],
            args: vec![key, value],
            then: resume,
            saved: saved.to_vec(),
        }),
        crate::map_keymap::Advance::Done(_) => Ok(Step::Done(Value::NIL)),
        crate::map_keymap::Advance::Load(..) => unreachable!("character tables do not autoload"),
    }
}

defsubr! { name = "map-char-table", gnu = "chartab.c:903", effect = Global,
fn map_char_table(cx, function: Value, table: Value) -> Step {
    let cursor = cx.char_table_cursor(table)?;
    resume(cx, Value::NIL, &[function, cursor])
}}
