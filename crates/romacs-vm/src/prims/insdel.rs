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
// KIND bit 0 is inherit, bit 1 before-markers and bit 2 printed output;
// NEXT indexes the argument being inserted, each one a change of its own as
// general_insert_function makes it; OPOINT is point before its hooks, where
// insert_from_string's after-change starts (a character's, as insdel.c
// insert's, and printed output's, as print_finish's, start at point less
// the inserted length after the edit); INHIBITED whether modification
// hooks were inhibited when it was prepared; BEHIND and IN-FRONT the text's insert hooks; OVERLAYS the
// (PROP . OVERLAY) hooks `romacs--before-change' found. The registers are
// traced across the Lisp calls.
const ARGS: usize = 7;

fn list(cx: &mut Leaf<'_>, values: &[Value]) -> Value {
    values.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail))
}
/// The overlay hooks in `romacs--before-change''s value (PRESERVE . OVERLAYS).
pub(crate) fn found_overlays(cx: &Leaf<'_>, before: Value) -> Value {
    cx.pair(before).map_or(Value::NIL, |(_, overlays)| overlays)
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
        let insert_hooks = prepared.behind != Value::NIL || prepared.in_front != Value::NIL;
        if (prepared.before || cx.after_change_runs(false, insert_hooks)?) && half_missing(cx)? {
            return install_half(cx, inserted, saved);
        }
        saved[2..ARGS].copy_from_slice(&[fixnum(point), flag(prepared.inhibited), prepared.behind, prepared.in_front, Value::NIL]);
        if prepared.before {
            let buffer = cx.current_buffer()?;
            let args = vec![buffer, fixnum(point), fixnum(point), Value::NIL, flag(prepared.undoable)];
            return Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: insert_prepared, saved });
        }
        if let Some(step) = insert_edit(cx, &mut saved)? { return Ok(step); }
    }
}

fn insert_prepared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    saved[6] = found_overlays(cx, before);
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
    let arg = saved[ARGS + next as usize];
    cx.change_deactivates_mark(saved[3] != Value::NIL)?;
    cx.insert_change(arg, kind & 1 != 0, kind & 2 != 0)?;
    saved[1] = fixnum(next + 1);
    let (behind, in_front, overlays) = (saved[4], saved[5], saved[6]);
    if !cx.after_change_runs(overlays != Value::NIL, behind != Value::NIL || in_front != Value::NIL)? { return Ok(None); }
    let point = cx.point_char()?;
    let start = match cx.string_chars(arg) {
        Some(_) if kind & 4 == 0 => saved[2],
        length => fixnum(point - length.unwrap_or(1) as i64),
    };
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, start, fixnum(point), fixnum(0), behind, in_front, overlays];
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

defsubr! { name = "romacs--insert-printed", gnu = "print.c:202", effect = Global,
fn insert_printed(cx, text: Value) -> Step {
    insert_family(cx, 4, &[text])
}}

// Registers of a deletion waiting on its before hooks:
//   [EXTRACT, LENGTH, INHIBITED]
// The hooks return FROM where they moved it, as GNU's preserve_ptr; the
// deletion is then LENGTH characters from there, cut at ZV.

/// del_range_1 for FROM..TO (FROM < TO, validated): prepare, the before
/// hooks, the deletion, the after hooks. EXTRACT returns the text.
fn delete_steps(cx: &mut Leaf<'_>, extract: bool, from: i64, to: i64) -> Next {
    let prepared = cx.prepare_change(from, to)?;
    if (prepared.before || cx.after_change_runs(false, false)?) && half_missing(cx)? {
        return install_half(cx, delete_installed, vec![flag(extract), fixnum(from), fixnum(to)]);
    }
    if !prepared.before {
        return delete_edit(cx, extract, from, to, prepared.inhibited, Value::NIL);
    }
    let buffer = cx.current_buffer()?;
    let hooks = list(cx, &prepared.modification_hooks);
    let args = vec![buffer, fixnum(from), fixnum(to), hooks, flag(prepared.undoable), fixnum(from)];
    let saved = vec![flag(extract), fixnum(to - from), flag(prepared.inhibited)];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: delete_prepared, saved })
}

fn delete_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let (from, to) = (saved[1].integer_value().expect("from"), saved[2].integer_value().expect("to"));
    delete_steps(cx, saved[0] != Value::NIL, from, to)
}

fn delete_prepared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let (preserved, overlays) = cx.pair(before).expect("romacs--before-change value");
    let from = preserved.integer_value().expect("preserved position");
    let (_, _, zv) = cx.point_extent()?;
    let to = zv.min(from + saved[1].integer_value().expect("deletion length"));
    delete_edit(cx, saved[0] != Value::NIL, from, to, saved[2] != Value::NIL, overlays)
}

