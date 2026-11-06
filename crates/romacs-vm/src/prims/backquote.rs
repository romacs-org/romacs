//! `backquote-process', whose walk is the compiler's (expand.rs): every
//! load expands its templates (DIVERGENCES.md VM2-BACKQUOTE).
use super::*;
use crate::expand::{self, BadTemplate, Kind};

/// The condition backquote.el signals for BAD.
fn refusal(cx: &mut Leaf<'_>, bad: BadTemplate) -> std::result::Result<Fault, Fault> {
    Ok(match bad {
        BadTemplate::Dotted(tail) => Fault::Wrong("listp", cx.form_value(&tail)?),
        BadTemplate::Operands(comma, form) => {
            let form = cx.form_value(&form)?;
            let text = cx.print_text(form, true, Value::NIL, Value::NIL, Value::NIL)?;
            let text = cx.string_view(text).map(|view| String::from_utf8_lossy(view.bytes()).into_owned()).unwrap_or_default();
            Fault::Error(format!("Multiple args to {comma} are not supported: {text}"))
        }
    })
}

defsubr! { name = "romacs--backquote-process", gnu = "romacs:backquote", effect = Pure,
fn romacs_backquote_process(cx, structure: Value; optional level: Value) {
    let level = if level == Value::NIL { 0 } else { level.as_integer().map_err(|_| Fault::Wrong("integerp", level))? };
    let template = cx.expansion_datum(structure)?;
    let part = match expand::backquote_process(&template, level) {
        Ok(part) => part,
        Err(bad) => return Err(refusal(cx, bad)?),
    };
    let tag = match part.kind { Kind::Constant => 0, Kind::Computed => 1, Kind::Spliced => 2 };
    let form = cx.form_value(&part.form)?;
    Ok(cx.cons(Value::integer(tag).map_err(Fault::Error)?, form))
}}
