//! GNU eval.c variable declarations without initializing the value cell.
//! The execution regime is compiled, matching GNU bytecode. GNU's lexbound_p
//! (eval.c:784-808) inspects saved interpreter environments only; VM2's `eval`
//! also compiles its input, so its interpreter-only lexical-conflict signal
//! is not modeled here.
use super::*;

defsubr! { name = "internal--define-uninitialized-variable", gnu = "eval.c:889", effect = Global,
fn define_uninitialized_variable(cx, symbol: Value; optional doc: Value) {
    // GNU assumes a symbol (XSYMBOL, not CHECK_SYMBOL); reject malformed
    // callers safely rather than reproducing its undefined memory access.
    cx.declare_variable_special(symbol)?;
    if doc != Value::NIL {
        cx.put_variable_documentation(symbol, doc)?;
    }
    cx.attach_variable_history(symbol)?;
    Ok(Value::NIL)
}}

defsubr! { name = "defvar-1", gnu = "eval.c:1010", effect = Global,
fn defvar_1(cx, symbol: Value, init: Value; optional doc: Value) {
    let bound = cx.default_cell(symbol);
    cx.declare_variable_special(symbol)?;
    if doc != Value::NIL { cx.put_variable_documentation(symbol, doc)?; }
    cx.attach_variable_history(symbol)?;
    match bound {
        Err(Fault::VoidVariable(_)) => { cx.default_cell_set(symbol, init)?; }
        Err(error) => return Err(error),
        Ok(_) => match cx.saved_default_cell(symbol) {
            Err(Fault::VoidVariable(_)) => { cx.saved_default_cell_set(symbol, init)?; }
            Err(error) => return Err(error),
            Ok(_) => {},
        },
    }
    Ok(symbol)
}}

// The compiler still classifies bindings by interned name. An uninterned
// declaration retains its object-owned flag (including across code images),
// but later uninterned lexical binding forms do not yet consume that flag.
