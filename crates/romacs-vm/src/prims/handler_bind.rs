//! GNU's non-unwinding error handlers are established by the engine.
use super::*;

defsubr! { name = "handler-bind-1", gnu = "eval.c:1551", effect = Global,
fn handler_bind_1(cx, body: Value; rest clauses) -> Step {
    if clauses.len() % 2 != 0 {
        let template = cx.string("Trailing CONDITIONS without HANDLER in `handler-bind`");
        let message = cx.format_message(template, &[])?;
        return Err(Fault::Error(cx.string_text(message)?.to_owned()));
    }
    let handlers = clauses.chunks_exact(2).filter_map(|pair| {
        (pair[0] != Value::NIL).then_some((pair[0], pair[1]))
    }).collect();
    Ok(Step::Handle { body, handlers })
}}
