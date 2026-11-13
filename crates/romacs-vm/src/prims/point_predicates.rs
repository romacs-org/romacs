//! GNU editfns.c point predicates. Accessible bounds do not rebase point.
use super::*;

defsubr! { name = "following-char", gnu = "editfns.c:967", effect = Reads,
fn following_char(cx) { cx.following_character() }}

defsubr! { name = "bolp", gnu = "editfns.c:1019", effect = Reads,
fn bolp(cx) {
    Ok(if cx.point_at_line_start()? { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "eolp", gnu = "editfns.c:1028", effect = Reads,
fn eolp(cx) {
    Ok(if cx.point_at_line_end()? { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "bobp", gnu = "editfns.c:999", effect = Reads,
fn bobp(cx) {
    Ok(if cx.point_at_bounds()?.0 { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "eobp", gnu = "editfns.c:1009", effect = Reads,
fn eobp(cx) {
    Ok(if cx.point_at_bounds()?.1 { Value::TRUE } else { Value::NIL })
}}

// char-before at point, with 0 for nil at the start of the accessible region.
defsubr! { name = "preceding-char", gnu = "editfns.c:975", effect = Reads,
fn preceding_char(cx) {
    let c = cx.character_before(Value::NIL)?;
    Ok(if c == Value::NIL { Value::integer(0).unwrap() } else { c })
}}
