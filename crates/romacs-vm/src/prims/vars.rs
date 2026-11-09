//! Variable aliasing from GNU `src/eval.c`.
//!
//! `defvaralias` (eval.c:631-726) makes NEW-ALIAS share BASE-VARIABLE's value
//! cell: every read, write, `let` binding and buffer-local lookup of NEW then
//! acts on the variable at the end of the alias chain (`Vm::indirect_variable`,
//! the counterpart of GNU's `SYMBOL_VARALIAS` redirect in data.c). The
//! redirect holds a stable slot ID, not a heap Value, so it needs no tracing.
//!
//! The checks run in GNU's order, and the order is observable: BASE's value is
//! copied from NEW *before* the let-binding check can still refuse.
use super::*;
use crate::prim::Fault;
use crate::prim::leaf::Kind;

fn symbol(cx: &Leaf<'_>, v: Value) -> std::result::Result<Value, Fault> {
    if v == Value::NIL || v == Value::TRUE || cx.kind(v) == Kind::Symbol {
        Ok(v)
    } else {
        Err(Fault::Wrong("symbolp", v))
    }
}

defsubr! { name = "defvaralias", gnu = "eval.c:631", effect = Global,
fn defvaralias(cx, new_alias: Value, base_variable: Value; optional docstring: Value) {
    let new_alias = symbol(cx, new_alias)?;
    let base_variable = symbol(cx, base_variable)?;
    if cx.variable_constant(new_alias)? {
        // Making it an alias effectively changes its value (eval.c:647).
        return Err(Fault::Error(format!("Cannot make a constant an alias: {}", cx.symbol_name(new_alias)?)));
    }
    // Ensure non-circularity (eval.c:653-661): the walk starts at BASE and
    // the signal names BASE, not the symbol where the cycle closed.
    let mut s = base_variable;
    loop {
        if s == new_alias {
            return Err(Fault::CyclicVariableIndirection(base_variable));
        }
        match cx.variable_alias(s)? {
            Some(next) => s = next,
            None => break,
        }
    }
    // eval.c:666-670, NEW's own redirect (SYMBOL_FORWARDED, then LOCALIZED).
    // GNU `error' formats with format-message, so its ' prints as U+2019.
    if cx.variable_forwarded(new_alias)? { return Err(Fault::Error(format!("Cannot make a built-in variable an alias: {}", cx.symbol_name(new_alias)?))); }
    if cx.variable_localized(new_alias)? {
        return Err(Fault::Error(format!(
            "Don\u{2019}t know how to make a buffer-local variable an alias: {}", cx.symbol_name(new_alias)?)));
    }
    let base_value = cx.variable_value(base_variable)?;
    let new_value = cx.variable_value(new_alias)?;
    match (base_value, new_value) {
        // eval.c:682-685: old code that set NEW before the alias existed
        // keeps working. A void NEW leaves BASE void.
        (None, Some(value)) => cx.variable_set(base_variable, value)?,
        (None, None) => {}
        (Some(base), Some(new)) if !cx.eq(base, new) => cx.batch_warning("defvaralias", &format!(
            "Overwriting value of \u{2018}{}\u{2019} by aliasing to \u{2018}{}\u{2019}",
            cx.symbol_name(new_alias)?, cx.symbol_name(base_variable)?)),
        (Some(_), _) => {}
    }
    if cx.variable_let_bound(new_alias)? {
        return Err(Fault::Error(format!(
            "Don\u{2019}t know how to make a let-bound variable an alias: {}", cx.symbol_name(new_alias)?)));
    }
    cx.make_variable_alias(new_alias, base_variable)?;
    cx.attach_load_history(new_alias)?;
    // Even if docstring is nil: remove old docstring (eval.c:722-723).
    cx.put_variable_documentation(new_alias, docstring)?;
    Ok(base_variable)
}}

// GNU eval.c:2411. Installs `(autoload FILE DOCSTRING INTERACTIVE TYPE)` in
// FUNCTION's cell through defalias's fset (data.c:971-1000), unless FUNCTION
// is already defined other than as an autoload. Calling the object later is
// the evaluator's business, not this primitive's (DIVERGENCES.md
// VM2-AUTOLOAD1: VM2 does not load FILE on call).
defsubr! { name = "autoload", gnu = "eval.c:2411", effect = Global,
fn autoload(cx, function: Value, file: Value; optional docstring: Value, interactive: Value, kind: Value) {
    let function = symbol(cx, function)?;
    cx.string_view(file)?;
    let current = cx.symbol_function(function)?;
    let tag = cx.intern("autoload");
    if current != Value::NIL && cx.pair(current).is_none_or(|(head, _)| head != tag) {
        return Ok(Value::NIL);
    }
    let mut descriptor = Value::NIL;
    for value in [kind, interactive, docstring, file, tag] {
        descriptor = cx.cons(value, descriptor);
    }
    // data.c defalias, which leaves these out of a dump only to save space.
    let defun = cx.intern("defun");
    let entry = cx.cons(defun, function);
    cx.attach_load_history(entry)?;
    cx.defalias_function(function, descriptor)?;
    cx.note_init_declared(file);
    Ok(function)
}}

