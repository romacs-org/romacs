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

// Where the process's heap bytes are: a plist of the production
// allocator's :platform-allocated and :platform-resident bytes
// (`romacs_allocator::stats`, 0 where it keeps none), the arenas' :used,
// :small-free, :slab-rest, :runs-kept, :runs-released and :above-top bytes
// (`world::arena::census`) and :classes, an alist of block size to free
// bytes; nil when the heap is the system allocator's.
defsubr! { name = "romacs--arena-census", gnu = "alloc.c:7154", effect = Global,
fn arena_census(cx) {
    let Some(c) = crate::world::arena::census() else { return Ok(Value::NIL) };
    let int = |n: usize| Value::integer(n as i64).expect("byte count fits fixnum");
    let classes = c.classes.iter().rev().fold(Value::NIL, |tail, &(size, free)| {
        let row = cx.cons(int(size), int(free));
        cx.cons(row, tail)
    });
    let (allocated, resident) = romacs_allocator::stats().unwrap_or((0, 0));
    let rows = [("platform-allocated", int(allocated)), ("platform-resident", int(resident)), ("used", int(c.used)),
        ("small-free", int(c.small_free)), ("slab-rest", int(c.slab_rest)), ("runs-kept", int(c.runs_kept)),
        ("runs-released", int(c.runs_released)), ("above-top", int(c.above_top)), ("classes", classes)];
    Ok(rows.iter().rev().fold(Value::NIL, |tail, &(key, value)| {
        let key = cx.intern(&format!(":{key}"));
        let tail = cx.cons(value, tail);
        cx.cons(key, tail)
    }))
}}

// The GNU/Linux sysinfo branch, in KiB; a handler for `default-directory'
// answers for a remote host.
defsubr! { name = "memory-info", gnu = "alloc.c:7154", effect = Global,
fn memory_info(cx) -> Step {
    let directory = super::fileio::var(cx, "default-directory")?;
    if let Some(call) = super::fileio::handler(cx, directory, "memory-info", vec![])? { return call; }
    let Some(info) = crate::host::memory_info() else { return Ok(Step::Done(Value::NIL)) };
    let list = info.iter().rev().fold(Value::NIL, |list, bytes| {
        let kib = cx.bigint((bytes / 1024).into());
        cx.cons(kib, list)
    });
    Ok(Step::Done(list))
}}
