//! GNU insdel.c through editfns.c, buffer.c and cmds.c: insert and its
//! variants, delete-region, delete-and-extract-region, erase-buffer,
//! delete-char and self-insert-command. Each change is GNU's bracket
//! (prim/leaf/insdel.rs): prepare, the before hooks in Lisp when any can run
//! (`romacs--before-change'), the edit with its undo records, then the after
//! hooks when any can run (`romacs--after-change').
//!
//! The hooks' Lisp half (rlisp/vm/insdel.el) is installed by one
//! `Step::Call' of `eval' the first time a change has Lisp to run, as
//! minibuf.rs installs its own: a bare `Vm::new()' stays bare.
use super::*;

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/insdel.el");

/// Whether the Lisp half must be installed before a change runs Lisp.
pub(crate) fn half_missing(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let helper = cx.intern("romacs--after-change");
    Ok(cx.symbol_function(helper)? == Value::NIL)
}

/// Install the Lisp half, then continue with THEN and SAVED.
pub(crate) fn install_half(cx: &mut Leaf<'_>, then: Then, saved: Vec<Value>) -> Next {
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then, saved })
}

// Registers of an insertion between its steps:
//   [KIND, NEXT, OPOINT, INHIBITED, BEHIND, IN-FRONT, OVERLAYS, ARGS...]
// KIND bit 0 is inherit and bit 1 before-markers; NEXT indexes the argument
// being inserted, each one a change of its own as general_insert_function
// makes it; OPOINT is point before its hooks, where GNU's after-change
// starts; INHIBITED whether modification hooks were inhibited when it was
// prepared; BEHIND and IN-FRONT the text's insert hooks; OVERLAYS a list
// of (PROP . OVERLAY). The registers are traced across the Lisp calls.
const ARGS: usize = 7;

fn list(cx: &mut Leaf<'_>, values: &[Value]) -> Value {
    values.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail))
}
fn overlay_list(cx: &mut Leaf<'_>, hooks: &[(Value, Value)]) -> Value {
    let pairs: Vec<Value> = hooks.iter().map(|&(prop, overlay)| cx.cons(prop, overlay)).collect();
    list(cx, &pairs)
}
fn flag(b: bool) -> Value { if b { Value::TRUE } else { Value::NIL } }
fn fixnum(n: i64) -> Value { Value::integer(n).expect("buffer position") }

fn insert_family(cx: &mut Leaf<'_>, kind: i64, args: &[Value]) -> Next {
    let mut saved = vec![fixnum(kind), fixnum(0), Value::NIL, Value::NIL, Value::NIL, Value::NIL, Value::NIL];
    saved.extend_from_slice(args);
    insert_from(cx, saved)
}

/// Insert the arguments from NEXT on, each prepared, edited and signalled.
fn insert_from(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    loop {
        let next = saved[1].integer_value().expect("argument index") as usize;
        let Some(&arg) = saved.get(ARGS + next) else { return Ok(Step::Done(Value::NIL)) };
        let character = arg.integer_value().is_some_and(|c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c));
        if !character {
            match cx.string_chars(arg) {
                Some(0) => { saved[1] = fixnum(next as i64 + 1); continue; }
                Some(_) => {}
                None => return Err(Fault::Wrong("char-or-string-p", arg)),
            }
        }
        let point = cx.point_char()?;
        let prepared = cx.prepare_change(point, point)?;
        let overlays = overlay_list(cx, &prepared.overlays);
        let insert_hooks = prepared.behind != Value::NIL || prepared.in_front != Value::NIL;
        if (prepared.before || cx.after_change_runs(overlays, insert_hooks)?) && half_missing(cx)? {
            return install_half(cx, inserted, saved);
        }
        saved[2..ARGS].copy_from_slice(&[fixnum(point), flag(prepared.inhibited), prepared.behind, prepared.in_front, overlays]);
        if prepared.before {
            let buffer = cx.current_buffer()?;
            let args = vec![buffer, fixnum(point), fixnum(point), Value::NIL, flag(prepared.undoable), overlays];
            return Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: insert_prepared, saved });
        }
        if let Some(step) = insert_edit(cx, &mut saved)? { return Ok(step); }
    }
}

fn insert_prepared(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    match insert_edit(cx, &mut saved)? {
        Some(step) => Ok(step),
        None => insert_from(cx, saved),
    }
}

