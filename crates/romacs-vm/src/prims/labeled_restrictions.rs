//! editfns.c's internal halves of `with-restriction` and
//! `without-restriction` with a label (`prim/leaf/labeled_restrictions.rs`).
use super::{Leaf, Next, Step, Value};

// Finternal__labeled_narrow_to_region: narrow to START-END, then push
// (LABEL BEGV ZV) at the new bounds, above the bounds before when it is the
// buffer's first.
defsubr! { name = "internal--labeled-narrow-to-region", gnu = "editfns.c:3008", effect = Global,
fn internal_labeled_narrow_to_region(cx, start: Value, end: Value, label: Value) -> Step {
    let outermost = cx.labeled_outermost()?;
    let f = cx.intern("narrow-to-region");
    Ok(Step::Call { f, args: vec![start, end], then: narrowed, saved: vec![label, outermost] })
}}

fn narrowed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let &[label, outermost] = saved else { unreachable!("two registers") };
    cx.push_labeled(label, outermost)?;
    Ok(Step::Done(Value::NIL))
}

// Finternal__labeled_widen: drop the innermost labeled restriction when it
// is LABEL's, then widen, as far as the one left allows.
defsubr! { name = "internal--labeled-widen", gnu = "editfns.c:3028", effect = Global,
fn internal_labeled_widen(cx, label: Value) -> Step {
    cx.pop_labeled_if(label)?;
    let f = cx.intern("widen");
    Ok(Step::Call { f, args: vec![], then: widened, saved: vec![] })
}}

#[expect(clippy::unnecessary_wraps, reason = "a Step::Call continuation's type")]
fn widened(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}
