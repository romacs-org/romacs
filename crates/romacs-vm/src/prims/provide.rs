//! fns.c Fprovide: record FEATURE, then run its `after-load-alist' entry.
use super::*;

defsubr! { name = "provide", gnu = "fns.c:3735", effect = Global,
fn provide(cx, feature: Value; optional subfeatures: Value) -> Step {
    cx.provide_feature(feature, subfeatures)?;
    let alist = super::fileio::var(cx, "after-load-alist")?;
    Ok(Step::Call { f: cx.intern("assq"), args: vec![feature, alist], then: hooks, saved: vec![feature] })
}}

// Fmapc (Qfuncall, XCDR (tem)) for an entry that is a cons.
fn hooks(cx: &mut Leaf<'_>, entry: Value, saved: &[Value]) -> Next {
    let Some((_, functions)) = cx.pair(entry) else { return Ok(Step::Done(saved[0])) };
    Ok(Step::Call { f: cx.intern("mapc"), args: vec![cx.intern("funcall"), functions], then: provided, saved: saved.to_vec() })
}

fn provided(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[0]))
}
