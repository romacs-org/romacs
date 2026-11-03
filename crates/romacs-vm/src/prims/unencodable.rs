//! GNU coding.c unencodable-char-position.
use super::*;

// Fcheck_coding_system first defines a coding system declared lazily.
defsubr! { name = "unencodable-char-position", gnu = "coding.c:9209", effect = Global,
fn unencodable_char_position(cx, start: Value, end: Value, coding: Value; optional count: Value, string: Value) -> Step {
    let saved = vec![start, end, coding, count, string];
    if coding != Value::NIL && cx.coding_spec(coding)? == Value::NIL {
        return super::coding::realize(cx, coding, saved, checked);
    }
    checked(cx, Value::NIL, &saved)
}}

fn checked(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(cx.unencodable_char_position(saved[0], saved[1], saved[2], saved[3], saved[4])?))
}