// GNU data.c:1960 `default_value`: the default binding through aliases, or
// `None` when void. An interned keyword is bound to itself in GNU
// (lread.c intern_sym); VM2 self-evaluates keywords without filling the value
// cell, so an empty keyword cell reads as the keyword. An uninterned `:x` is an
// ordinary symbol in both.
pub(crate) fn default_value(cx: &mut Leaf<'_>, v: Value) -> std::result::Result<Option<Value>, Fault> {
    let v = symbol(cx, v)?;
    match cx.default_cell(v) {
        Ok(value) => Ok(Some(value)),
        Err(Fault::VoidVariable(_)) => {
            // An interned keyword's empty cell (see above).
            interned_keyword(cx, v)
        }
        Err(fault) => Err(fault),
    }
}

// GNU data.c:2006: t iff SYMBOL's default value is not void.
defsubr! { name = "default-boundp", gnu = "data.c:2006", effect = Global,
fn default_boundp(cx, symbol: Value) {
    Ok(if default_value(cx, symbol)?.is_some() { Value::TRUE } else { Value::NIL })
}}

// GNU data.c:2019: SYMBOL's default value, signalling void-variable if void.
defsubr! { name = "default-value", gnu = "data.c:2019", effect = Global,
fn default_value_subr(cx, symbol: Value) {
    default_value(cx, symbol)?.ok_or(Fault::VoidVariable(symbol))
}}

// GNU eval.c:3087 over FUNCTIONP (eval.c:3103): a non-nil fbound symbol is
// replaced by the end of its function chain (indirect_function, data.c:2550;
// a macro's cell reads `(macro . F)` through symbol_definition); an autoload
// reached through a symbol is a function unless its TYPE (fifth element) is
// non-nil. Then: a subr that is not a special form (UNEVALLED), a closure, or
// a cons whose car is `lambda'.
defsubr! { name = "functionp", gnu = "eval.c:3087", effect = Global,
fn functionp(cx, object: Value) {
    Ok(if function_p(cx, object)? { Value::TRUE } else { Value::NIL })
}}

/// GNU `FUNCTIONP` (eval.c:3103), shared by `functionp` and `run-hooks`.
pub(crate) fn function_p(cx: &mut Leaf<'_>, mut object: Value) -> std::result::Result<bool, Fault> {
    let symbolp = |cx: &Leaf<'_>, v: Value| v != Value::NIL && cx.kind(v) == Kind::Symbol;
    if symbolp(cx, object) && cx.symbol_function(object)? != Value::NIL {
        // `defalias'/`fset' refuse cycles; the second cursor only stops a hang.
        let mut slow = object;
        while symbolp(cx, object) {
            object = cx.symbol_definition(object)?;
            if symbolp(cx, object) {
                object = cx.symbol_definition(object)?;
                slow = cx.symbol_function(slow)?;
                if object == slow && symbolp(cx, object) { return Ok(false); }
            }
        }
        if let Some((head, mut rest)) = cx.pair(object) {
            if head == cx.intern("autoload") {
                for _ in 0..3 { match cx.pair(rest) { Some((_, next)) => rest = next, None => break } }
                return Ok(!cx.pair(rest).is_some_and(|(kind, _)| kind != Value::NIL));
            }
        }
    }
    Ok(match cx.kind(object) {
        Kind::Subr | Kind::Closure => crate::prim::name(object).and_then(super::special_forms::special_form_min).is_none(),
        Kind::Cons => cx.pair(object).is_some_and(|(head, _)| head == cx.intern("lambda")),
        _ => false,
    })
}

// GNU data.c:2130 over set_default_internal (data.c:2063). An interned
// keyword is SYMBOL_NOWRITE and may only be "set" to itself, which is a no-op;
// every other constant signals setting-constant. A plain variable's value cell
// is its current binding, a `let' binding included (set_internal); a localized
// variable's default cell is written whatever buffer is current. VM2 holds
// both in the slot at the end of the alias chain (Leaf::default_cell_set).
// Variable watchers are not modelled (DIVERGENCES.md VM2-VARALIAS1).
defsubr! { name = "set-default", gnu = "data.c:2130", effect = Global,
fn set_default(cx, symbol: Value, value: Value) {
    let symbol = self::symbol(cx, symbol)?;
    let name = cx.symbol_name(symbol)?;
    if name.starts_with(':') && cx.obarray_lookup(&name) == Some(symbol) {
        return if value == symbol { Ok(value) } else { Err(Fault::SettingConstant(symbol)) };
    }
    cx.default_cell_set(symbol, value)
}}

