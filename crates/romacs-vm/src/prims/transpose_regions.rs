//! GNU editfns.c transpose-regions: modify_text's bracket over the span
//! from the first region's start to the second's end, the swap after its
//! hooks, and one after-change over the span, whose length stays.
use super::*;

fn flag(b: bool) -> Value { if b { Value::TRUE } else { Value::NIL } }
fn fixnum(n: i64) -> Value { Value::integer(n).expect("buffer position") }

// Registers between the steps:
//   [STARTR1, ENDR1, STARTR2, ENDR2, LEAVE-MARKERS, S1, E1, S2, E2, INHIBITED, A1, B1, A2, B2]
// S1..E2 are the regions in buffer order and A1..B2 as given, each
// validated before the hooks ran.
defsubr! { name = "transpose-regions", gnu = "editfns.c:4539", effect = Global,
fn transpose_regions(cx, startr1: Value, endr1: Value, startr2: Value, endr2: Value; optional leave_markers: Value) -> Step {
    let Some((args, [s1, e1, s2, e2])) = cx.transpose_regions_span(startr1, endr1, startr2, endr2)? else { return Ok(Step::Done(Value::NIL)) };
    let prepared = cx.prepare_change(s1, e2)?;
    if (prepared.before || cx.after_change_runs(false, false)?) && super::insdel::half_missing(cx)? {
        return super::insdel::install_half(cx, restart, vec![startr1, endr1, startr2, endr2, leave_markers]);
    }
    let mut saved = vec![startr1, endr1, startr2, endr2, leave_markers, fixnum(s1), fixnum(e1), fixnum(s2), fixnum(e2), flag(prepared.inhibited)];
    saved.extend(args.map(fixnum));
    if !prepared.before { return swap(cx, Value::NIL, &saved); }
    let buffer = cx.current_buffer()?;
    let hooks = prepared.modification_hooks.iter().rev().fold(Value::NIL, |tail, &h| cx.cons(h, tail));
    let args = vec![buffer, fixnum(s1), fixnum(e2), hooks, flag(prepared.undoable)];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: swap, saved })
}}

fn restart(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    transpose_regions(cx, saved[0], saved[1], saved[2], saved[3], saved[4])
}

fn swap(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let overlays = super::insdel::found_overlays(cx, before);
    cx.change_deactivates_mark(saved[9] != Value::NIL)?;
    let span = [5, 6, 7, 8].map(|i| saved[i].integer_value().expect("position"));
    cx.modify_text_tail(span[0], span[3], false)?;
    let args = [10, 11, 12, 13].map(|i| saved[i].integer_value().expect("position"));
    cx.transpose_regions_swap(args, span, saved[4] != Value::NIL)?;
    if !cx.after_change_runs(overlays != Value::NIL, false)? { return Ok(Step::Done(Value::NIL)); }
    let buffer = cx.current_buffer()?;
    let (start, end) = (fixnum(span[0]), fixnum(span[3]));
    let args = vec![buffer, start, end, fixnum(span[3] - span[0]), Value::NIL, Value::NIL, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: swapped, saved: vec![] })
}

fn swapped(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}
