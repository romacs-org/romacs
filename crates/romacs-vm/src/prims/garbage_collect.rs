//! Explicit collection is an engine request, never a collecting leaf.
use super::*;

defsubr! { name = "garbage-collect", gnu = "alloc.c:6030", effect = Global,
fn garbage_collect(_cx) -> Step {
    Ok(Step::Collect)
}}

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