/// An empty value cell read as GNU would: SYMBOL itself when it is an
/// interned keyword (see `default_value'), else void.
fn interned_keyword(cx: &mut Leaf<'_>, v: Value) -> std::result::Result<Option<Value>, Fault> {
    let name = cx.symbol_name(v)?;
    Ok((name.starts_with(':') && cx.obarray_lookup(&name) == Some(v)).then_some(v))
}

/// GNU `find_symbol_value` (data.c:1619): the current binding through
/// aliases and buffer-local values, `None` when void; a non-symbol signals
/// `(wrong-type-argument symbolp X)' (CHECK_SYMBOL). An interned keyword's
/// empty cell reads as the keyword, as in `default_value'.
pub(crate) fn find_symbol_value(cx: &mut Leaf<'_>, v: Value) -> std::result::Result<Option<Value>, Fault> {
    let v = symbol(cx, v)?;
    match cx.variable_value(v)? {
        Some(value) => Ok(Some(value)),
        None => interned_keyword(cx, v),
    }
}

// GNU scans the original symbol's saved bindings before default-value's
// alias resolution. An alias therefore does not find its base's saved cell.
defsubr! { name = "default-toplevel-value", gnu = "eval.c:811", effect = Global,
fn default_toplevel_value(cx, symbol: Value) {
    let symbol = self::symbol(cx, symbol)?;
    match cx.saved_default_cell(symbol)? {
        Some(value) => Ok(value),
        None => default_value(cx, symbol)?.ok_or(Fault::VoidVariable(symbol)),
    }
}}

// With no saved default GNU delegates to set-default, including constants
// and keyword self-assignment. The public return value is always nil.
defsubr! { name = "set-default-toplevel-value", gnu = "eval.c:827", effect = Global,
fn set_default_toplevel_value(cx, symbol: Value, value: Value) {
    let symbol = self::symbol(cx, symbol)?;
    if !cx.saved_default_cell_set(symbol, value)? {
        set_default(cx, symbol, value)?;
    }
    Ok(Value::NIL)
}}

// eval.c:842/861 buffer-local-toplevel-value and its setter (prelude.el)
// read and replace the value a local let saved, through these.
defsubr! { name = "romacs--local-toplevel-cell", gnu = "eval.c:766", effect = Global,
fn local_toplevel_cell(cx, symbol: Value, buffer: Value) {
    let symbol = self::symbol(cx, symbol)?;
    Ok(match cx.local_toplevel_cell(symbol, buffer)? { Some(value) => cx.cons(Value::TRUE, value), None => Value::NIL })
}}
defsubr! { name = "romacs--set-local-toplevel-cell", gnu = "eval.c:766", effect = Global,
fn set_local_toplevel_cell(cx, symbol: Value, buffer: Value, value: Value) {
    let symbol = self::symbol(cx, symbol)?;
    Ok(if cx.local_toplevel_cell_set(symbol, buffer, value)? { Value::TRUE } else { Value::NIL })
}}

// Finternal_delete_indirect_variable: SYMBOL no longer an alias, void and
// undocumented; its base keeps its value.
defsubr! { name = "internal-delete-indirect-variable", gnu = "eval.c:728", effect = Global,
fn internal_delete_indirect_variable(cx, symbol: Value) {
    let symbol = self::symbol(cx, symbol)?;
    if cx.variable_alias(symbol)?.is_none() {
        let text = cx.byte_string(b"Cannot undeclare a variable that is not an alias");
        let data = cx.coding_list(&[text, symbol]);
        return Err(Fault::Signal(cx.intern("error"), data));
    }
    cx.delete_variable_alias(symbol);
    let key = cx.intern("variable-documentation");
    cx.put_symbol_property(symbol, key, Value::NIL)?;
    Ok(symbol)
}}

// Findirect_variable: the end of OBJECT's variable-alias chain; anything
// else is itself. defvaralias refuses cycles, so the chain ends.
defsubr! { name = "indirect-variable", gnu = "data.c:1291", effect = Reads,
fn indirect_variable(cx, object: Value) {
    if !(object == Value::NIL || object == Value::TRUE || cx.kind(object) == Kind::Symbol) {
        return Ok(object);
    }
    let mut symbol = object;
    for _ in 0..=cx.list_limit() {
        match cx.variable_alias(symbol)? {
            Some(base) => symbol = base,
            None => return Ok(symbol),
        }
    }
    Err(Fault::Circular(object))
}}