fn delete_edit(cx: &mut Leaf<'_>, extract: bool, from: i64, to: i64, inhibited: bool, overlays: Value) -> Next {
    cx.change_deactivates_mark(inhibited)?;
    let text = cx.delete_change(from, to, extract)?;
    let result = if extract { text } else { Value::NIL };
    if !cx.after_change_runs(overlays != Value::NIL, false)? { return Ok(Step::Done(result)); }
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

// report_overlay_modification's scan for `romacs--before-change', which
// runs it after the hooks before it, as signal_before_change does.
defsubr! { name = "romacs--overlay-change-hooks", gnu = "buffer.c:4164", effect = Reads,
fn overlay_change_hooks(cx, beg: Value, end: Value) {
    let beg = beg.integer_value().ok_or(Fault::Wrong("fixnump", beg))?;
    let end = end.integer_value().ok_or(Fault::Wrong("fixnump", end))?;
    let hooks = cx.overlay_change_hooks(beg, end)?;
    let pairs: Vec<Value> = hooks.iter().map(|&(prop, overlay)| cx.cons(prop, overlay)).collect();
    Ok(list(cx, &pairs))
}}

// Freplace_region_contents's region check; the rest is the Lisp half's
// `romacs--replace-region-contents', over the substrate below: GNU's
// comparison (`romacs--replace-runs'), one replace_range
// (`romacs--replace-range') per change run with modification hooks bound
// off, and one prepare (`romacs--prepare-change') and after-change for the
// whole region.
defsubr! { name = "replace-region-contents", gnu = "editfns.c:1922", effect = Global,
fn replace_region_contents(cx, beg: Value, end: Value, source: Value; optional max_secs: Value, max_costs: Value, inherit: Value) -> Step {
    let (from, to) = cx.validate_region(beg, end)?;
    let saved = vec![fixnum(from), fixnum(to), source, max_secs, max_costs, inherit];
    if half_missing(cx)? { return install_half(cx, replace_region, saved); }
    replace_region(cx, Value::NIL, &saved)
}}

fn replace_region(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--replace-region-contents"), args: saved.to_vec(), then: done_value, saved: vec![] })
}

fn done_value(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

// validate_region for the Lisp half: (FROM . TO) of the current buffer.
defsubr! { name = "romacs--validate-region", gnu = "buffer.c:2504", effect = Reads,
fn validate_region_pair(cx, beg: Value, end: Value) {
    let (from, to) = cx.validate_region(beg, end)?;
    Ok(cx.cons(fixnum(from), fixnum(to)))
}}

// prepare_to_modify_buffer for BEG..END with no position to preserve: its
// checks, then its hooks when any can run. The value is the before hooks'
// (PRESERVE . OVERLAYS), whose overlays signal_after_change runs again.
defsubr! { name = "romacs--prepare-change", gnu = "insdel.c:2196", effect = Global,
fn prepare_change(cx, beg: Value, end: Value) -> Step {
    let from = beg.integer_value().ok_or(Fault::Wrong("fixnump", beg))?;
    let to = end.integer_value().ok_or(Fault::Wrong("fixnump", end))?;
    let prepared = cx.prepare_change(from, to)?;
    if !prepared.before {
        cx.change_deactivates_mark(prepared.inhibited)?;
        return Ok(Step::Done(cx.cons(Value::NIL, Value::NIL)));
    }
    if half_missing(cx)? { return install_half(cx, prepare_installed, vec![beg, end]); }
    let buffer = cx.current_buffer()?;
    let hooks = list(cx, &prepared.modification_hooks);
    let args = vec![buffer, beg, end, hooks, flag(prepared.undoable), Value::NIL];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: change_prepared, saved: vec![flag(prepared.inhibited)] })
}}

fn prepare_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--prepare-change"), args: saved.to_vec(), then: done_value, saved: vec![] })
}

fn change_prepared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    cx.change_deactivates_mark(saved[0] != Value::NIL)?;
    Ok(Step::Done(before))
}

// Registers of a replacement between its steps:
//   [FROM, LENGTH, SOURCE, ADJUST, INHIBITED, BEHIND, IN-FRONT]
// SOURCE is a string or [BUFFER BEG END]; ADJUST moves the match data
// with the change. The before hooks return FROM where they moved it, and
// the replaced text is LENGTH characters from there.
const REPLACE_WIDTH: usize = 7;

// replace_range (insdel.c:1463) with its hooks, inheriting no properties.
defsubr! { name = "romacs--replace-range", gnu = "insdel.c:1463", effect = Global,
fn replace_range(cx, from: Value, to: Value, source: Value, adjust: Value) -> Step {
    let start = from.integer_value().ok_or(Fault::Wrong("fixnump", from))?;
    let end = to.integer_value().ok_or(Fault::Wrong("fixnump", to))?;
    let prepared = cx.prepare_change(start, end)?;
    let insert_hooks = prepared.behind != Value::NIL || prepared.in_front != Value::NIL;
    if (prepared.before || cx.after_change_runs(false, insert_hooks)?) && half_missing(cx)? {
        return install_half(cx, replace_installed, vec![from, to, source, adjust]);
    }
    let mut saved = vec![Value::NIL; REPLACE_WIDTH];
    saved.copy_from_slice(&[from, fixnum(end - start), source, adjust, flag(prepared.inhibited), prepared.behind, prepared.in_front]);
    if !prepared.before { return replace_edit(cx, start, &saved, Value::NIL); }
    let buffer = cx.current_buffer()?;
    let hooks = list(cx, &prepared.modification_hooks);
    let args = vec![buffer, from, to, hooks, flag(prepared.undoable), from];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: replace_prepared, saved })
}}

