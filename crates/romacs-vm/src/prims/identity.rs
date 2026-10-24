use super::*;

// GNU fns.c:49: return the argument itself, without copying or coercion.
defsubr! { name = "identity", gnu = "fns.c:49", effect = Pure,
fn identity(_cx, argument: Value) {
    Ok(argument)
}}
