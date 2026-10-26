//! GNU eval.c Ffuncall as a callable subr. Calls written `(funcall F ...)`
//! compile to a direct call; this is the function value `#'funcall`.
use super::*;

defsubr! { name = "funcall", gnu = "eval.c:3185", effect = Global,
fn funcall(cx, function: Value; rest args) -> Step {
    Ok(Step::Call { f: function, args: args.to_vec(), then: called, saved: vec![] })
}}
fn called(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }
