//! GNU src/minibuf.c:1986-2058: the C completing-read entrypoint only.
//! Completion policy, input handling and defaults belong to GNU Lisp's
//! completing-read-function, not to this primitive.
use super::*;

defsubr! { name = "completing-read", gnu = "minibuf.c:1986", effect = Global,
fn completing_read(cx, prompt: Value, collection: Value; optional predicate: Value,
                   require_match: Value, initial_input: Value, hist: Value,
                   def: Value, inherit_input_method: Value) -> Step {
    let variable = cx.intern("completing-read-function");
    let function = cx.variable_value(variable)?.ok_or(Fault::VoidVariable(variable))?;
    Ok(Step::Call {
        f: function,
        args: vec![prompt, collection, predicate, require_match, initial_input,
                   hist, def, inherit_input_method],
        then: completed,
        saved: vec![],
    })
}}

fn completed(_: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    Ok(Step::Done(result))
}
