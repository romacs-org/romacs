//! GNU editfns.c point predicates. Accessible bounds do not rebase point.
use super::*;

defsubr! { name = "bolp", gnu = "editfns.c:1019", effect = Reads,
fn bolp(cx) {
    let (beginning, _, previous_newline, _) = cx.point_boundaries()?;
    Ok(if beginning || previous_newline { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "eolp", gnu = "editfns.c:1028", effect = Reads,
fn eolp(cx) {
    let (_, end, _, next_newline) = cx.point_boundaries()?;
    Ok(if end || next_newline { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "bobp", gnu = "editfns.c:999", effect = Reads,
fn bobp(cx) {
    let (beginning, _, _, _) = cx.point_boundaries()?;
    Ok(if beginning { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "eobp", gnu = "editfns.c:1009", effect = Reads,
fn eobp(cx) {
    let (_, end, _, _) = cx.point_boundaries()?;
    Ok(if end { Value::TRUE } else { Value::NIL })
}}
