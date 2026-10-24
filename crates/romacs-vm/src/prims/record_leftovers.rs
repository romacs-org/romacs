//! Small GNU-native array/list operations required by record witnesses.
use super::*;

defsubr! { name = "make-list", gnu = "alloc.c:2743", effect = Pure,
fn make_list(cx, length: Value, init: Value) {
    let count = length.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or(Fault::Wrong("wholenump", length))?;
    // Leaf allocation does not collect or re-enter Lisp; the caller's arguments
    // and the completed spine remain rooted across the next VM safepoint.
    let mut list = Value::NIL;
    for _ in 0..count {
        list = cx.cons(init, list);
    }
    Ok(list)
}}
