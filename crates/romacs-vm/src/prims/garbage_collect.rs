//! Explicit collection is an engine request, never a collecting leaf.
use super::*;

defsubr! { name = "garbage-collect", gnu = "alloc.c:6030", effect = Global,
fn garbage_collect(_cx) -> Step {
    Ok(Step::Collect)
}}

defsubr! { name = "garbage-collect-maybe", gnu = "alloc.c:6110", effect = Global,
fn garbage_collect_maybe(cx, factor: Value) -> Step {
    let factor = factor.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", factor))?;
    let (allocated, budget) = cx.collection_budget();
    if factor >= 1 && allocated > budget / factor as f64 {
        // The collector itself, as GNU calls it: no advice or redefinition.
        let collect = crate::prim::subr_named("garbage-collect").expect("garbage-collect subr");
        return Ok(Step::Call { f: collect, args: vec![], then: collected, saved: vec![] });
    }
    Ok(Step::Done(Value::NIL))
}}

// How far the next automatic collection is along, 1 being due
// (`Vm::collection_share'): for the command loop's idle collection, which
// the host makes at a request boundary, where request code retires.
defsubr! { name = "romacs--collection-share", gnu = "alloc.c:6110", effect = Global,
fn collection_share(cx) {
    Ok(cx.float(cx.collection_share()))
}}

/// `garbage-collect-maybe' after the collection it asked for.
fn collected(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::TRUE))
}

defsubr! { name = "make-finalizer", gnu = "alloc.c:3978", effect = Global,
fn make_finalizer(cx, function: Value) {
    if !super::vars::function_p(cx, function)? { return Err(Fault::Wrong("functionp", function)); }
    Ok(cx.make_finalizer(function))
}}

// The functions of the finalizers the collections since the last call
// found unreachable, for `romacs--run-finalizers' (rlisp/vm/prelude.el).
defsubr! { name = "romacs--take-doomed-finalizers", gnu = "alloc.c:3960", effect = Global,
fn take_doomed_finalizers(cx) {
    Ok(cx.take_doomed_finalizers())
}}

/// The engine's continuation after `romacs--run-finalizers', which
/// returns the collection's statistics it was given.
pub(crate) fn finalizers_ran(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}