fn replace_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--replace-range"), args: saved.to_vec(), then: done_value, saved: vec![] })
}

fn replace_prepared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let (preserved, overlays) = cx.pair(before).expect("romacs--before-change value");
    replace_edit(cx, preserved.integer_value().expect("preserved position"), saved, overlays)
}

/// The text a replacement inserts: SOURCE itself, or BUFFER's BEG..END.
fn replacement_text(cx: &mut Leaf<'_>, source: Value) -> std::result::Result<Value, Fault> {
    if cx.string_chars(source).is_some() { return Ok(source); }
    let field = |i: i64| cx.vector_get(source, fixnum(i));
    let (buffer, beg, end) = (field(0)?, field(1)?, field(2)?);
    let position = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    cx.buffer_region_text(buffer, position(beg)?, position(end)?)
}

fn replace_edit(cx: &mut Leaf<'_>, from: i64, saved: &[Value], overlays: Value) -> Next {
    let to = from + saved[1].integer_value().expect("replaced length");
    cx.change_deactivates_mark(saved[4] != Value::NIL)?;
    let text = replacement_text(cx, saved[2])?;
    let Some((from, to, inserted)) = cx.replace_change(from, to, text)? else { return Ok(Step::Done(Value::NIL)) };
    if saved[3] != Value::NIL { cx.update_search_regs(from, to, from + inserted); }
    let (behind, in_front) = (saved[5], saved[6]);
    if !cx.after_change_runs(overlays != Value::NIL, behind != Value::NIL || in_front != Value::NIL)? {
        return Ok(Step::Done(Value::NIL));
    }
    let buffer = cx.current_buffer()?;
    let args = vec![buffer, fixnum(from), fixnum(from + inserted), fixnum(to - from), behind, in_front, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: done, saved: vec![] })
}

/// Character codes as GNU compares them: a unibyte string's bytes above
/// 127 as raw-byte characters (BUF_FETCH_CHAR_AS_MULTIBYTE).
fn char_codes(cx: &Leaf<'_>, text: Value) -> std::result::Result<Vec<u32>, Fault> {
    let view = cx.string_view(text)?;
    Ok(if view.is_multibyte() {
        view.chars().collect()
    } else {
        view.bytes().iter().map(|&b| if b < 128 { u32::from(b) } else { u32::from(b) + crate::lstring::BYTE8_OFFSET }).collect()
    })
}

// editfns.c's comparison of BEG..END with SOURCE, a string or
// [BUFFER SBEG SEND]: the change runs, last first, as
// (BEG-A END-A BEG-B END-B) (B positions index a string from 0), or t
// when it took longer than MAX-SECS seconds (nil for no limit) or more
// memory than there is.
defsubr! { name = "romacs--replace-runs", gnu = "editfns.c:2148", effect = Reads,
fn replace_runs(cx, beg: Value, end: Value, source: Value, too_expensive: Value, max_secs: Value) {
    let position = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    let (min_a, end_a) = (position(beg)?, position(end)?);
    let text = cx.region_text(min_a, end_a)?;
    let a = char_codes(cx, text)?;
    let (min_b, text) = if cx.string_chars(source).is_some() {
        (0, source)
    } else {
        let (buffer, sbeg, send) = (cx.vector_get(source, fixnum(0))?, cx.vector_get(source, fixnum(1))?, cx.vector_get(source, fixnum(2))?);
        (position(sbeg)?, cx.buffer_region_text(buffer, position(sbeg)?, position(send)?)?)
    };
    let b = char_codes(cx, text)?;
    let deadline = if max_secs == Value::NIL { None } else {
        let secs = cx.float_value(max_secs)?;
        // A limit past what a time_t holds is no limit.
        (secs < 9.2e18).then(|| crate::host::monotonic() + std::time::Duration::from_secs_f64(secs.max(0.0)))
    };
    let expensive = position(too_expensive)? as isize;
    let Some((deletions, insertions)) = super::compareseq::compare(&a, &b, expensive, deadline) else {
        return Ok(Value::TRUE);
    };
    let runs: Vec<Value> = super::compareseq::runs(&deletions, &insertions).into_iter().map(|(i, i_end, j, j_end)| {
        let run = [min_a + i as i64, min_a + i_end as i64, min_b + j as i64, min_b + j_end as i64].map(fixnum);
        list(cx, &run)
    }).collect();
    Ok(list(cx, &runs))
}}
