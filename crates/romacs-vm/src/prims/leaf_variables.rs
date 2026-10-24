//! GNU eval.c declaration state, independently of the variable's value.
use super::*;
defsubr! { name = "internal-make-var-non-special", gnu = "eval.c:1065", effect = Global,
fn make_var_non_special(cx, symbol: Value) {
    cx.make_variable_non_special(symbol)?;
    Ok(Value::NIL)
}}
