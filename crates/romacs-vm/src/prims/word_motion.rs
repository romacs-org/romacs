//! Native syntax-table word-motion prerequisite for GNU Lisp commands.
use super::*;

defsubr! { name = "forward-word", gnu = "syntax.c:1581", effect = Global,
fn forward_word(cx; optional count: Value) -> Step {
    // syntax.c scan_words calls the table's functions, so a table with any
    // takes lisp/romacs/vm/prelude.el's scan: `subword-mode', mostly.
    if cx.word_boundary_functions()? {
        let count = if count == Value::NIL { Value::integer(1).map_err(Fault::Error)? } else { count };
        count.integer_value().ok_or(Fault::Wrong("fixnump", count))?;
        return Ok(Step::Call { f: cx.intern("romacs--forward-word"), args: vec![count], then: word_moved, saved: vec![] });
    }
    cx.move_words(count).map(Step::Done)
}}

fn word_moved(_cx: &mut Leaf<'_>, value: Value, _saved: &[Value]) -> Next {
    Ok(Step::Done(value))
}