/// The insertion itself, then its after hooks when any run (None when
/// the next argument follows directly).
fn insert_edit(cx: &mut Leaf<'_>, saved: &mut Vec<Value>) -> std::result::Result<Option<Step>, Fault> {
    let kind = saved[0].integer_value().expect("insert kind");
    let next = saved[1].integer_value().expect("argument index");
    cx.change_deactivates_mark(saved[3] != Value::NIL)?;
    cx.insert_change(saved[ARGS + next as usize], kind & 1 != 0, kind & 2 != 0)?;
    saved[1] = fixnum(next + 1);
    let (behind, in_front, overlays) = (saved[4], saved[5], saved[6]);
    if !cx.after_change_runs(overlays, behind != Value::NIL || in_front != Value::NIL)? { return Ok(None); }
    let point = cx.point_char()?;
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, saved[2], fixnum(point), fixnum(0), behind, in_front, overlays];
    Ok(Some(Step::Call { f: cx.intern("romacs--after-change"), args, then: inserted, saved: saved.clone() }))
}

fn inserted(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    insert_from(cx, saved.to_vec())
}

defsubr! { name = "insert", gnu = "editfns.c:1354", effect = Global,
fn insert(cx; rest args) -> Step {
    insert_family(cx, 0, args)
}}

defsubr! { name = "insert-and-inherit", gnu = "editfns.c:1377", effect = Global,
fn insert_and_inherit(cx; rest args) -> Step {
    insert_family(cx, 1, args)
}}

defsubr! { name = "insert-before-markers", gnu = "editfns.c:1397", effect = Global,
fn insert_before_markers(cx; rest args) -> Step {
    insert_family(cx, 2, args)
}}

defsubr! { name = "insert-before-markers-and-inherit", gnu = "editfns.c:1422", effect = Global,
fn insert_before_markers_and_inherit(cx; rest args) -> Step {
    insert_family(cx, 3, args)
}}

// Registers of a deletion between its steps:
//   [EXTRACT, FROM, LENGTH, INHIBITED, OVERLAYS]
// FROM follows the before hooks as GNU's preserve_ptr marker does; the
// deletion is then LENGTH characters from there, cut at ZV.

/// del_range_1 for FROM..TO (FROM < TO, validated): prepare, the before
/// hooks, the deletion, the after hooks. EXTRACT returns the text.
fn delete_steps(cx: &mut Leaf<'_>, extract: bool, from: i64, to: i64) -> Next {
    let prepared = cx.prepare_change(from, to)?;
    let overlays = overlay_list(cx, &prepared.overlays);
    if (prepared.before || cx.after_change_runs(overlays, false)?) && half_missing(cx)? {
        return install_half(cx, delete_installed, vec![flag(extract), fixnum(from), fixnum(to)]);
    }
    if !prepared.before {
        return delete_edit(cx, extract, from, to, prepared.inhibited, overlays);
    }
    let buffer = cx.current_buffer()?;
    let hooks = list(cx, &prepared.modification_hooks);
    let preserved = cx.preserve_position(from)?;
    let args = vec![buffer, fixnum(from), fixnum(to), hooks, flag(prepared.undoable), overlays];
    let saved = vec![flag(extract), preserved, fixnum(to - from), flag(prepared.inhibited), overlays];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: delete_prepared, saved })
}

fn delete_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (from, to) = (saved[1].integer_value().expect("from"), saved[2].integer_value().expect("to"));
    delete_steps(cx, saved[0] != Value::NIL, from, to)
}

fn delete_prepared(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let from = cx.preserved_position(saved[1])?;
    let (_, _, zv) = cx.point_extent()?;
    let from = from.min(zv);
    let to = zv.min(from + saved[2].integer_value().expect("deletion length"));
    delete_edit(cx, saved[0] != Value::NIL, from, to, saved[3] != Value::NIL, saved[4])
}

fn delete_edit(cx: &mut Leaf<'_>, extract: bool, from: i64, to: i64, inhibited: bool, overlays: Value) -> Next {
    cx.change_deactivates_mark(inhibited)?;
    let text = cx.delete_change(from, to, extract)?;
    let result = if extract { text } else { Value::NIL };
    if !cx.after_change_runs(overlays, false)? { return Ok(Step::Done(result)); }
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, fixnum(from), fixnum(from), fixnum(to - from), Value::NIL, Value::NIL, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: deleted, saved: vec![result] })
}

