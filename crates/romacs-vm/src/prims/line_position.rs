//! Point-preserving logical line boundary queries.
use super::*;

defsubr! { name = "line-beginning-position", gnu = "editfns.c:774", effect = Reads,
fn line_beginning_position(cx; optional n: Value) {
    cx.line_field_position(n, false)
}}

defsubr! { name = "line-end-position", gnu = "editfns.c:823", effect = Reads,
fn line_end_position(cx; optional n: Value) {
    cx.line_field_position(n, true)
}}

defsubr! { name = "line-number-at-pos", gnu = "fns.c:6688", effect = Reads,
fn line_number_at_pos(cx; optional position: Value, absolute: Value) {
    cx.line_number_at_pos(position, absolute)
}}

// The field-free scans line-beginning-position and line-end-position
// constrain (editfns.c bol and eol).
defsubr! { name = "pos-bol", gnu = "editfns.c:684", effect = Reads,
fn pos_bol(cx; optional n: Value) {
    cx.line_boundary_position(n, false)
}}

defsubr! { name = "pos-eol", gnu = "editfns.c:740", effect = Reads,
fn pos_eol(cx; optional n: Value) {
    cx.line_boundary_position(n, true)
}}

#[cfg(test)]
mod tests {
    #[test]
    fn boundaries_preserve_point_and_restriction() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(with-temp-buffer (insert \"aα\nbb\ncc\") (goto-char 5) (list (line-beginning-position) (line-end-position) (line-beginning-position 0) (line-end-position 2) (line-end-position 999) (point) (progn (narrow-to-region 2 8) (list (line-beginning-position -999) (line-end-position 999) (point)))))").unwrap();
        assert_eq!(vm.display(value).unwrap(), "(4 6 1 9 9 5 (2 8 5))");
    }

    /// With no field to respect, a boundary costs what GNU's does: the
    /// line scanned in bytes and the answer converted once, the
    /// restriction's edges and point not at all (each converted position
    /// cost a chunk lookup that evicted point's).
    #[test]
    fn boundaries_without_fields_convert_only_the_answer() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(progn (set-buffer (get-buffer-create \"lines\")) (insert (make-string 3000 ?α) \"\\n\" (make-string 3000 ?β) \"\\n\") (goto-char 4500) (narrow-to-region 2 6000))").unwrap();
        let converted = |vm: &mut crate::Vm, form: &str| {
            let before = crate::text::char_conversions();
            let value = vm.eval_source_raw(form).unwrap();
            (vm.display(value).unwrap(), crate::text::char_conversions() - before)
        };
        let (_, idle) = converted(&mut vm, "(list nil nil)");
        let (value, used) = converted(&mut vm, "(list (line-beginning-position) (line-end-position))");
        assert_eq!(value, "(3002 6000)");
        assert_eq!(used - idle, 2, "one conversion a call");
    }
}
