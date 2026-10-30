//! editfns.c fields: runs of text with an `eq' `field' char-property.
//! find_field and constrain-to-field are prim/leaf/field.rs.
use super::*;

defsubr! { name = "current-column", gnu = "indent.c:298", effect = Reads,
fn current_column(cx) { cx.native_current_column() }}

defsubr! { name = "constrain-to-field", gnu = "editfns.c:551", effect = Reads,
fn constrain_to_field(cx, new: Value, old: Value; optional escape: Value, only_in_line: Value, capture: Value) {
    cx.constrain_to_field(new, old, escape, only_in_line, capture)
}}

defsubr! { name = "field-beginning", gnu = "editfns.c:521", effect = Reads,
fn field_beginning(cx; optional pos: Value, escape: Value, limit: Value) {
    let (beg, _) = cx.find_field(pos, escape != Value::NIL, limit, Value::NIL, (true, false))?;
    Value::integer(beg).map_err(Fault::Error)
}}

defsubr! { name = "field-end", gnu = "editfns.c:536", effect = Reads,
fn field_end(cx; optional pos: Value, escape: Value, limit: Value) {
    let (_, end) = cx.find_field(pos, escape != Value::NIL, Value::NIL, limit, (false, true))?;
    Value::integer(end).map_err(Fault::Error)
}}

/// Call F, a buffer function of two positions, on POS's field.
fn over_field(cx: &mut Leaf<'_>, f: &str, pos: Value) -> Next {
    let (beg, end) = cx.find_field(pos, false, Value::NIL, Value::NIL, (true, true))?;
    let args = vec![Value::integer(beg).map_err(Fault::Error)?, Value::integer(end).map_err(Fault::Error)?];
    if f == "delete-region" && beg == end {
        return Ok(Step::Done(Value::NIL));
    }
    Ok(Step::Call { f: cx.intern(f), args, then: done, saved: vec![] })
}

fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

defsubr! { name = "field-string", gnu = "editfns.c:499", effect = Reads,
fn field_string(cx; optional pos: Value) -> Step { over_field(cx, "buffer-substring", pos) }}

defsubr! { name = "field-string-no-properties", gnu = "editfns.c:510", effect = Reads,
fn field_string_no_properties(cx; optional pos: Value) -> Step {
    over_field(cx, "buffer-substring-no-properties", pos)
}}

defsubr! { name = "delete-field", gnu = "editfns.c:486", effect = Global,
fn delete_field(cx; optional pos: Value) -> Step { over_field(cx, "delete-region", pos) }}

#[cfg(test)]
mod tests {
    #[test]
    fn native_column_preserves_point_and_handles_unicode_tabs() {
        let mut vm = crate::Vm::new();
        let got = vm.eval_source_raw(r#"(with-temp-buffer (insert "aα界\tZ") (list (current-column) (progn (goto-char 4) (current-column)) (progn (setq tab-width 4) (goto-char (point-max)) (current-column))))"#).unwrap();
        assert_eq!(vm.display(got).unwrap(), "(9 4 9)");
    }

    #[test]
    fn field_free_constraints_match_gnu_and_preserve_point() {
        let mut vm = crate::Vm::new();
        let source = "(with-temp-buffer (insert \"aα\\nbc\") (goto-char 4) (list (constrain-to-field 1 4) (constrain-to-field nil 1) (point) (condition-case e (constrain-to-field 999 4) (error e)) (condition-case e (constrain-to-field nil nil) (error e)) (let ((inhibit-field-text-motion t)) (constrain-to-field 0 4)) (constrain-to-field 999 999) (constrain-to-field (copy-marker 2) 4)))";
        let got = vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.display(got).unwrap(), "(1 4 4 (args-out-of-range 999) (wrong-type-argument integer-or-marker-p nil) 0 999 2)");
    }

    #[test]
    fn field_constraints_stop_at_field_boundaries_as_gnu_does() {
        // Pinned GNU's answers: motion out of either side stops at the edge.
        let mut vm = crate::Vm::new();
        let got = vm.eval_source_raw("(with-temp-buffer (insert \"abc\") (put-text-property 1 3 'field 'prompt) (list (constrain-to-field 3 1) (constrain-to-field 1 3) (constrain-to-field 4 2) (constrain-to-field 2 4) (constrain-to-field 1 4 t) (progn (goto-char 4) (list (constrain-to-field nil 2) (point)))))").unwrap();
        assert_eq!(vm.display(got).unwrap(), "(3 1 3 3 3 (3 3))");
    }
}
