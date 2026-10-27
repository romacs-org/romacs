//! GNU indent.c columns and insert-char. Insertion and deletion are ordinary
//! T2 calls of `insert-and-inherit' and `delete-region', so modification
//! hooks, undo and markers behave as for any insertion.
use super::*;

fn fixnum(value: Value) -> std::result::Result<i64, Fault> {
    value.integer_value().ok_or(Fault::Wrong("fixnump", value))
}
fn done(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { Ok(Step::Done(saved[0])) }

defsubr! { name = "insert-char", gnu = "editfns.c:1444", effect = Global,
fn insert_char(cx, character: Value; optional count: Value, inherit: Value) -> Step {
    let c = character.integer_value().filter(|&c| (0..=crate::lstring::MAX_CHAR as i64).contains(&c))
        .ok_or(Fault::Wrong("characterp", character))? as u32;
    let count = if count == Value::NIL { 1 } else { fixnum(count)? };
    if count <= 0 { return Ok(Step::Done(Value::NIL)); }
    let mut unit = [0; 5];
    let width = crate::lstring::char_string(c, &mut unit);
    let bytes = unit[..width].repeat(count as usize);
    let text = cx.make_string(bytes, true);
    let f = cx.intern(if inherit == Value::NIL { "insert" } else { "insert-and-inherit" });
    Ok(Step::Call { f, args: vec![text], then: done, saved: vec![Value::NIL] })
}}

defsubr! { name = "current-indentation", gnu = "indent.c:997", effect = Reads,
fn current_indentation(cx) {
    cx.current_indentation()
}}

// Findent_to: tabs up to the last stop when indent-tabs-mode, then spaces.
fn indent_to_steps(cx: &mut Leaf<'_>, column: i64, minimum: i64) -> Next {
    let from = cx.native_current_column()?.integer_value().unwrap();
    let goal = (from + minimum).max(column);
    let result = Value::integer(goal).map_err(Fault::Error)?;
    if from == goal { return Ok(Step::Done(result)); }
    let tabs_mode = cx.intern("indent-tabs-mode");
    let tab = { let v = cx.intern("tab-width"); cx.variable_value(v)?.and_then(|v| v.integer_value()).filter(|n| (1..=1000).contains(n)).unwrap_or(8) };
    let mut from = from;
    let mut tabs = 0;
    if cx.variable_value(tabs_mode)?.is_some_and(|v| v != Value::NIL) {
        tabs = goal / tab - from / tab;
        if tabs != 0 { from = (goal / tab) * tab; }
    }
    let spaces = Value::integer(goal - from).map_err(Fault::Error)?;
    let insert = cx.intern("insert-char");
    if tabs != 0 {
        return Ok(Step::Call { f: insert, args: vec![Value::integer(9).unwrap(), Value::integer(tabs).unwrap(), Value::TRUE],
            then: indent_spaces, saved: vec![result, spaces] });
    }
    Ok(Step::Call { f: insert, args: vec![Value::integer(32).unwrap(), spaces, Value::TRUE], then: done, saved: vec![result] })
}
fn indent_spaces(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let insert = cx.intern("insert-char");
    Ok(Step::Call { f: insert, args: vec![Value::integer(32).unwrap(), saved[1], Value::TRUE], then: done, saved: vec![saved[0]] })
}

defsubr! { name = "indent-to", gnu = "indent.c:947", effect = Global,
fn indent_to(cx, column: Value; optional minimum: Value) -> Step {
    let column = fixnum(column)?;
    let minimum = if minimum == Value::NIL { 0 } else { fixnum(minimum)? };
    indent_to_steps(cx, column, minimum)
}}

// Fmove_to_column. With FORCE, a tab that overshoots GOAL becomes spaces
// up to GOAL followed by indentation to the tab's old end, and with FORCE t
// a short line is indented out to GOAL.
defsubr! { name = "move-to-column", gnu = "indent.c:1112", effect = Global,
fn move_to_column(cx, column: Value; optional force: Value) -> Step {
    let goal = column.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("wholenump", column))?;
    let (reached, prev_byte, prev_column, prev_char) = cx.scan_to_column(goal as usize)?;
    let reached = reached as i64;
    let goal_value = Value::integer(goal).unwrap();
    if force != Value::NIL && reached > goal && prev_char == Some('\t') && (prev_column as i64) < goal {
        // Spaces before the tab first, so a marker after the tab moves.
        cx.set_point_byte(prev_byte)?;
        let spaces = Value::integer(goal - prev_column as i64).unwrap();
        let insert = cx.intern("insert-char");
        return Ok(Step::Call { f: insert, args: vec![Value::integer(32).unwrap(), spaces, Value::TRUE],
            then: split_tab_delete, saved: vec![goal_value, Value::integer(reached).unwrap()] });
    }
    if reached < goal && force == Value::TRUE {
        return indent_to_steps(cx, goal, 0);
    }
    Ok(Step::Done(Value::integer(reached).unwrap()))
}}
fn split_tab_delete(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let point = cx.point()?;
    let delete = cx.intern("delete-region");
    Ok(Step::Call { f: delete, args: vec![Value::integer(point).unwrap(), Value::integer(point + 1).unwrap()],
        then: split_tab_indent, saved: vec![saved[0], saved[1], Value::integer(point).unwrap()] })
}
fn split_tab_indent(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let indent = cx.intern("indent-to");
    Ok(Step::Call { f: indent, args: vec![saved[1]], then: split_tab_return, saved: vec![saved[0], saved[2]] })
}
fn split_tab_return(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    cx.goto_char_position(saved[1].integer_value().unwrap())?;
    Ok(Step::Done(saved[0]))
}

// line_number_display_width: no width while `display-line-numbers' is nil;
// otherwise GNU lays the window out, which VM2 cannot do synchronously.
defsubr! { name = "line-number-display-width", gnu = "indent.c:2150", effect = Reads,
fn line_number_display_width(cx; optional pixelwise: Value) {
    let numbers = cx.intern("display-line-numbers");
    if cx.variable_value(numbers)?.unwrap_or(Value::NIL) != Value::NIL {
        return Err(Fault::Error("VM2 line-number-display-width with display-line-numbers needs synchronous layout, which is unsupported".into()));
    }
    Ok(if pixelwise == cx.intern("columns") { cx.float(0.0) } else { Value::integer(0).expect("fixnum") })
}}
