//! `backquote-process', whose walk is the compiler's (expand.rs): every
//! load expands its templates (DIVERGENCES.md VM2-BACKQUOTE).
use super::*;
use crate::expand::{BadTemplate, Kind, Walk};

/// The condition backquote.el signals for BAD.
fn refusal(cx: &mut Leaf<'_>, bad: BadTemplate) -> std::result::Result<Fault, Fault> {
    Ok(match bad {
        BadTemplate::Dotted(tail) => Fault::Wrong("listp", cx.form_value(&tail)?),
        BadTemplate::Quote(operands) => {
            let (condition, quote) = (cx.intern("wrong-number-of-arguments"), cx.intern("quote"));
            let count = Value::integer(operands as i64).map_err(Fault::Error)?;
            let tail = cx.cons(count, Value::NIL);
            Fault::Signal(condition, cx.cons(quote, tail))
        }
        BadTemplate::Operands(comma, form) => {
            let form = cx.form_value(&form)?;
            let text = cx.print_text(form, true, Value::NIL, Value::NIL, Value::NIL)?;
            let text = cx.string_view(text).map(|view| String::from_utf8_lossy(view.bytes()).into_owned()).unwrap_or_default();
            Fault::Error(format!("Multiple args to {comma} are not supported: {text}"))
        }
    })
}

/// The walk as `backquote-unquote-symbol' and its siblings stand.
fn walk(cx: &mut Leaf<'_>) -> std::result::Result<Walk, Fault> {
    let mut marker = |name: &str, standard: crate::Form| {
        let variable = cx.intern(name);
        match cx.variable_value(variable)? {
            Some(value) => cx.expansion_datum(value),
            None => Ok(standard),
        }
    };
    let Walk { unquote, splice, backquote, bare } = Walk::standard();
    Ok(Walk { unquote: marker("backquote-unquote-symbol", unquote)?, splice: marker("backquote-splice-symbol", splice)?,
              backquote: marker("backquote-backquote-symbol", backquote)?, bare })
}

defsubr! { name = "romacs--backquote-process", gnu = "romacs:backquote", effect = Pure,
fn romacs_backquote_process(cx, structure: Value; optional level: Value) {
    let level = if level == Value::NIL { 0 } else { level.as_integer().map_err(|_| Fault::Wrong("integerp", level))? };
    let template = cx.expansion_datum(structure)?;
    let part = match walk(cx)?.template(&template, level) {
        Ok(part) => part,
        Err(bad) => return Err(refusal(cx, bad)?),
    };
    let tag = match part.kind { Kind::Constant => 0, Kind::Computed => 1, Kind::Spliced => 2 };
    let form = cx.form_value(&part.form)?;
    Ok(cx.cons(Value::integer(tag).map_err(Fault::Error)?, form))
}}