fn deleted(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[0]))
}

defsubr! { name = "delete-region", gnu = "editfns.c:2681", effect = Global,
fn delete_region(cx, start: Value, end: Value) -> Step {
    let (from, to) = cx.validate_region(start, end)?;
    if from == to { return Ok(Step::Done(Value::NIL)); }
    delete_steps(cx, false, from, to)
}}

defsubr! { name = "delete-and-extract-region", gnu = "editfns.c:2692", effect = Global,
fn delete_and_extract_region(cx, start: Value, end: Value) -> Step {
    let (from, to) = cx.validate_region(start, end)?;
    if from == to { return Ok(Step::Done(cx.byte_string(&[]))); }
    delete_steps(cx, true, from, to)
}}

// Ferase_buffer: widen, then del_range (BEG, Z).
defsubr! { name = "erase-buffer", gnu = "buffer.c:2484", effect = Global,
fn erase_buffer(cx) -> Step {
    cx.widen_current()?;
    let (_, begv, zv) = cx.point_extent()?;
    if begv == zv { return Ok(Step::Done(Value::NIL)); }
    delete_steps(cx, false, begv, zv)
}}

fn done(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    Ok(Step::Done(Value::NIL))
}

// Fdelete_char: undo-auto-amalgamate for a single character, then
// del_range from point, or kill-forward-chars with KILLFLAG.
defsubr! { name = "delete-char", gnu = "cmds.c:221", effect = Global,
fn delete_char(cx, n: Value; optional killflag: Value) -> Step {
    let count = n.integer_value().ok_or(Fault::Wrong("fixnump", n))?;
    let amalgamate = cx.intern("undo-auto-amalgamate");
    if count.abs() < 2 && cx.symbol_function(amalgamate)? != Value::NIL {
        return Ok(Step::Call { f: amalgamate, args: vec![], then: delete_chars, saved: vec![n, killflag] });
    }
    delete_chars(cx, Value::NIL, &[n, killflag])
}}

fn delete_chars(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let n = saved[0].integer_value().expect("checked count");
    if saved[1] != Value::NIL {
        return Ok(Step::Call { f: cx.intern("kill-forward-chars"), args: vec![saved[0]], then: done, saved: vec![] });
    }
    let (point, begv, zv) = cx.point_extent()?;
    let pos = point + n;
    if pos < begv { return Err(Fault::Signal(cx.intern("beginning-of-buffer"), Value::NIL)); }
    if pos > zv { return Err(Fault::Signal(cx.intern("end-of-buffer"), Value::NIL)); }
    if pos == point { return Ok(Step::Done(Value::NIL)); }
    delete_steps(cx, false, pos.min(point), pos.max(point))
}

// simple.el's command for a VM without it; simple.el's own definition
// replaces this one when it loads.
defsubr! { name = "delete-backward-char", gnu = "simple.el:1478", effect = Global,
fn delete_backward_char(cx, n: Value; optional killflag: Value) -> Step {
    let count = n.integer_value().ok_or(Fault::Wrong("fixnump", n))?;
    let n = Value::integer(-count).map_err(Fault::Error)?;
    Ok(Step::Call { f: cx.intern("delete-char"), args: vec![n, killflag], then: done, saved: vec![] })
}}

// Fself_insert_command's argument checks; the rest is the Lisp half's
// `romacs--self-insert', since internal_self_insert calls Lisp throughout.
defsubr! { name = "self-insert-command", gnu = "cmds.c:263", effect = Global,
fn self_insert_command(cx, n: Value; optional c: Value) -> Step {
    let count = n.integer_value().ok_or(Fault::Wrong("fixnump", n))?;
    let event = cx.intern("last-command-event");
    let c = if c == Value::NIL { cx.variable_value(event)?.unwrap_or(Value::NIL) } else { cx.variable_set(event, c)?; c };
    if count < 0 { return Err(Fault::Error(format!("Negative repetition argument {count}"))); }
    if half_missing(cx)? { return install_half(cx, self_insert, vec![n, c]); }
    self_insert(cx, Value::NIL, &[n, c])
}}

fn self_insert(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--self-insert"), args: saved.to_vec(), then: done, saved: vec![] })
}
