//! Field-free motion; field-bearing buffers are explicitly refused.
use super::*;

defsubr! { name = "current-column", gnu = "indent.c:298", effect = Reads,
fn current_column(cx) { cx.native_current_column() }}

defsubr! { name = "constrain-to-field", gnu = "editfns.c:551", effect = Reads,
fn constrain_to_field(cx, new: Value, old: Value; optional _escape: Value, _line: Value, _capture: Value) {
    cx.fieldless_constraint(new, old)
}}

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
    fn field_constraints_refuse_instead_of_crossing_protected_fields() {
        let mut vm = crate::Vm::new();
        let got = vm.eval_source_raw("(with-temp-buffer (insert \"abc\") (put-text-property 1 3 'field 'prompt) (condition-case e (constrain-to-field 3 1) (error e)))").unwrap();
        assert_eq!(vm.display(got).unwrap(), "(error \"VM2 field-boundary motion is not implemented\")");
    }
}
