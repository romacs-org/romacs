//! GNU function-cell removal, distinct from variable-cell unbinding.
use super::*;

defsubr! { name = "fmakunbound", gnu = "data.c:792", effect = Global,
fn fmakunbound(cx, symbol: Value) {
    if symbol == Value::NIL || symbol == Value::TRUE {
        return Err(Fault::SettingConstant(symbol));
    }
    cx.set_symbol_function(symbol, Value::NIL)?;
    Ok(symbol)
}}
