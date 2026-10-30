//! fns.c Flocale_info over crate::locale_info.
use super::*;
use crate::locale_info::{query, Info, Item};

defsubr! { name = "locale-info", gnu = "fns.c:3896", effect = Reads,
fn locale_info(cx, item: Value) {
    let item = if item == cx.intern("codeset") { Item::Codeset } else if item == cx.intern("days") { Item::Days }
        else if item == cx.intern("months") { Item::Months } else if item == cx.intern("paper") { Item::Paper }
        else { return Ok(Value::NIL) };
    // Names are decoded with `locale-coding-system', none before startup
    // sets it; the codeset is built as it comes.
    let coding = cx.intern("locale-coding-system");
    let decode = cx.variable_value(coding)?.is_some_and(|v| v != Value::NIL);
    let mut string = |cx: &mut Leaf<'_>, bytes: Vec<u8>, decode: bool| match String::from_utf8(bytes) {
        Ok(text) if decode || text.is_ascii() => cx.string(&text),
        Ok(text) => cx.make_string(text.into_bytes(), false),
        Err(raw) => cx.make_string(raw.into_bytes(), false),
    };
    Ok(match query(item) {
        None => Value::NIL,
        Some(Info::Text(bytes)) => string(cx, bytes, false),
        Some(Info::Texts(names)) => {
            let values: Vec<Value> = names.into_iter().map(|bytes| string(cx, bytes, decode)).collect();
            cx.vector(&values)
        }
        Some(Info::Paper(width, height)) => {
            let (width, height) = (Value::integer(width).map_err(Fault::Error)?, Value::integer(height).map_err(Fault::Error)?);
            let tail = cx.cons(height, Value::NIL);
            cx.cons(width, tail)
        }
    })
}}
