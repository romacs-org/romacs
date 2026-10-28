//! GNU editfns.c subst-char-in-region: modify_text's bracket from the
//! first character to change to END, then the rewrite after its hooks,
//! rescanned as GNU restarts, and one after-change over what changed.
use super::*;

fn flag(b: bool) -> Value { if b { Value::TRUE } else { Value::NIL } }
fn fixnum(n: i64) -> Value { Value::integer(n).expect("buffer position") }

// Registers between the steps:
//   [START, END, FROMCHAR, TOCHAR, NOUNDO, POS, STOP, INHIBITED]
// POS..STOP is the region modify_text prepared, as positions before its
// hooks; the overlay hooks are those `romacs--before-change' returns.
defsubr! { name = "subst-char-in-region", gnu = "editfns.c:2294", effect = Global,
fn subst_char_in_region(cx, start: Value, end: Value, fromchar: Value, tochar: Value; optional noundo: Value) -> Step {
    let (hits, stop) = cx.subst_char_hits(start, end, fromchar, tochar)?;
    let Some(&pos) = hits.first() else { return Ok(Step::Done(Value::NIL)) };
    let prepared = cx.prepare_change(pos, stop)?;
    if (prepared.before || cx.after_change_runs(false, false)?) && super::insdel::half_missing(cx)? {
        return super::insdel::install_half(cx, restart, vec![start, end, fromchar, tochar, noundo]);
    }
    let saved = vec![start, end, fromchar, tochar, noundo, fixnum(pos), fixnum(stop), flag(prepared.inhibited)];
    if !prepared.before { return substitute(cx, Value::NIL, &saved); }
    let buffer = cx.current_buffer()?;
    let hooks = prepared.modification_hooks.iter().rev().fold(Value::NIL, |tail, &h| cx.cons(h, tail));
    // NOUNDO turns undo off around the change: no undo-auto call.
    let undoable = flag(prepared.undoable && noundo == Value::NIL);
    let args = vec![buffer, fixnum(pos), fixnum(stop), hooks, undoable];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: substitute, saved })
}}

fn restart(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    subst_char_in_region(cx, saved[0], saved[1], saved[2], saved[3], saved[4])
}

fn substitute(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let overlays = super::insdel::found_overlays(cx, before);
    let noundo = saved[4] != Value::NIL;
    cx.change_deactivates_mark(saved[7] != Value::NIL)?;
    let (pos, stop) = (saved[5].integer_value().expect("pos"), saved[6].integer_value().expect("stop"));
    cx.modify_text_tail(pos, stop, noundo)?;
    let (hits, _) = cx.subst_char_hits(saved[0], saved[1], saved[2], saved[3])?;
    let (Some(&changed), Some(&last)) = (hits.first(), hits.last()) else { return Ok(Step::Done(Value::NIL)) };
    cx.subst_char_write(&hits, saved[3], noundo)?;
    let last = last + 1;
    if !cx.after_change_runs(overlays != Value::NIL, false)? { return Ok(Step::Done(Value::NIL)); }
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, fixnum(changed), fixnum(last), fixnum(last - changed), Value::NIL, Value::NIL, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: substituted, saved: vec![] })
}

fn substituted(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}
